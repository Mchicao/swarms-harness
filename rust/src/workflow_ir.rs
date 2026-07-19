//! Native compiler for the bounded, declarative workflow schema v2.

use serde_json::{json, Map, Value};
use std::collections::{HashMap, HashSet};

type Result<T> = std::result::Result<T, String>;

#[derive(Clone, Copy)]
struct Limits {
    max_total_workers: usize,
    max_depth: usize,
    max_children_per_agent: usize,
    max_rounds: usize,
}

impl Limits {
    fn from_plan(plan: &Value) -> Result<Self> {
        let budget = plan.get("budget_policy").unwrap_or(&Value::Null);
        let limit = |name, default| -> Result<usize> {
            match budget.get(name) {
                None => Ok(default),
                Some(value) => value
                    .as_u64()
                    .and_then(|n| usize::try_from(n).ok())
                    .ok_or_else(|| format!("budget_policy.{name} must be a non-negative integer")),
            }
        };
        let limits = Self {
            max_total_workers: limit("max_total_workers", 12)?,
            max_depth: limit("max_depth", 2)?,
            max_children_per_agent: limit("max_children_per_agent", 4)?,
            max_rounds: limit("max_rounds", 4)?,
        };
        if limits.max_total_workers == 0
            || limits.max_children_per_agent == 0
            || limits.max_rounds == 0
        {
            return Err("worker, child, and round limits must be positive".to_string());
        }
        let spawn_budget = limit("spawn_budget", 0)?;
        if spawn_budget != 0 {
            return Err(
                "spawn_budget must remain 0 until runtime-controlled insertion is available"
                    .to_string(),
            );
        }
        Ok(limits)
    }
}

/// Compile schema v2 to the ordinary schema v1 consumed by the Rust runtime.
/// Schema v1 input is returned unchanged.
pub fn compile_plan(mut plan: Value) -> Result<Value> {
    if plan.get("schema_version").and_then(Value::as_u64) != Some(2) {
        apply_default_tools_policy(&mut plan)?;
        return Ok(plan);
    }
    let limits = Limits::from_plan(&plan)?;
    if plan.get("workflow_compiled").and_then(Value::as_bool) == Some(true)
        && plan.get("workflow").is_none()
    {
        validate_flat_plan(&plan, limits)?;
        plan["schema_version"] = json!(1);
        return Ok(plan);
    }

    let steps = plan
        .get("workflow")
        .and_then(|v| v.get("steps"))
        .and_then(Value::as_array)
        .cloned()
        .ok_or_else(|| "schema_version 2 requires workflow.steps".to_string())?;
    let existing_count = plan
        .get("stages")
        .and_then(Value::as_array)
        .map(|stages| {
            stages
                .iter()
                .filter_map(|stage| stage.get("tasks").and_then(Value::as_array))
                .map(Vec::len)
                .sum()
        })
        .unwrap_or(0);
    let mut compiler = Compiler {
        limits,
        existing_count,
        generated: Vec::new(),
        outputs: HashMap::new(),
    };
    for step in steps {
        let id = required_id(&step)?;
        if compiler.outputs.contains_key(&id) {
            return Err(format!("duplicate workflow step id: {id}"));
        }
        let produced = compiler.expand(&step, None, &[])?;
        compiler.outputs.insert(id, produced);
    }

    let object = plan
        .as_object_mut()
        .ok_or_else(|| "plan must be an object".to_string())?;
    object.remove("workflow");
    object.insert("schema_version".to_string(), json!(1));
    object.insert("workflow_compiled".to_string(), json!(true));
    if !compiler.generated.is_empty() {
        let stages = object
            .entry("stages")
            .or_insert_with(|| Value::Array(Vec::new()))
            .as_array_mut()
            .ok_or_else(|| "stages must be an array".to_string())?;
        stages.push(json!({
            "name": "Dynamic Workflow",
            "parallel": true,
            "tasks": compiler.generated,
        }));
    }
    validate_flat_plan(&plan, limits)?;
    Ok(plan)
}

/// A plan may opt in to a single explicit default for artifact-producing
/// workers. Individual task values always win; absent defaults remain `none`.
fn apply_default_tools_policy(plan: &mut Value) -> Result<()> {
    let default = plan
        .get("default_tools_policy")
        .and_then(Value::as_str)
        .unwrap_or("none")
        .to_string();
    if !matches!(default.as_str(), "none" | "full") {
        return Err("default_tools_policy must be 'none' or 'full'".to_string());
    }
    let Some(stages) = plan.get_mut("stages").and_then(Value::as_array_mut) else {
        return Ok(());
    };
    for task in stages
        .iter_mut()
        .filter_map(|stage| stage.get_mut("tasks").and_then(Value::as_array_mut))
        .flatten()
        .filter_map(Value::as_object_mut)
    {
        task.entry("tools_policy".to_string())
            .or_insert_with(|| json!(default));
    }
    Ok(())
}

struct Compiler {
    limits: Limits,
    existing_count: usize,
    generated: Vec<Value>,
    outputs: HashMap<String, Vec<String>>,
}

impl Compiler {
    fn expand(
        &mut self,
        step: &Value,
        inherited_id: Option<&str>,
        inherited_needs: &[String],
    ) -> Result<Vec<String>> {
        let object = step
            .as_object()
            .ok_or_else(|| "every workflow step must be an object".to_string())?;
        let id = object
            .get("id")
            .and_then(Value::as_str)
            .or(inherited_id)
            .filter(|id| !id.trim().is_empty())
            .ok_or_else(|| "every workflow step requires id".to_string())?
            .to_string();
        let kind = object
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or("agent");
        let needs = self.resolve_refs(object.get("needs"), inherited_needs)?;
        match kind {
            "agent" | "verify" | "reduce" => {
                let mut normalized = object.clone();
                if kind == "verify" {
                    normalized.insert("role".to_string(), json!("verifier"));
                }
                let task_needs = if kind == "reduce" {
                    let mut all = self.resolve_refs(object.get("from"), &[])?;
                    extend_unique(&mut all, needs);
                    all
                } else {
                    needs
                };
                self.add_task(&normalized, &id, &HashMap::new(), task_needs)?;
                Ok(vec![id])
            }
            "map" => {
                let items = object
                    .get("items")
                    .and_then(Value::as_array)
                    .ok_or_else(|| format!("map step {id} requires literal items"))?;
                let item_name = object
                    .get("item_name")
                    .and_then(Value::as_str)
                    .unwrap_or("item");
                let mut ids = Vec::with_capacity(items.len());
                for (index, item) in items.iter().enumerate() {
                    let task_id = format!("{id}-{index:03}-{}", slug_value(item));
                    let variables = HashMap::from([
                        (item_name.to_string(), scalar_text(item)),
                        ("index".to_string(), index.to_string()),
                    ]);
                    self.add_task(object, &task_id, &variables, needs.clone())?;
                    ids.push(task_id);
                }
                Ok(ids)
            }
            "condition" => {
                let enabled = object
                    .get("when")
                    .and_then(Value::as_bool)
                    .ok_or_else(|| format!("condition step {id} requires a boolean when"))?;
                if !enabled {
                    return Ok(Vec::new());
                }
                let mut nested = object
                    .get("step")
                    .and_then(Value::as_object)
                    .cloned()
                    .ok_or_else(|| format!("condition step {id} requires step"))?;
                nested.entry("id").or_insert_with(|| json!(id));
                self.expand(&Value::Object(nested), Some(&id), &needs)
            }
            "loop" => {
                let rounds = match object.get("max_rounds") {
                    None => 1,
                    Some(value) => value
                        .as_u64()
                        .and_then(|n| usize::try_from(n).ok())
                        .ok_or_else(|| format!("loop step {id} max_rounds must be an integer"))?,
                };
                if rounds == 0 || rounds > self.limits.max_rounds {
                    return Err(format!(
                        "loop step {id} exceeds max_rounds={}",
                        self.limits.max_rounds
                    ));
                }
                let nested = object
                    .get("step")
                    .and_then(Value::as_object)
                    .cloned()
                    .ok_or_else(|| format!("loop step {id} requires step"))?;
                let mut produced = Vec::new();
                let mut round_needs = needs;
                for round in 1..=rounds {
                    let round_id = format!("{id}-round-{round:03}");
                    let mut normalized = nested.clone();
                    normalized.insert("id".to_string(), json!(round_id));
                    normalized.insert("needs".to_string(), json!(round_needs));
                    if let Some(task) = normalized.get("task").cloned() {
                        normalized.insert(
                            "task".to_string(),
                            format_value(
                                &task,
                                &HashMap::from([("round".to_string(), round.to_string())]),
                            )?,
                        );
                    }
                    let ids =
                        self.expand(&Value::Object(normalized), Some(&round_id), &round_needs)?;
                    extend_unique(&mut produced, ids.clone());
                    round_needs = ids;
                }
                Ok(produced)
            }
            other => Err(format!("unsupported workflow step type: {other}")),
        }
    }

    fn resolve_refs(&self, value: Option<&Value>, fallback: &[String]) -> Result<Vec<String>> {
        let Some(value) = value else {
            return Ok(fallback.to_vec());
        };
        let refs = value
            .as_array()
            .ok_or_else(|| "needs/from must be an array".to_string())?;
        let mut resolved = Vec::new();
        for reference in refs {
            let reference = reference
                .as_str()
                .ok_or_else(|| "needs/from entries must be strings".to_string())?;
            if let Some(outputs) = self.outputs.get(reference) {
                extend_unique(&mut resolved, outputs.clone());
            } else {
                extend_unique(&mut resolved, vec![reference.to_string()]);
            }
        }
        Ok(resolved)
    }

    fn add_task(
        &mut self,
        step: &Map<String, Value>,
        id: &str,
        variables: &HashMap<String, String>,
        needs: Vec<String>,
    ) -> Result<()> {
        if step
            .get("allow_subagent_spawning")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            return Err(format!(
                "task {id} cannot enable subagent spawning while spawn_budget is machine-locked to 0"
            ));
        }
        let mut task = Map::new();
        task.insert("id".to_string(), json!(id));
        task.insert(
            "role".to_string(),
            step.get("role")
                .cloned()
                .unwrap_or_else(|| json!("general")),
        );
        task.insert(
            "route".to_string(),
            step.get("route").cloned().unwrap_or_else(|| json!("mock")),
        );
        task.insert(
            "task".to_string(),
            format_value(
                step.get("task").unwrap_or(&Value::String(String::new())),
                variables,
            )?,
        );
        for field in ["artifacts", "verify"] {
            task.insert(
                field.to_string(),
                format_value(
                    step.get(field).unwrap_or(&Value::Array(Vec::new())),
                    variables,
                )?,
            );
        }
        task.insert("needs".to_string(), json!(needs));
        task.insert(
            "tools_policy".to_string(),
            step.get("tools_policy")
                .cloned()
                .unwrap_or_else(|| json!("none")),
        );
        task.insert("allow_subagent_spawning".to_string(), json!(false));
        if let Some(parent) = step
            .get("parent")
            .or_else(|| step.get("parent_task_id"))
            .and_then(Value::as_str)
        {
            let parents = self
                .outputs
                .get(parent)
                .cloned()
                .unwrap_or_else(|| vec![parent.to_string()]);
            if parents.len() != 1 {
                return Err(format!(
                    "parent step {parent} must produce exactly one task"
                ));
            }
            task.insert("parent_task_id".to_string(), json!(parents[0]));
        }
        self.generated.push(Value::Object(task));
        if self.existing_count + self.generated.len() > self.limits.max_total_workers {
            return Err(format!(
                "expanded workflow exceeds max_total_workers={}",
                self.limits.max_total_workers
            ));
        }
        Ok(())
    }
}

fn validate_flat_plan(plan: &Value, limits: Limits) -> Result<()> {
    let tasks: Vec<&Map<String, Value>> = plan
        .get("stages")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|stage| stage.get("tasks").and_then(Value::as_array))
        .flatten()
        .filter_map(Value::as_object)
        .collect();
    if tasks.len() > limits.max_total_workers {
        return Err(format!(
            "plan exceeds max_total_workers={}",
            limits.max_total_workers
        ));
    }
    let mut by_id = HashMap::new();
    for task in &tasks {
        let id = task
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| "every task requires id".to_string())?;
        if by_id.insert(id, *task).is_some() {
            return Err(format!("duplicate task id: {id}"));
        }
        if task.get("allow_subagent_spawning").and_then(Value::as_bool) == Some(true) {
            return Err("allow_subagent_spawning is machine-locked to false".to_string());
        }
    }
    let mut children = HashMap::<&str, usize>::new();
    for task in &tasks {
        if let Some(parent) = task
            .get("parent_task_id")
            .or_else(|| task.get("parent_id"))
            .and_then(Value::as_str)
        {
            if !by_id.contains_key(parent) {
                return Err(format!("invalid parent chain for {}", task["id"]));
            }
            let count = children.entry(parent).or_default();
            *count += 1;
            if *count > limits.max_children_per_agent {
                return Err(format!(
                    "parent {parent} exceeds max_children_per_agent={}",
                    limits.max_children_per_agent
                ));
            }
        }
        for dependency in string_array(task.get("needs"))? {
            if !by_id.contains_key(dependency) {
                return Err(format!(
                    "task {} has missing dependency {dependency}",
                    task["id"]
                ));
            }
        }
    }
    fn parent_depth<'a>(
        id: &'a str,
        by_id: &HashMap<&'a str, &'a Map<String, Value>>,
        trail: &mut HashSet<&'a str>,
    ) -> Result<usize> {
        if !trail.insert(id) {
            return Err(format!("parent cycle includes {id}"));
        }
        let depth = match by_id[id]
            .get("parent_task_id")
            .or_else(|| by_id[id].get("parent_id"))
            .and_then(Value::as_str)
        {
            Some(parent) => 1 + parent_depth(parent, by_id, trail)?,
            None => 0,
        };
        trail.remove(id);
        Ok(depth)
    }
    fn visit_needs<'a>(
        id: &'a str,
        by_id: &HashMap<&'a str, &'a Map<String, Value>>,
        active: &mut HashSet<&'a str>,
        done: &mut HashSet<&'a str>,
    ) -> Result<()> {
        if done.contains(id) {
            return Ok(());
        }
        if !active.insert(id) {
            return Err(format!("dependency cycle includes {id}"));
        }
        for dependency in string_array(by_id[id].get("needs"))? {
            visit_needs(dependency, by_id, active, done)?;
        }
        active.remove(id);
        done.insert(id);
        Ok(())
    }
    let mut done = HashSet::new();
    for id in by_id.keys().copied() {
        let depth = parent_depth(id, &by_id, &mut HashSet::new())?;
        let declared_depth = match by_id[id].get("depth") {
            None => 0,
            Some(value) => value
                .as_u64()
                .and_then(|n| usize::try_from(n).ok())
                .ok_or_else(|| format!("task {id} depth must be an integer"))?,
        };
        if depth > limits.max_depth || declared_depth > limits.max_depth {
            return Err(format!("task {id} exceeds max_depth={}", limits.max_depth));
        }
        visit_needs(id, &by_id, &mut HashSet::new(), &mut done)?;
    }
    Ok(())
}

fn string_array(value: Option<&Value>) -> Result<Vec<&str>> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    value
        .as_array()
        .ok_or_else(|| "needs must be an array".to_string())?
        .iter()
        .map(|value| {
            value
                .as_str()
                .ok_or_else(|| "needs entries must be strings".to_string())
        })
        .collect()
}

fn required_id(step: &Value) -> Result<String> {
    step.as_object()
        .and_then(|step| step.get("id"))
        .and_then(Value::as_str)
        .filter(|id| !id.trim().is_empty())
        .map(str::to_string)
        .ok_or_else(|| "every workflow step requires id".to_string())
}

fn format_value(value: &Value, variables: &HashMap<String, String>) -> Result<Value> {
    match value {
        Value::String(text) => {
            let mut formatted = text.clone();
            for (name, replacement) in variables {
                formatted = formatted.replace(&format!("{{{name}}}"), replacement);
            }
            if let Some(start) = formatted.find('{') {
                if let Some(end) = formatted[start + 1..].find('}') {
                    return Err(format!(
                        "unknown workflow template variable: {}",
                        &formatted[start + 1..start + 1 + end]
                    ));
                }
            }
            Ok(Value::String(formatted))
        }
        Value::Array(values) => values
            .iter()
            .map(|value| format_value(value, variables))
            .collect::<Result<Vec<_>>>()
            .map(Value::Array),
        other => Ok(other.clone()),
    }
}

fn extend_unique(target: &mut Vec<String>, values: Vec<String>) {
    for value in values {
        if !target.contains(&value) {
            target.push(value);
        }
    }
}

fn scalar_text(value: &Value) -> String {
    value
        .as_str()
        .map(str::to_string)
        .unwrap_or_else(|| value.to_string())
}

fn slug_value(value: &Value) -> String {
    let clean: String = scalar_text(value)
        .trim()
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-') {
                ch
            } else {
                '-'
            }
        })
        .collect();
    let clean = clean.trim_matches('-');
    if clean.is_empty() {
        "item".to_string()
    } else {
        clean.chars().take(48).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dynamic_plan() -> Value {
        json!({
            "schema_version": 2,
            "goal": "Audit and summarize files",
            "budget_policy": {"max_total_workers": 12, "max_depth": 2, "max_children_per_agent": 4, "max_rounds": 3, "spawn_budget": 0},
            "workflow": {"steps": [
                {"id": "discover", "type": "agent", "route": "mock", "task": "Discover"},
                {"id": "audit", "type": "map", "items": ["a.py", "b.py"], "item_name": "file", "route": "mock", "task": "Audit {file}", "needs": ["discover"], "parent": "discover"},
                {"id": "merge", "type": "reduce", "from": ["audit"], "route": "mock", "task": "Merge"},
                {"id": "check", "type": "verify", "route": "mock", "task": "Verify", "needs": ["merge"]},
                {"id": "optional", "type": "condition", "when": false, "step": {"type": "agent", "task": "Skip"}},
                {"id": "polish", "type": "loop", "max_rounds": 2, "needs": ["check"], "step": {"type": "agent", "route": "mock", "task": "Polish round {round}"}}
            ]}
        })
    }

    #[test]
    fn expands_all_steps_deterministically() {
        let first = compile_plan(dynamic_plan()).unwrap();
        let second = compile_plan(dynamic_plan()).unwrap();
        assert_eq!(first, second);
        assert_eq!(first["schema_version"], 1);
        let tasks = first["stages"][0]["tasks"].as_array().unwrap();
        let ids: Vec<_> = tasks
            .iter()
            .map(|task| task["id"].as_str().unwrap())
            .collect();
        assert_eq!(
            ids,
            [
                "discover",
                "audit-000-a.py",
                "audit-001-b.py",
                "merge",
                "check",
                "polish-round-001",
                "polish-round-002"
            ]
        );
        assert_eq!(
            tasks[3]["needs"],
            json!(["audit-000-a.py", "audit-001-b.py"])
        );
        assert_eq!(tasks[4]["role"], "verifier");
        assert_eq!(tasks[6]["needs"], json!(["polish-round-001"]));
    }

    #[test]
    fn applies_explicit_default_tools_policy_to_flat_tasks() {
        let plan = compile_plan(json!({
            "schema_version": 1,
            "goal": "Write an artifact",
            "default_tools_policy": "full",
            "stages": [{"tasks": [{"id": "write", "route": "mock", "task": "write"}]}]
        }))
        .unwrap();
        assert_eq!(plan["stages"][0]["tasks"][0]["tools_policy"], "full");
    }

    #[test]
    fn enforces_all_hard_limits_and_spawn_lock() {
        for (field, value) in [
            ("max_total_workers", 3),
            ("max_children_per_agent", 1),
            ("max_depth", 0),
            ("max_rounds", 1),
        ] {
            let mut plan = dynamic_plan();
            plan["budget_policy"][field] = json!(value);
            assert!(compile_plan(plan).unwrap_err().contains(field));
        }
        let mut plan = dynamic_plan();
        plan["budget_policy"]["spawn_budget"] = json!(1);
        assert!(compile_plan(plan).unwrap_err().contains("spawn_budget"));
    }

    #[test]
    fn rejects_cycles_and_recursive_worker_override() {
        let mut plan = dynamic_plan();
        plan["workflow"]["steps"][0]["allow_subagent_spawning"] = json!(true);
        assert!(compile_plan(plan).unwrap_err().contains("machine-locked"));
        let cyclic = json!({
            "schema_version": 2,
            "workflow_compiled": true,
            "budget_policy": {"spawn_budget": 0},
            "stages": [{"tasks": [
                {"id": "a", "route": "mock", "task": "a", "needs": ["b"]},
                {"id": "b", "route": "mock", "task": "b", "needs": ["a"]}
            ]}]
        });
        assert!(compile_plan(cyclic)
            .unwrap_err()
            .contains("dependency cycle"));
    }
}
