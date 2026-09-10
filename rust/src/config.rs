//! Configuration loading: router overlay merge and plan parsing.

use crate::model::{self, Plan, Router};
use serde_json::Value;
use std::fs;
use std::path::Path;

type Result<T> = std::result::Result<T, String>;

const ROUTER_FIELDS: &[&str] = &[
    "_schema",
    "version",
    "fallback_route",
    "quota_policy",
    "preferences",
    "aliases",
    "role_routes",
    "providers",
];

const QUOTA_POLICY_FIELDS: &[&str] = &[
    "enabled",
    "snapshot_path",
    "min_remaining_percent",
    "max_age_seconds",
    "on_unknown",
];

// Compatibility-only metadata retained by the public router files. These fields
// are descriptive and do not participate in route selection in the Rust runtime.
const PREFERENCE_FIELDS: &[&str] = &[
    "_quality_weight",
    "quality_weight",
    "_cost_weight",
    "cost_weight",
    "_quota_saving_weight",
    "quota_saving_weight",
];

const PROVIDER_FIELDS: &[&str] = &[
    "_doc",
    "enabled",
    "provider",
    "model",
    "canonical_model",
    "wrapper",
    "cost_class",
    "host_id",
    "key_env",
    "base_url",
    "base_url_env",
    "thinking_field",
    "quota_key",
    "fallback_routes",
    // Compatibility-only descriptive metadata. Keep the allowlist explicit so
    // typos and newly invented executable-looking fields still fail closed.
    "variant",
    "health_key",
    "metric_key",
    "relative_cost",
    "quality",
    "scarcity",
    "strengths",
    "weaknesses",
];

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

fn validate_object_fields(value: &Value, path: &str, allowed: &[&str]) -> Result<()> {
    let object = value
        .as_object()
        .ok_or_else(|| format!("router config: '{path}' must be an object"))?;
    for key in object.keys() {
        if !allowed.contains(&key.as_str()) {
            return Err(format!("router config: unknown field '{path}.{key}'"));
        }
    }
    Ok(())
}

fn validate_router_config(value: &Value) -> Result<()> {
    validate_object_fields(value, "router", ROUTER_FIELDS)?;
    let root = value
        .as_object()
        .ok_or_else(|| "router config: 'router' must be an object".to_string())?;

    if let Some(quota_policy) = root.get("quota_policy") {
        validate_object_fields(quota_policy, "quota_policy", QUOTA_POLICY_FIELDS)?;
    }
    if let Some(preferences) = root.get("preferences") {
        validate_object_fields(preferences, "preferences", PREFERENCE_FIELDS)?;
    }
    if let Some(providers) = root.get("providers") {
        let providers = providers
            .as_object()
            .ok_or_else(|| "router config: 'providers' must be an object".to_string())?;
        for (route, provider) in providers {
            validate_object_fields(provider, &format!("providers.{route}"), PROVIDER_FIELDS)?;
        }
    }
    Ok(())
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
    validate_router_config(&value)?;
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

#[cfg(test)]
mod tests {
    use super::{merge, validate_router_config};
    use serde_json::json;

    #[test]
    fn router_validation_rejects_unknown_root_field() {
        let value = json!({"providers": {}, "preferneces": {}});
        let error = validate_router_config(&value).expect_err("unknown root field must fail");
        assert!(error.contains("router.preferneces"), "{error}");
    }

    #[test]
    fn router_validation_rejects_unknown_nested_fields() {
        let quota = json!({"providers": {}, "quota_policy": {"max_age_second": 600}});
        let quota_error =
            validate_router_config(&quota).expect_err("unknown quota field must fail");
        assert!(
            quota_error.contains("quota_policy.max_age_second"),
            "{quota_error}"
        );

        let provider = json!({
            "providers": {
                "glm": {
                    "enabled": true,
                    "provider": "opencode",
                    "model": "glm",
                    "wrapper": "opencode",
                    "modle": "typo"
                }
            }
        });
        let provider_error =
            validate_router_config(&provider).expect_err("unknown provider field must fail");
        assert!(
            provider_error.contains("providers.glm.modle"),
            "{provider_error}"
        );
    }

    #[test]
    fn router_validation_accepts_declared_compatibility_metadata() {
        let value = json!({
            "_schema": "docs",
            "version": "legacy",
            "preferences": {
                "_quality_weight": "doc",
                "quality_weight": 0.5,
                "cost_weight": 0.4,
                "quota_saving_weight": 0.1
            },
            "providers": {
                "glm": {
                    "_doc": "route docs",
                    "enabled": false,
                    "provider": "opencode",
                    "model": "glm",
                    "canonical_model": "glm",
                    "wrapper": "opencode",
                    "variant": "high",
                    "health_key": "opencode",
                    "metric_key": "glm",
                    "relative_cost": 0.2,
                    "quality": 0.8,
                    "scarcity": 0.2,
                    "strengths": ["coding"],
                    "weaknesses": ["vision"]
                }
            }
        });
        validate_router_config(&value)
            .expect("declared compatibility metadata must remain accepted");
    }

    #[test]
    fn merged_overlay_unknown_field_is_rejected() {
        let mut base = json!({"providers": {"mock": {"enabled": true, "provider": "mock", "model": "mock", "wrapper": "mock"}}});
        merge(
            &mut base,
            json!({"providers": {"mock": {"wrappper": "mock"}}}),
        );
        let error = validate_router_config(&base).expect_err("overlay typo must fail after merge");
        assert!(error.contains("providers.mock.wrappper"), "{error}");
    }
}
