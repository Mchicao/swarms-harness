//! swarms-rs — self-contained deterministic SWARMS workflow coordinator.

use std::env;
use std::io::Read;
use std::path::Path;
use swarms_runtime::{cli, config, model::Router, observer, review, runtime};

type Result<T> = std::result::Result<T, String>;

fn main() {
    if let Err(error) = run() {
        eprintln!("[swarms-rs] ERROR: {error}");
        std::process::exit(2);
    }
}

fn run() -> Result<()> {
    let args = cli::parse_args()?;
    let root = env::current_dir().map_err(|e| e.to_string())?;

    let router_path = cli::resolve_router_path(&root, &args.router_config);
    let router = config::load_router_from_path(&root, &router_path)?;

    if args.command == "doctor" {
        return print_doctor(&root, &router);
    }

    if args.command == "observe" {
        let mut prompt = String::new();
        std::io::stdin()
            .read_to_string(&mut prompt)
            .map_err(|e| format!("read observer prompt from stdin: {e}"))?;
        let route = args
            .observer_route
            .as_deref()
            .ok_or_else(|| "observe route missing".to_string())?;
        let output = observer::run(&router, route, &prompt, args.observer_thinking)?;
        println!(
            "{}",
            serde_json::to_string(&output).map_err(|e| e.to_string())?
        );
        return Ok(());
    }

    let workspace_root = cli::resolve_workspace_root(
        &root,
        &args.plan,
        args.workspace_root.as_deref(),
        &args.command,
    )?;

    let plan = config::load_plan(&args.plan)?;
    let tasks = config::build_tasks(&plan, &router)?;

    let review_result = review::review_plan(&plan, &router, &tasks);
    if args.command == "review" {
        println!(
            "{}",
            serde_json::to_string_pretty(&review_result).map_err(|e| e.to_string())?
        );
        if !review_result.ok {
            return Err("plan review failed".to_string());
        }
        return Ok(());
    }

    if !review_result.ok {
        return Err(format!(
            "plan review failed with {} error(s); run 'review' for details",
            review_result.errors
        ));
    }

    let global_cap = cli::effective_global_cap(args.global_cap, &plan);
    let caps = config::effective_caps(&plan, &args.caps, &router);

    if args.command == "dry-run" {
        let run_dir = cli::run_dir(&workspace_root, &args.run_id);
        let report = runtime::dry_run(
            &run_dir,
            &workspace_root,
            &args.run_id,
            &tasks,
            &plan,
            global_cap,
            &caps,
        )?;
        println!(
            "{}",
            serde_json::to_string_pretty(&report).map_err(|e| e.to_string())?
        );
        return Ok(());
    }

    if args.command != "run" && args.command != "singularity" {
        return Err(format!("unsupported command: {}", args.command));
    }

    if args.command == "singularity" {
        return run_singularity_loop(
            &root,
            &workspace_root,
            &plan,
            &router,
            &tasks,
            global_cap,
            &caps,
            &args.run_id,
            args.max_cycles,
        );
    }

    let report = runtime::execute(
        &root,
        &workspace_root,
        &tasks,
        &plan,
        &router,
        global_cap,
        &caps,
        &args.run_id,
        args.force,
        args.resume,
    )?;

    println!(
        "{}",
        serde_json::to_string_pretty(&report).map_err(|e| e.to_string())?
    );

    if !report.is_completed() {
        return Err("one or more workers failed".to_string());
    }
    Ok(())
}

/// Run a bounded autonomous loop where every cycle executes exclusively
/// through the Rust coordinator. This is the native replacement for the legacy
/// `scripts/start_singularity*.ps1` + `scripts/architect.py` path, which
/// bypassed the scheduler, quotas, state contract, verification, and resume.
///
/// Each cycle reuses normal plan review, routing, state, verification, and
/// reporting. Create a `STOP_SINGULARITY` file in the workspace root to stop
/// before the next cycle. A failed cycle does not abort the loop: it reports
/// the failure and continues to the next cycle so the loop can self-correct,
/// mirroring the legacy "continue to next cycle to fix" behavior.
#[allow(clippy::too_many_arguments)]
fn run_singularity_loop(
    root: &Path,
    workspace_root: &Path,
    plan: &swarms_runtime::model::Plan,
    router: &Router,
    tasks: &[swarms_runtime::model::Task],
    global_cap: usize,
    caps: &std::collections::HashMap<String, usize>,
    base_run_id: &str,
    max_cycles: u32,
) -> Result<()> {
    let stop_file = workspace_root.join("STOP_SINGULARITY");
    let mut cycles_run = 0;
    let mut failed = false;
    for cycle in 1..=max_cycles {
        if stop_file.exists() {
            println!("[singularity-rs] STOP_SINGULARITY detected; halting before cycle {cycle}.");
            break;
        }

        cycles_run = cycle;
        let run_id = format!("{base_run_id}-c{cycle:03}");
        println!("[singularity-rs] Cycle {cycle}/{max_cycles}: {run_id} (via Rust coordinator)");

        match runtime::execute(
            root,
            workspace_root,
            tasks,
            plan,
            router,
            global_cap,
            caps,
            &run_id,
            true, // each cycle is a fresh run
            false,
        ) {
            Ok(report) => {
                let status = if report.is_completed() {
                    "completed"
                } else {
                    failed = true;
                    "incomplete"
                };
                println!("[singularity-rs] Cycle {cycle} {status} ({run_id}).");
            }
            Err(e) => {
                failed = true;
                // A cycle failure is reported but does not abort the loop; the
                // next cycle may self-correct. Only the coordinator-aborting
                // errors (returned as Err) reach here.
                println!("[singularity-rs] Cycle {cycle} failed ({run_id}): {e}");
            }
        }
    }
    println!("[singularity-rs] Loop finished after {cycles_run} cycle(s).");
    if failed {
        Err("one or more singularity cycles failed".to_string())
    } else {
        Ok(())
    }
}

fn print_doctor(root: &Path, router: &Router) -> Result<()> {
    let os = std::env::consts::OS;
    println!("[OK] Rust coordinator available on {os}");
    println!("[OK] router loaded ({} providers)", router.providers.len());

    let mock = router.providers.get("mock");
    match mock {
        Some(p) if p.enabled => println!("[OK] mock provider enabled (offline-safe)"),
        Some(_) => println!("[WARN] mock provider disabled — offline tests will fail"),
        None => println!("[WARN] no mock provider in router config"),
    }

    let real_enabled: Vec<&str> = router
        .providers
        .keys()
        .filter(|k| *k != "mock")
        .filter(|k| router.providers.get(*k).is_some_and(|p| p.enabled))
        .map(String::as_str)
        .collect();
    if real_enabled.is_empty() {
        println!("[OK] no real providers enabled (offline-safe)");
    } else {
        println!(
            "[WARN] real providers enabled: {} — verify secrets are local",
            real_enabled.join(", ")
        );
    }

    // Check supported wrappers
    let wrappers: std::collections::HashSet<&str> = router
        .providers
        .values()
        .map(|p| p.wrapper.as_str())
        .collect();
    for w in &wrappers {
        if router
            .providers
            .values()
            .any(|provider| provider.enabled && provider.wrapper == *w)
            && swarms_runtime::adapter::AdapterKind::from_wrapper(w).is_none()
        {
            println!("[WARN] unknown wrapper '{w}' in router config");
        }
    }

    // Quick plan review
    let plan_path = root.join("docs/workflow_plan_example.json");
    if plan_path.exists() {
        match config::load_plan(&plan_path) {
            Ok(plan) => match config::build_tasks(&plan, router) {
                Ok(tasks) => {
                    let result = review::review_plan(&plan, router, &tasks);
                    if result.ok {
                        println!("[OK] example plan review passed ({} tasks)", tasks.len());
                    } else {
                        println!("[WARN] example plan has {} review error(s)", result.errors);
                    }
                }
                Err(e) => println!("[WARN] example plan build failed: {e}"),
            },
            Err(e) => println!("[WARN] example plan parse failed: {e}"),
        }
    }

    Ok(())
}
