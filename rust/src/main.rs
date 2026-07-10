use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::env;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

type Result<T> = std::result::Result<T, String>;

#[derive(Clone, Debug, Deserialize)]
struct Provider {
    #[serde(default)]
    enabled: bool,
    provider: String,
    model: String,
    wrapper: String,
}

#[derive(Deserialize)]
struct Router {
    providers: HashMap<String, Provider>,
}

#[derive(Deserialize)]
struct Plan {
    #[serde(default)]
    budget_policy: BudgetPolicy,
    stages: Vec<Stage>,
}

#[derive(Default, Deserialize)]
struct BudgetPolicy {
    #[serde(default)]
    global_max_concurrency: usize,
    #[serde(default)]
    provider_concurrency: HashMap<String, usize>,
}

#[derive(Deserialize)]
struct Stage {
    #[serde(default = "default_stage_name")]
    name: String,
    tasks: Vec<TaskSpec>,
}

fn default_stage_name() -> String {
    "Unnamed".to_string()
}

#[derive(Clone, Deserialize)]
struct TaskSpec {
    id: String,
    route: String,
    task: String,
    #[serde(default = "default_role")]
    role: String,
    #[serde(default)]
    needs: Vec<String>,
    #[serde(default = "default_tools_policy")]
    tools_policy: String,
}

fn default_role() -> String {
    "general".to_string()
}

fn default_tools_policy() -> String {
    "none".to_string()
}

#[derive(Clone)]
struct Task {
    id: String,
    source_id: String,
    stage: String,
    spec: TaskSpec,
    provider: Provider,
}

#[derive(Serialize)]
struct TaskResult {
    task_id: String,
    source_id: String,
    route: String,
    provider: String,
    model: String,
    status: String,
    duration_ms: u128,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

struct Args {
    command: String,
    plan: PathBuf,
    run_id: String,
    force: bool,
    global_cap: Option<usize>,
    caps: HashMap<String, usize>,
}

fn main() {
    if let Err(error) = run() {
        eprintln!("[swarms-rs] ERROR: {error}");
        std::process::exit(2);
    }
}

fn run() -> Result<()> {
    let args = parse_args()?;
    let root = env::current_dir().map_err(|e| e.to_string())?;
    let router = load_router(&root)?;
    if args.command == "doctor" {
        println!("[OK] Rust coordinator available on {}", env::consts::OS);
        println!(
            "[OK] default router loaded ({} providers)",
            router.providers.len()
        );
        return Ok(());
    }
    let plan = load_plan(&args.plan)?;
    let tasks = build_tasks(&plan, &router)?;
    if args.command == "review" {
        println!(
            "{}",
            json!({"ok": true, "errors": 0, "task_count": tasks.len()})
        );
        return Ok(());
    }
    let global_cap = args
        .global_cap
        .unwrap_or(plan.budget_policy.global_max_concurrency.max(1));
    let caps = effective_caps(&plan, &args.caps);
    if args.command == "dry-run" {
        println!(
            "{}",
            json!({"status": "planned", "task_count": tasks.len(), "global_max_concurrency": global_cap, "provider_max_concurrency": caps})
        );
        return Ok(());
    }
    if args.command != "run" {
        return Err(format!("unsupported command: {}", args.command));
    }
    let report = execute(&root, tasks, global_cap, caps, &args.run_id, args.force)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&report).map_err(|e| e.to_string())?
    );
    if report["status"] != "completed" {
        return Err("one or more workers failed".to_string());
    }
    Ok(())
}

fn parse_args() -> Result<Args> {
    let mut values = env::args().skip(1);
    let command = values
        .next()
        .ok_or("usage: swarms-rs <doctor|review|dry-run|run> --plan <file>")?;
    if command == "doctor" {
        return Ok(Args {
            command,
            plan: PathBuf::new(),
            run_id: make_run_id(),
            force: false,
            global_cap: None,
            caps: HashMap::new(),
        });
    }
    let mut plan = None;
    let mut run_id = make_run_id();
    let mut force = false;
    let mut global_cap = None;
    let mut caps = HashMap::new();
    while let Some(value) = values.next() {
        match value.as_str() {
            "--plan" => plan = Some(PathBuf::from(values.next().ok_or("--plan needs a file")?)),
            "--run-id" => run_id = values.next().ok_or("--run-id needs a value")?,
            "--force" => force = true,
            "--global-max-concurrency" => {
                global_cap = Some(parse_positive(&values.next().ok_or("missing global cap")?)?)
            }
            "--provider-cap" => {
                let pair = values.next().ok_or("--provider-cap needs route=count")?;
                let (route, count) = pair
                    .split_once('=')
                    .ok_or("provider cap must be route=count")?;
                caps.insert(route.to_string(), parse_positive(count)?);
            }
            other => return Err(format!("unknown argument: {other}")),
        }
    }
    if !safe_run_id(&run_id) {
        return Err(
            "run id must contain only letters, numbers, dot, underscore, or dash".to_string(),
        );
    }
    Ok(Args {
        command,
        plan: plan.ok_or("--plan is required")?,
        run_id,
        force,
        global_cap,
        caps,
    })
}

fn parse_positive(value: &str) -> Result<usize> {
    value
        .parse::<usize>()
        .ok()
        .filter(|value| *value > 0)
        .ok_or_else(|| "capacity must be positive".to_string())
}

fn make_run_id() -> String {
    format!(
        "rs-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
    )
}

fn safe_run_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-'))
}

fn load_json(path: &Path) -> Result<Value> {
    serde_json::from_str(&fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?)
        .map_err(|e| e.to_string())
}

fn merge(base: &mut Value, local: Value) {
    match (base, local) {
        (Value::Object(base_map), Value::Object(local_map)) => {
            for (key, value) in local_map {
                if let Some(existing) = base_map.get_mut(&key) {
                    merge(existing, value);
                } else {
                    base_map.insert(key, value);
                }
            }
        }
        (base, local) => *base = local,
    }
}

fn load_router(root: &Path) -> Result<Router> {
    let mut value = load_json(&root.join("config/swarm_router.json"))?;
    let local = root.join("config/swarm_router.local.json");
    if local.exists() {
        merge(&mut value, load_json(&local)?);
    }
    serde_json::from_value(value).map_err(|e| e.to_string())
}

fn load_plan(path: &Path) -> Result<Plan> {
    serde_json::from_value(load_json(path)?).map_err(|e| format!("{}: {e}", path.display()))
}

fn build_tasks(plan: &Plan, router: &Router) -> Result<Vec<Task>> {
    let mut tasks = Vec::new();
    for stage in &plan.stages {
        for spec in &stage.tasks {
            let provider = router
                .providers
                .get(&spec.route)
                .ok_or_else(|| format!("unknown route: {}", spec.route))?
                .clone();
            if !provider.enabled {
                return Err(format!("route is disabled: {}", spec.route));
            }
            if provider.model.is_empty() || provider.wrapper.is_empty() {
                return Err(format!("route has no model or wrapper: {}", spec.route));
            }
            let id = format!("{:04}-{}", tasks.len(), slug(&spec.id));
            tasks.push(Task {
                id,
                source_id: spec.id.clone(),
                stage: stage.name.clone(),
                spec: spec.clone(),
                provider,
            });
        }
    }
    if tasks.is_empty() {
        return Err("plan has no tasks".to_string());
    }
    Ok(tasks)
}

fn slug(value: &str) -> String {
    let out: String = value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-') {
                ch
            } else {
                '-'
            }
        })
        .collect();
    let clean: String = out.trim_matches('-').chars().take(80).collect();
    if clean.is_empty() {
        "task".to_string()
    } else {
        clean
    }
}

fn effective_caps(plan: &Plan, overrides: &HashMap<String, usize>) -> HashMap<String, usize> {
    let mut caps = plan.budget_policy.provider_concurrency.clone();
    caps.extend(overrides.clone());
    caps
}

fn execute(
    root: &Path,
    tasks: Vec<Task>,
    global_cap: usize,
    caps: HashMap<String, usize>,
    run_id: &str,
    force: bool,
) -> Result<Value> {
    let run_dir = root.join(".agent/swarm/runs").join(run_id);
    if run_dir.exists() {
        if !force {
            return Err(format!("run already exists: {}", run_id));
        }
        fs::remove_dir_all(&run_dir).map_err(|e| e.to_string())?;
    }
    fs::create_dir_all(&run_dir).map_err(|e| e.to_string())?;
    let mut pending = tasks;
    let mut complete = HashSet::new();
    let mut results = Vec::new();
    while !pending.is_empty() {
        let mut active_by_route: HashMap<String, usize> = HashMap::new();
        let mut selected = Vec::new();
        let mut next = Vec::new();
        for task in pending {
            let dependency_ready = task.spec.needs.iter().all(|need| complete.contains(need));
            let cap = *caps.get(&task.spec.route).unwrap_or(&1);
            let used = *active_by_route.get(&task.spec.route).unwrap_or(&0);
            if dependency_ready && selected.len() < global_cap && used < cap {
                *active_by_route.entry(task.spec.route.clone()).or_default() += 1;
                selected.push(task);
            } else {
                next.push(task);
            }
        }
        if selected.is_empty() {
            return Err("no runnable tasks; check dependencies and provider caps".to_string());
        }
        let (sender, receiver) = mpsc::channel();
        for task in selected {
            let sender = sender.clone();
            let root = root.to_path_buf();
            let run_dir = run_dir.clone();
            thread::spawn(move || {
                let _ = sender.send(run_worker(&root, &run_dir, task));
            });
        }
        drop(sender);
        for result in receiver {
            if result.status == "completed" {
                complete.insert(result.source_id.clone());
            }
            results.push(result);
        }
        pending = next;
    }
    let success = results.iter().all(|result| result.status == "completed");
    let report = json!({"run_id": run_id, "status": if success { "completed" } else { "failed" }, "task_counts": {"completed": results.iter().filter(|result| result.status == "completed").count(), "failed": results.iter().filter(|result| result.status != "completed").count()}, "results": results});
    write_json(&run_dir.join("report-rs.json"), &report)?;
    Ok(report)
}

fn run_worker(root: &Path, run_dir: &Path, task: Task) -> TaskResult {
    let started = SystemTime::now();
    let work_dir = run_dir.join("results").join(&task.id);
    let result = (|| -> Result<()> {
        fs::create_dir_all(&work_dir).map_err(|e| e.to_string())?;
        let prompt = work_dir.join("prompt.txt");
        fs::write(
            &prompt,
            format!(
                "[{}:{}] {}\nReturn only the required result and keep output concise.\n",
                task.stage, task.spec.role, task.spec.task
            ),
        )
        .map_err(|e| e.to_string())?;
        let status = work_dir.join("status.json");
        let script = match task.provider.wrapper.as_str() {
            "mock" => "mock_worker",
            "gemini" => "gemini_worker",
            "opencode" => "opencode_worker",
            "kilo" => "kilo_worker",
            "codex" => "codex_worker",
            "openai_compat" => "openai_compat_worker",
            "hermes" => "hermes_worker",
            other => return Err(format!("unsupported wrapper: {other}")),
        };
        let python = env::var("SWARMS_PYTHON").unwrap_or_else(|_| {
            if cfg!(windows) {
                "python".to_string()
            } else {
                "python3".to_string()
            }
        });
        let mut command = Command::new(python);
        command
            .current_dir(root)
            .arg("-m")
            .arg(format!("scripts.{script}"))
            .arg("--prompt")
            .arg(&prompt);
        if script != "mock_worker" {
            command
                .arg("--status")
                .arg(&status)
                .arg("--model")
                .arg(&task.provider.model)
                .arg("--tools-policy")
                .arg(&task.spec.tools_policy);
        }
        if task.provider.wrapper == "openai_compat" {
            let key = match task.provider.provider.as_str() {
                "openrouter" => "OPENROUTER_API_KEY",
                "gitlawb" => "GITLAWB_API_KEY",
                "novita" => "NOVITA_API_KEY",
                "siliconflow" => "SILICONFLOW_API_KEY",
                _ => "OPENAI_COMPAT_API_KEY",
            };
            command
                .arg("--key-env")
                .arg(key)
                .arg("--base-url-env")
                .arg(format!(
                    "{}_BASE_URL",
                    task.provider.provider.to_uppercase()
                ));
            if task.provider.provider == "gitlawb" {
                command
                    .arg("--base-url")
                    .arg("https://opengateway.gitlawb.com/v1");
            }
        }
        if task.provider.wrapper == "hermes" && task.provider.model.starts_with("tencent/hy3") {
            command.arg("--provider").arg("nous");
        }
        let output = command
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .map_err(|e| e.to_string())?;
        let mut log = fs::File::create(work_dir.join("worker.log")).map_err(|e| e.to_string())?;
        log.write_all(&output.stdout).map_err(|e| e.to_string())?;
        log.write_all(&output.stderr).map_err(|e| e.to_string())?;
        if !output.status.success() {
            return Err(format!("worker exited {:?}", output.status.code()));
        }
        Ok(())
    })();
    let duration_ms = started.elapsed().unwrap_or(Duration::ZERO).as_millis();
    let result = TaskResult {
        task_id: task.id,
        source_id: task.source_id,
        route: task.spec.route,
        provider: task.provider.provider,
        model: task.provider.model,
        status: if result.is_ok() {
            "completed".to_string()
        } else {
            "failed".to_string()
        },
        duration_ms,
        error: result.err(),
    };
    let _ = write_json(
        &work_dir.join("result-rs.json"),
        &serde_json::to_value(&result).unwrap_or(Value::Null),
    );
    result
}

fn write_json(path: &Path, value: &Value) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let tmp = path.with_extension("tmp");
    fs::write(
        &tmp,
        serde_json::to_string_pretty(value).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    fs::rename(&tmp, path).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn run_ids_are_portable() {
        assert!(safe_run_id("windows-linux_macos.1"));
        assert!(!safe_run_id("../escape"));
    }
    #[test]
    fn slug_is_safe() {
        assert_eq!(slug("hello / world"), "hello---world");
    }
}
