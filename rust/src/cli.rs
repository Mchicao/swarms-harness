//! CLI argument parsing.

use crate::model;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
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
    /// `singularity` only: number of bounded coordinator cycles to run.
    pub max_cycles: u32,
}

pub fn parse_args() -> Result<Args> {
    let mut values = std::env::args().skip(1);
    let command = values
        .next()
        .ok_or("usage: swarms-rs <doctor|review|dry-run|run|singularity> --plan <file>")?;

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
            max_cycles: 0,
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
    let mut max_cycles = if command == "singularity" { 5 } else { 0 };

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
            "--max-cycles" => {
                if command != "singularity" {
                    return Err("--max-cycles is only valid for singularity".to_string());
                }
                max_cycles = parse_cycle_count(
                    values
                        .next()
                        .ok_or("--max-cycles needs a value".to_string())?
                        .as_str(),
                )?;
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
    if command == "singularity" && (force || resume) {
        return Err(
            "singularity creates fresh cycle runs; --force/--resume are not valid".to_string(),
        );
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
        max_cycles,
    })
}

/// Parse the `--max-cycles` value for `singularity`. Bounded to [1, 1000] so
/// an autonomous loop can never run unbounded by accident.
pub(crate) fn parse_cycle_count(value: &str) -> Result<u32> {
    let n = value
        .parse::<u32>()
        .map_err(|_| "max-cycles must be a positive integer".to_string())?;
    if !(1..=1000).contains(&n) {
        return Err("max-cycles must be between 1 and 1000".to_string());
    }
    Ok(n)
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

/// Commands whose plan must live inside the launcher directory unless
/// `--workspace-root` is given. New executable commands must opt in here
/// consciously; otherwise they silently skip the external-plan gate.
pub fn is_workspace_gated(command: &str) -> bool {
    matches!(command, "dry-run" | "run" | "singularity")
}

/// Strip the Windows verbatim `\\?\` prefix that `canonicalize()` produces.
/// Verbatim forms leak into worker prompts, session identity and reports,
/// where many tools (git -C, npm, rg) reject them.
fn non_verbatim(path: PathBuf) -> PathBuf {
    let text = path.as_os_str().to_string_lossy();
    if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
        PathBuf::from(format!(r"\\{rest}"))
    } else if let Some(rest) = text.strip_prefix(r"\\?\") {
        PathBuf::from(rest.to_string())
    } else {
        path
    }
}

/// Resolve a stable target workspace and fail closed on the common
/// "launcher repo != target repo" mistake. The returned path never uses the
/// Windows verbatim form; canonical forms are kept only for containment
/// comparisons.
pub fn resolve_workspace_root(
    current_dir: &Path,
    plan_path: &Path,
    explicit: Option<&Path>,
    command: &str,
) -> Result<PathBuf> {
    let current_canon = current_dir.canonicalize().map_err(|e| {
        format!(
            "cannot resolve current directory {}: {e}",
            current_dir.display()
        )
    })?;
    let current = non_verbatim(current_canon.clone());
    if let Some(path) = explicit {
        let candidate = if path.is_absolute() {
            path.to_path_buf()
        } else {
            current.join(path)
        };
        return candidate
            .canonicalize()
            .map_err(|e| format!("cannot resolve workspace root {}: {e}", candidate.display()))
            .and_then(|resolved| {
                if resolved.is_dir() {
                    Ok(non_verbatim(resolved))
                } else {
                    Err(format!(
                        "workspace root is not a directory: {}",
                        non_verbatim(resolved).display()
                    ))
                }
            });
    }

    if is_workspace_gated(command) {
        let candidate = if plan_path.is_absolute() {
            plan_path.to_path_buf()
        } else {
            current.join(plan_path)
        };
        let plan = candidate
            .canonicalize()
            .map_err(|e| format!("cannot resolve plan {}: {e}", candidate.display()))?;
        if !plan.starts_with(&current_canon) {
            return Err(format!(
                "--workspace-root is required because plan {} is outside launcher directory {}",
                non_verbatim(plan).display(),
                current.display()
            ));
        }
    }
    Ok(current)
}

/// Persist all run state inside the selected target workspace.
pub fn run_dir(workspace_root: &Path, run_id: &str) -> PathBuf {
    workspace_root.join(".agent/swarm/runs").join(run_id)
}

/// Resolve the effective global concurrency from CLI override or plan budget.
pub fn effective_global_cap(cli_override: Option<usize>, plan: &model::Plan) -> usize {
    cli_override.unwrap_or(plan.budget_policy.global_max_concurrency.max(1))
}

#[cfg(test)]
mod workspace_tests {
    use super::*;
    use std::fs;

    fn temp_dir(label: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "swarms-cli-{label}-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn execution_requires_workspace_for_external_plan() {
        let launcher = temp_dir("launcher");
        let target = temp_dir("target");
        let plan = target.join("plan.json");
        fs::write(&plan, "{}").unwrap();

        let error = resolve_workspace_root(&launcher, &plan, None, "run").unwrap_err();
        assert!(error.contains("--workspace-root is required"));

        fs::remove_dir_all(launcher).unwrap();
        fs::remove_dir_all(target).unwrap();
    }

    #[test]
    fn explicit_workspace_owns_run_state() {
        let launcher = temp_dir("launcher-explicit");
        let target = temp_dir("target-explicit");
        let plan = target.join("plan.json");
        fs::write(&plan, "{}").unwrap();

        let workspace = resolve_workspace_root(&launcher, &plan, Some(&target), "run").unwrap();
        assert_eq!(workspace, non_verbatim(target.canonicalize().unwrap()));
        assert_eq!(
            run_dir(&workspace, "proof"),
            workspace.join(".agent/swarm/runs/proof")
        );

        fs::remove_dir_all(launcher).unwrap();
        fs::remove_dir_all(target).unwrap();
    }

    #[test]
    fn default_workspace_maps_launcher_and_never_leaks_verbatim_paths() {
        let launcher = temp_dir("launcher-default");
        let plan = launcher.join("plan.json");
        fs::write(&plan, "{}").unwrap();

        let workspace = resolve_workspace_root(&launcher, &plan, None, "run").unwrap();
        let text = workspace.to_string_lossy().to_string();
        assert!(!text.contains(r"\\?\"), "verbatim path leaked: {text}");
        assert_eq!(workspace, non_verbatim(launcher.canonicalize().unwrap()));

        fs::remove_dir_all(launcher).unwrap();
    }

    #[test]
    fn review_command_is_exempt_from_the_external_plan_gate() {
        let launcher = temp_dir("launcher-review");
        let outside = temp_dir("outside-review");
        let plan = outside.join("plan.json");
        fs::write(&plan, "{}").unwrap();

        let workspace = resolve_workspace_root(&launcher, &plan, None, "review").unwrap();
        let text = workspace.to_string_lossy().to_string();
        assert!(!text.contains(r"\\?\"), "verbatim path leaked: {text}");
        assert_eq!(workspace, non_verbatim(launcher.canonicalize().unwrap()));

        fs::remove_dir_all(launcher).unwrap();
        fs::remove_dir_all(outside).unwrap();
    }
}
