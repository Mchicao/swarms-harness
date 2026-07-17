//! Configuration loading: router overlay merge and plan parsing.

use crate::model::{self, Plan, Router};
use serde_json::Value;
use std::fs;
use std::path::Path;

type Result<T> = std::result::Result<T, String>;

/// Read and parse a JSON file.
pub fn load_json(path: &Path) -> Result<Value> {
    let text = fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))
}

/// Deep-merge `local` into `base` (in-place).  Object keys are merged
/// recursively; non-object values are overwritten.
pub fn merge(base: &mut Value, local: Value) {
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

/// Load router from `config/swarm_router.json` with optional
/// `config/swarm_router.local.json` overlay.
pub fn load_router(root: &Path) -> Result<Router> {
    load_router_from_path(root, &root.join("config/swarm_router.json"))
}

/// Load a router from an explicit base file while preserving the ignored local
/// overlay rooted in the workspace.
pub fn load_router_from_path(root: &Path, base_path: &Path) -> Result<Router> {
    let base_path = if base_path.is_absolute() {
        base_path.to_path_buf()
    } else {
        root.join(base_path)
    };
    let mut value = load_json(&base_path)?;
    let local = root.join("config/swarm_router.local.json");
    if local.exists() {
        merge(&mut value, load_json(&local)?);
    }
    let router: Router =
        serde_json::from_value(value).map_err(|e| format!("router config: {e}"))?;
    if router.quota_policy.enabled {
        let policy = &router.quota_policy;
        if !(0.0..=100.0).contains(&policy.min_remaining_percent) {
            return Err("quota_policy.min_remaining_percent must be between 0 and 100".to_string());
        }
        if policy.snapshot_path.trim().is_empty() || policy.max_age_seconds == 0 {
            return Err(
                "quota_policy needs a non-empty snapshot_path and max_age_seconds > 0".to_string(),
            );
        }
    }
    Ok(router)
}

/// Parse a plan JSON file.
pub fn load_plan(path: &Path) -> Result<Plan> {
    let value = crate::workflow_ir::compile_plan(load_json(path)?)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    serde_json::from_value(value).map_err(|e| format!("{}: {e}", path.display()))
}

/// Build resolved [`Task`] list from a plan and router.
pub fn build_tasks(plan: &Plan, router: &Router) -> Result<Vec<model::Task>> {
    let mut tasks = Vec::new();
    for stage in &plan.stages {
        for spec in &stage.tasks {
            let provider = router.get_provider(&spec.route).ok_or_else(|| {
                format!(
                    "unknown route '{}' (resolved: '{}')",
                    spec.route,
                    router.resolve_route(&spec.route)
                )
            })?;
            if !provider.enabled {
                return Err(format!("route is disabled: {}", spec.route));
            }
            if provider.model.is_empty() && provider.provider != "mock" {
                return Err(format!("route '{}' must pin a model", spec.route));
            }
            if provider.wrapper.is_empty() {
                return Err(format!("route '{}' has no wrapper", spec.route));
            }
            let id = model::task_index_to_id(tasks.len(), &spec.id);
            tasks.push(model::Task {
                id,
                source_id: spec.id.clone(),
                stage: stage.name.clone(),
                stage_parallel: stage.parallel,
                spec: spec.clone(),
                provider: provider.clone(),
                effective_route: router.resolve_route(&spec.route).to_string(),
            });
        }
    }
    if tasks.is_empty() {
        return Err("plan has no tasks".to_string());
    }
    Ok(tasks)
}

/// Compute effective per-route concurrency caps.
/// Plan caps are the base; CLI overrides take precedence.
pub fn effective_caps(
    plan: &Plan,
    overrides: &std::collections::HashMap<String, usize>,
    router: &Router,
) -> std::collections::HashMap<String, usize> {
    let mut caps = std::collections::HashMap::new();
    for (route, cap) in &plan.budget_policy.provider_concurrency {
        caps.insert(router.resolve_route(route).to_string(), *cap);
    }
    for (route, cap) in overrides {
        caps.insert(router.resolve_route(route).to_string(), *cap);
    }
    caps
}
