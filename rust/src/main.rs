//! swarms-rs — self-contained deterministic SWARMS workflow coordinator.

use std::env;
use std::path::Path;
use swarms_runtime::{cli, config, model::Router, review, runtime};

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
    let workspace_root = args.workspace_root.clone().unwrap_or_else(|| root.clone());
    if !workspace_root.is_dir() {
        return Err(format!(
            "workspace root is not a directory: {}",
            workspace_root.display()
        ));
    }

    let router_path = cli::resolve_router_path(&root, &args.router_config);
    let router = config::load_router_from_path(&root, &router_path)?;

    if args.command == "doctor" {
        return print_doctor(&root, &router);
    }

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
        let run_dir = root.join(".agent/swarm/runs").join(&args.run_id);
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

    if args.command != "run" {
        return Err(format!("unsupported command: {}", args.command));
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
        if swarms_runtime::adapter::AdapterKind::from_wrapper(w).is_none() {
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
