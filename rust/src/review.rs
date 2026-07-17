//! Static plan validation: schema, DAG, routes, thinking, session compatibility.

use crate::adapter::AdapterKind;
use crate::model::{find_dependency_task, Plan, Router, Task};
use serde::Serialize;
use std::collections::HashSet;

// ---------------------------------------------------------------------------
// Review result types
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Error,
    Warning,
}

#[derive(Clone, Debug, Serialize)]
pub struct Finding {
    pub severity: Severity,
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct ReviewResult {
    pub ok: bool,
    pub errors: usize,
    pub warnings: usize,
    pub task_count: usize,
    pub routes: Vec<String>,
    pub findings: Vec<Finding>,
}

const VALID_ROLES: &[&str] = &[
    "planner",
    "critic",
    "programmer",
    "verifier",
    "docs",
    "backend",
    "qa",
    "debug",
    "general",
];

const PREMIUM_SUBSTRINGS: &[&str] = &["codex", "claude", "opus", "gpt55", "gpt-5.5"];

// ---------------------------------------------------------------------------
// Top-level review entry point
// ---------------------------------------------------------------------------

pub fn review_plan(plan: &Plan, router: &Router, tasks: &[Task]) -> ReviewResult {
    let mut findings = Vec::new();
    let task_count = tasks.len();
    let routes: Vec<String> = tasks
        .iter()
        .map(|t| t.spec.route.clone())
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();

    // --- Schema version ---
    if let Some(v) = plan.schema_version {
        if v != 1 {
            findings.push(Finding {
                severity: Severity::Error,
                code: "schema_version".to_string(),
                message: format!("schema_version must be 1, got {v}"),
                task_id: None,
            });
        }
    }

    // --- Goal ---
    if plan.goal.as_deref().unwrap_or("").is_empty() {
        findings.push(Finding {
            severity: Severity::Error,
            code: "missing_goal".to_string(),
            message: "plan must have a goal".to_string(),
            task_id: None,
        });
    }

    if let Some(project) = &plan.project {
        let valid_id = !project.id.is_empty()
            && project.id.len() <= 80
            && project
                .id
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-'));
        if !valid_id {
            findings.push(Finding {
                severity: Severity::Error,
                code: "invalid_project_id".to_string(),
                message: "project.id must use 1-80 letters, numbers, dot, underscore, or dash"
                    .to_string(),
                task_id: None,
            });
        }
        if project
            .name
            .as_deref()
            .is_some_and(|name| name.trim().is_empty() || name.chars().count() > 80)
        {
            findings.push(Finding {
                severity: Severity::Error,
                code: "invalid_project_name".to_string(),
                message: "project.name must use 1-80 characters".to_string(),
                task_id: None,
            });
        }
    }

    // --- Budget ---
    if plan.budget_policy.max_total_workers < task_count {
        findings.push(Finding {
            severity: Severity::Error,
            code: "worker_budget".to_string(),
            message: format!(
                "max_total_workers ({}) < task_count ({})",
                plan.budget_policy.max_total_workers, task_count
            ),
            task_id: None,
        });
    }

    // --- Unique task IDs ---
    let mut seen_ids = HashSet::new();
    for task in tasks {
        if !seen_ids.insert(&task.source_id) {
            findings.push(Finding {
                severity: Severity::Error,
                code: "duplicate_task_id".to_string(),
                message: format!("duplicate task id '{}'", task.source_id),
                task_id: Some(task.source_id.clone()),
            });
        }
        if task.spec.task.is_empty() {
            findings.push(Finding {
                severity: Severity::Error,
                code: "missing_task_text".to_string(),
                message: "task must have non-empty 'task' text".to_string(),
                task_id: Some(task.source_id.clone()),
            });
        }
    }

    // --- Per-task validation ---
    let premium_allowed = plan
        .review_policy
        .as_ref()
        .and_then(|rp| rp.premium_allowed)
        .unwrap_or(false);

    for task in tasks {
        let route = &task.spec.route;

        // Route enabled
        let provider = match router.get_provider(route) {
            Some(p) if p.enabled => p,
            Some(_) => {
                findings.push(Finding {
                    severity: Severity::Error,
                    code: "route_disabled".to_string(),
                    message: format!("route '{}' is disabled", route),
                    task_id: Some(task.source_id.clone()),
                });
                continue;
            }
            None => {
                findings.push(Finding {
                    severity: Severity::Error,
                    code: "invalid_route".to_string(),
                    message: format!("unknown route '{}'", route),
                    task_id: Some(task.source_id.clone()),
                });
                continue;
            }
        };

        // Wrapper supported
        let kind = match AdapterKind::from_wrapper(&provider.wrapper) {
            Some(k) => k,
            None => {
                findings.push(Finding {
                    severity: Severity::Error,
                    code: "unsupported_wrapper".to_string(),
                    message: format!(
                        "route '{}' uses unsupported wrapper '{}'",
                        route, provider.wrapper
                    ),
                    task_id: Some(task.source_id.clone()),
                });
                continue;
            }
        };

        // Role
        if !VALID_ROLES.contains(&task.spec.role.as_str()) {
            findings.push(Finding {
                severity: Severity::Error,
                code: "invalid_role".to_string(),
                message: format!("invalid role '{}'", task.spec.role),
                task_id: Some(task.source_id.clone()),
            });
        }

        // Tools policy
        if !matches!(task.spec.tools_policy.as_str(), "none" | "full") {
            findings.push(Finding {
                severity: Severity::Error,
                code: "invalid_tools_policy".to_string(),
                message: format!("invalid tools_policy '{}'", task.spec.tools_policy),
                task_id: Some(task.source_id.clone()),
            });
        }

        // Premium routes
        let route_lower = route.to_lowercase();
        if !premium_allowed && PREMIUM_SUBSTRINGS.iter().any(|s| route_lower.contains(s)) {
            findings.push(Finding {
                severity: Severity::Error,
                code: "premium_route_blocked".to_string(),
                message: format!(
                    "route '{}' is premium but review_policy.premium_allowed is not true",
                    route
                ),
                task_id: Some(task.source_id.clone()),
            });
        }

        // Artifacts safe
        for art in &task.spec.artifacts {
            if let Err(msg) = validate_artifact_path(art) {
                findings.push(Finding {
                    severity: Severity::Error,
                    code: "unsafe_artifact_path".to_string(),
                    message: msg,
                    task_id: Some(task.source_id.clone()),
                });
            }
        }

        // Thinking compatibility (mock silently accepts any level)
        let thinking = task.spec.effective_thinking(plan);
        if !thinking.is_default() && !kind.supports_thinking() && kind != AdapterKind::Mock {
            if kind == AdapterKind::OpenAiCompat && provider.thinking_field.is_none() {
                findings.push(Finding {
                    severity: Severity::Error,
                    code: "thinking_not_supported".to_string(),
                    message: format!(
                        "task '{}' requests thinking={:?} but route '{}' (openai_compat) has no thinking_field configured",
                        task.source_id, thinking, route
                    ),
                    task_id: Some(task.source_id.clone()),
                });
            } else if kind != AdapterKind::OpenAiCompat {
                findings.push(Finding {
                    severity: Severity::Error,
                    code: "thinking_not_supported".to_string(),
                    message: format!(
                        "task '{}' requests thinking={:?} but adapter '{}' does not support a thinking flag",
                        task.source_id, thinking, provider.wrapper
                    ),
                    task_id: Some(task.source_id.clone()),
                });
            }
        }

        // Session compatibility (mock silently accepts sessions for testing)
        let session = task.spec.effective_session(plan);
        match session.mode {
            crate::model::SessionMode::Reuse
                if !kind.supports_session_reuse() && kind != AdapterKind::Mock =>
            {
                findings.push(Finding {
                    severity: Severity::Error,
                    code: "session_reuse_not_supported".to_string(),
                    message: format!(
                        "task '{}' requests session reuse but adapter '{}' cannot safely capture/resume session IDs",
                        task.source_id, provider.wrapper
                    ),
                    task_id: Some(task.source_id.clone()),
                });
            }
            crate::model::SessionMode::New | crate::model::SessionMode::Reuse
                if session.key.as_deref().unwrap_or("").is_empty() =>
            {
                findings.push(Finding {
                    severity: Severity::Error,
                    code: "session_missing_key".to_string(),
                    message: format!(
                        "task '{}' requests session mode {:?} without a key",
                        task.source_id, session.mode
                    ),
                    task_id: Some(task.source_id.clone()),
                });
            }
            _ => {}
        }

        // Warnings for verifier tasks
        if task.spec.role == "verifier" && task.spec.verify.is_empty() {
            findings.push(Finding {
                severity: Severity::Warning,
                code: "missing_verification".to_string(),
                message: "verifier task should include a deterministic verify command".to_string(),
                task_id: Some(task.source_id.clone()),
            });
        }
        if matches!(
            task.spec.role.as_str(),
            "programmer" | "verifier" | "backend" | "qa"
        ) && task.spec.artifacts.is_empty()
        {
            findings.push(Finding {
                severity: Severity::Warning,
                code: "missing_artifacts".to_string(),
                message: format!("{} task should declare artifacts", task.spec.role),
                task_id: Some(task.source_id.clone()),
            });
        }
    }

    // --- Dependencies: existence ---
    for task in tasks {
        for dep in &task.spec.needs {
            if find_dependency_task(tasks, dep).is_none() {
                findings.push(Finding {
                    severity: Severity::Error,
                    code: "missing_dependency".to_string(),
                    message: format!(
                        "task '{}' needs '{}' but no such task id exists",
                        task.source_id, dep
                    ),
                    task_id: Some(task.source_id.clone()),
                });
            }
        }
    }

    // --- Dependencies: cycle detection (DFS) ---
    let cycles = detect_cycles(tasks);
    for cycle in &cycles {
        findings.push(Finding {
            severity: Severity::Error,
            code: "cyclic_dependency".to_string(),
            message: format!("cyclic dependency: {}", cycle.join(" -> ")),
            task_id: None,
        });
    }

    // --- Provider concurrency > 0 for used routes ---
    for route in &routes {
        let cap = plan
            .budget_policy
            .provider_concurrency
            .get(route)
            .copied()
            .unwrap_or(1);
        if cap == 0 {
            findings.push(Finding {
                severity: Severity::Error,
                code: "provider_capacity".to_string(),
                message: format!(
                    "provider_concurrency for route '{}' is 0 but tasks use it",
                    route
                ),
                task_id: None,
            });
        }
    }

    let errors = findings
        .iter()
        .filter(|f| f.severity == Severity::Error)
        .count();
    let warnings = findings
        .iter()
        .filter(|f| f.severity == Severity::Warning)
        .count();

    ReviewResult {
        ok: errors == 0,
        errors,
        warnings,
        task_count,
        routes,
        findings,
    }
}

// ---------------------------------------------------------------------------
// Artifact path validation
// ---------------------------------------------------------------------------

pub fn validate_artifact_path(path: &str) -> Result<(), String> {
    if path.is_empty() {
        return Err("artifact path is empty".to_string());
    }
    if path.starts_with('/') || path.starts_with('\\') {
        return Err(format!(
            "artifact must be repo-relative, not absolute: '{path}'"
        ));
    }
    if path.contains(':') {
        return Err(format!("artifact must not contain ':': '{path}'"));
    }
    if path.contains("..") {
        return Err(format!("artifact must not contain '..': '{path}'"));
    }
    if path.contains('\0') {
        return Err(format!("artifact must not contain null byte: '{path}'"));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Cycle detection (iterative DFS)
// ---------------------------------------------------------------------------

pub(crate) fn detect_cycles(tasks: &[Task]) -> Vec<Vec<String>> {
    let mut cycles = Vec::new();
    let mut visited = HashSet::new();
    let mut stack = Vec::new();
    let mut on_stack = HashSet::new();

    fn visit(
        task: &Task,
        tasks: &[Task],
        visited: &mut HashSet<String>,
        stack: &mut Vec<String>,
        on_stack: &mut HashSet<String>,
        cycles: &mut Vec<Vec<String>>,
    ) {
        let id = &task.source_id;
        if on_stack.contains(id) {
            let start = stack.iter().position(|s| s == id).unwrap_or(0);
            let mut cycle: Vec<String> = stack[start..].to_vec();
            cycle.push(id.clone());
            cycles.push(cycle);
            return;
        }
        if visited.contains(id) {
            return;
        }
        visited.insert(id.clone());
        on_stack.insert(id.clone());
        stack.push(id.clone());

        for dep in &task.spec.needs {
            if let Some(dep_task) = find_dependency_task(tasks, dep) {
                visit(dep_task, tasks, visited, stack, on_stack, cycles);
            }
        }

        on_stack.remove(id);
        stack.pop();
    }

    for task in tasks {
        visit(
            task,
            tasks,
            &mut visited,
            &mut stack,
            &mut on_stack,
            &mut cycles,
        );
    }
    cycles.dedup_by(|a, b| a.len() == b.len() && a.iter().all(|x| b.contains(x)));
    cycles
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_artifact_path() {
        assert!(validate_artifact_path("src/main.rs").is_ok());
        assert!(validate_artifact_path("docs/bench_notes/plan.md").is_ok());
        assert!(validate_artifact_path("/etc/passwd").is_err());
        assert!(validate_artifact_path("../escape").is_err());
        assert!(validate_artifact_path("C:\\Windows").is_err());
    }
}
