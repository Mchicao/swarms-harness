//! CLI argument parsing.

use crate::model;
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

type Result<T> = std::result::Result<T, String>;

pub struct Args {
    pub command: String,
    pub plan: PathBuf,
    pub run_id: String,
    pub force: bool,
    pub resume: bool,
    pub workspace_root: Option<PathBuf>,
    pub global_cap: Option<usize>,
    pub caps: HashMap<String, usize>,
    pub router_config: Option<PathBuf>,
}

pub fn parse_args() -> Result<Args> {
    let mut values = std::env::args().skip(1);
    let command = values
        .next()
        .ok_or("usage: swarms-rs <doctor|review|dry-run|run> --plan <file>")?;

    if command == "doctor" {
        return Ok(Args {
            command,
            plan: PathBuf::new(),
            run_id: make_run_id(),
            force: false,
            resume: false,
            workspace_root: None,
            global_cap: None,
            caps: HashMap::new(),
            router_config: None,
        });
    }

    let mut plan = None;
    let mut run_id = make_run_id();
    let mut force = false;
    let mut resume = false;
    let mut workspace_root = None;
    let mut global_cap = None;
    let mut caps = HashMap::new();
    let mut router_config = None;

    while let Some(arg) = values.next() {
        match arg.as_str() {
            "--plan" => {
                plan = Some(PathBuf::from(
                    values.next().ok_or("--plan needs a file".to_string())?,
                ))
            }
            "--run-id" => run_id = values.next().ok_or("--run-id needs a value".to_string())?,
            "--force" => force = true,
            "--resume" => resume = true,
            "--workspace-root" => {
                workspace_root = Some(PathBuf::from(
                    values
                        .next()
                        .ok_or("--workspace-root needs a path".to_string())?,
                ))
            }
            "--global-max-concurrency" => {
                global_cap = Some(parse_positive(
                    &values.next().ok_or("missing global cap".to_string())?,
                )?)
            }
            "--provider-cap" => {
                let pair = values
                    .next()
                    .ok_or("--provider-cap needs route=count".to_string())?;
                let (route, count) = pair
                    .split_once('=')
                    .ok_or("provider cap must be route=count".to_string())?;
                caps.insert(route.to_string(), parse_positive(count)?);
            }
            "--router-config" => {
                router_config = Some(PathBuf::from(
                    values
                        .next()
                        .ok_or("--router-config needs a path".to_string())?,
                ))
            }
            other => return Err(format!("unknown argument: {other}")),
        }
    }

    if !safe_run_id(&run_id) {
        return Err(
            "run id must contain only letters, numbers, dot, underscore, or dash".to_string(),
        );
    }
    if force && resume {
        return Err("--force and --resume are mutually exclusive".to_string());
    }

    Ok(Args {
        command,
        plan: plan.ok_or("--plan is required")?,
        run_id,
        force,
        resume,
        workspace_root,
        global_cap,
        caps,
        router_config,
    })
}

fn parse_positive(value: &str) -> Result<usize> {
    value
        .parse::<usize>()
        .ok()
        .filter(|v| *v > 0)
        .ok_or_else(|| "capacity must be positive".to_string())
}

pub fn make_run_id() -> String {
    format!(
        "rs-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
    )
}

pub fn safe_run_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-'))
}

/// Resolve the router config path, respecting `--router-config` override.
pub fn resolve_router_path(root: &std::path::Path, override_path: &Option<PathBuf>) -> PathBuf {
    match override_path {
        Some(p) => p.clone(),
        None => root.join("config/swarm_router.json"),
    }
}

/// Resolve the effective global concurrency from CLI override or plan budget.
pub fn effective_global_cap(cli_override: Option<usize>, plan: &model::Plan) -> usize {
    cli_override.unwrap_or(plan.budget_policy.global_max_concurrency.max(1))
}
