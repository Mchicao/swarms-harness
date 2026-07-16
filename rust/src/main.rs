use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::env;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

type Result<T> = std::result::Result<T, String>;

#[derive(Clone, Debug, Deserialize)]
struct Provider {
    #[serde(default)]
    enabled: bool,
    provider: String,
    model: String,
    #[serde(default)]
    variant: Option<String>,
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
    #[serde(default, alias = "parent_id")]
    parent_task_id: Option<String>,
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
    subagents: Vec<String>,
}

#[derive(Deserialize, Serialize)]
struct TaskResult {
    task_id: String,
    source_id: String,
    route: String,
    provider: String,
    model: String,
    checkpoint_key: String,
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
    resume: bool,
    workspace_root: Option<PathBuf>,
    global_cap: Option<usize>,
    caps: HashMap<String, usize>,
    allow_unverified_agents: bool,
}

#[derive(Clone, Copy)]
enum RestartMode {
    Fresh,
    Force,
    Resume,
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
    let workspace_root = args.workspace_root.clone().unwrap_or_else(|| root.clone());
    if !workspace_root.is_dir() {
        return Err(format!(
            "workspace root is not a directory: {}",
            workspace_root.display()
        ));
    }
    let router = load_router(&root)?;
    let preflight = agent_preflight(&router);
    if args.command == "doctor" || args.command == "preflight" {
        println!("[OK] Rust coordinator available on {}", env::consts::OS);
        println!(
            "{}",
            serde_json::to_string_pretty(&preflight).map_err(|e| e.to_string())?
        );
        return Ok(());
    }
    let plan = load_plan(&args.plan)?;
    let tasks = build_tasks(&plan, &router)?;
    if args.command == "run" && !args.allow_unverified_agents {
        let blocked: Vec<String> = tasks
            .iter()
            .filter(|task| task.provider.wrapper != "mock")
            .filter_map(|task| {
                let status = preflight
                    .get("routes")
                    .and_then(Value::as_array)
                    .and_then(|routes| {
                        routes.iter().find(|route| {
                            route.get("id").and_then(Value::as_str)
                                == Some(task.spec.route.as_str())
                        })
                    })
                    .and_then(|route| route.get("status"))
                    .and_then(Value::as_str)
                    .unwrap_or("unknown");
                (status != "ready").then(|| format!("{}:{}", task.spec.route, status))
            })
            .collect();
        if !blocked.is_empty() {
            return Err(format!(
                "agent preflight blocked dispatch: {}",
                blocked.join(", ")
            ));
        }
    }
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
            json!({"status": "planned", "task_count": tasks.len(), "workspace_root": workspace_root, "global_max_concurrency": global_cap, "provider_max_concurrency": caps})
        );
        return Ok(());
    }
    if args.command != "run" {
        return Err(format!("unsupported command: {}", args.command));
    }
    let report = execute(
        &root,
        &workspace_root,
        tasks,
        global_cap,
        caps,
        &args.run_id,
        restart_mode(args.force, args.resume)?,
    )?;
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
    if command == "doctor" || command == "preflight" {
        return Ok(Args {
            command,
            plan: PathBuf::new(),
            run_id: make_run_id(),
            force: false,
            resume: false,
            workspace_root: None,
            global_cap: None,
            caps: HashMap::new(),
            allow_unverified_agents: false,
        });
    }
    let mut plan = None;
    let mut run_id = make_run_id();
    let mut force = false;
    let mut resume = false;
    let mut workspace_root = None;
    let mut global_cap = None;
    let mut caps = HashMap::new();
    let mut allow_unverified_agents = false;
    while let Some(value) = values.next() {
        match value.as_str() {
            "--plan" => plan = Some(PathBuf::from(values.next().ok_or("--plan needs a file")?)),
            "--run-id" => run_id = values.next().ok_or("--run-id needs a value")?,
            "--force" => force = true,
            "--resume" => resume = true,
            "--workspace-root" => {
                workspace_root = Some(PathBuf::from(
                    values.next().ok_or("--workspace-root needs a directory")?,
                ))
            }
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
            "--allow-unverified-agents" => allow_unverified_agents = true,
            other => return Err(format!("unknown argument: {other}")),
        }
    }
    if !safe_run_id(&run_id) {
        return Err(
            "run id must contain only letters, numbers, dot, underscore, or dash".to_string(),
        );
    }
    restart_mode(force, resume)?;
    Ok(Args {
        command,
        plan: plan.ok_or("--plan is required")?,
        run_id,
        force,
        resume,
        workspace_root,
        global_cap,
        caps,
        allow_unverified_agents,
    })
}

fn restart_mode(force: bool, resume: bool) -> Result<RestartMode> {
    if force && resume {
        return Err("--force and --resume are mutually exclusive".to_string());
    }
    Ok(if force {
        RestartMode::Force
    } else if resume {
        RestartMode::Resume
    } else {
        RestartMode::Fresh
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

fn command_exists(command: &str) -> bool {
    let probe = if cfg!(windows) { "where" } else { "which" };
    Command::new(probe)
        .arg(command)
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

fn provider_command(route: &str, provider: &Provider) -> Option<&'static str> {
    if route == "mock" || provider.wrapper == "mock" {
        return None;
    }
    match provider.wrapper.as_str() {
        "opencode" => Some("opencode"),
        "gemini" => {
            if command_exists("agy") {
                Some("agy")
            } else {
                Some("gemini")
            }
        }
        "codex" => Some("codex"),
        "kilo" => Some("kilo"),
        "hermes" => Some("hermes"),
        _ => None,
    }
}

fn agent_preflight(router: &Router) -> Value {
    let routes: Vec<Value> = router
        .providers
        .iter()
        .map(|(route, provider)| {
            let command = provider_command(route, provider);
            let installed = command.map(command_exists).unwrap_or(true);
            let status = if !provider.enabled {
                "disabled"
            } else if route == "mock" || provider.wrapper == "mock" {
                "ready"
            } else if !installed {
                "missing_cli"
            } else {
                "unverified"
            };
            json!({
                "id": route,
                "provider": provider.provider.clone(),
                "model": provider.model.clone(),
                "wrapper": provider.wrapper.clone(),
                "command": command,
                "installed": installed,
                "auth_present": Value::Null,
                "enabled": provider.enabled,
                "status": status,
            })
        })
        .collect();
    json!({"routes": routes, "note": "real routes require an explicit model probe"})
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
                subagents: Vec::new(),
            });
        }
    }
    let mut children: HashMap<String, Vec<String>> = HashMap::new();
    for task in &tasks {
        if let Some(parent) = &task.spec.parent_task_id {
            children
                .entry(parent.clone())
                .or_default()
                .push(task.source_id.clone());
        }
    }
    for task in &mut tasks {
        task.subagents = children.remove(&task.source_id).unwrap_or_default();
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
    workspace_root: &Path,
    tasks: Vec<Task>,
    global_cap: usize,
    caps: HashMap<String, usize>,
    run_id: &str,
    restart: RestartMode,
) -> Result<Value> {
    let run_dir = root.join(".agent/swarm/runs").join(run_id);
    let run_exists = run_dir.exists();
    if run_dir.exists() {
        match restart {
            RestartMode::Force => fs::remove_dir_all(&run_dir).map_err(|e| e.to_string())?,
            RestartMode::Resume => {}
            RestartMode::Fresh => return Err(format!("run already exists: {}", run_id)),
        }
    } else if matches!(restart, RestartMode::Resume) {
        return Err(format!("cannot resume missing run: {run_id}"));
    }
    fs::create_dir_all(&run_dir).map_err(|e| e.to_string())?;
    if !matches!(restart, RestartMode::Resume) {
        write_json(
            &run_dir.join("workflow.json"),
            &json!({
                "state_schema_version": 1,
                "runtime": "rust",
                "run_id": run_id,
                "created_unix_ms": unix_ms(),
                "workspace_root": workspace_root,
                "heartbeat_interval_seconds": positive_env_u64("SWARMS_HEARTBEAT_SECONDS", 30),
                "global_max_concurrency": global_cap,
                "provider_max_concurrency": caps,
                "task_count": tasks.len(),
            }),
        )?;
        for task in &tasks {
            write_task_state(&run_dir, task, "pending", 0, None)?;
        }
        append_event(&run_dir, "workflow_initialized", None)?;
    } else {
        append_event(&run_dir, "workflow_resumed", None)?;
    }
    let mut complete = HashSet::new();
    let mut results = Vec::new();
    let mut pending = Vec::new();
    for task in tasks {
        if run_exists && matches!(restart, RestartMode::Resume) {
            if let Some(result) = load_completed_checkpoint(&run_dir, &task) {
                complete.insert(result.source_id.clone());
                results.push(result);
                continue;
            }
            // No valid checkpoint: the previous run crashed mid-task or the
            // definition changed. Emit a recovery event so operators can see
            // exactly which tasks are being re-dispatched.
            let _ = append_event(&run_dir, "task_recovered", Some(&task.id));
        }
        pending.push(task);
    }
    let resumed_task_count = results.len();
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
            let workspace_root = workspace_root.to_path_buf();
            let run_dir = run_dir.clone();
            thread::spawn(move || {
                let _ = sender.send(run_worker(&root, &workspace_root, &run_dir, task));
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
    let report = json!({"run_id": run_id, "status": if success { "completed" } else { "failed" }, "resumed_task_count": resumed_task_count, "task_counts": {"completed": results.iter().filter(|result| result.status == "completed").count(), "failed": results.iter().filter(|result| result.status != "completed").count()}, "results": results});
    write_json(&run_dir.join("report-rs.json"), &report)?;
    append_event(&run_dir, "workflow_finished", None)?;
    Ok(report)
}

fn checkpoint_key(task: &Task) -> String {
    let definition = format!(
        "{}\n{}\n{}\n{}\n{}\n{}\n{:?}\n{}\n{}\n{}\n{:?}\n{}",
        task.id,
        task.source_id,
        task.stage,
        task.spec.route,
        task.spec.task,
        task.spec.role,
        task.spec.needs,
        task.spec.tools_policy,
        task.provider.provider,
        task.provider.model,
        task.provider.variant,
        task.provider.wrapper,
    );
    let hash = definition
        .bytes()
        .fold(0xcbf29ce484222325_u64, |hash, byte| {
            (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3)
        });
    format!("{hash:016x}")
}

fn load_completed_checkpoint(run_dir: &Path, task: &Task) -> Option<TaskResult> {
    let path = run_dir
        .join("results")
        .join(&task.id)
        .join("result-rs.json");
    let result: TaskResult = serde_json::from_str(&fs::read_to_string(path).ok()?).ok()?;
    (result.status == "completed" && result.checkpoint_key == checkpoint_key(task))
        .then_some(result)
}

fn unix_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

fn positive_env_u64(name: &str, default: u64) -> u64 {
    env::var(name)
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(default)
}

fn task_attempts(run_dir: &Path, task: &Task) -> usize {
    let path = run_dir.join("tasks").join(format!("{}.json", task.id));
    load_json(&path)
        .ok()
        .and_then(|value| value["attempts"].as_u64())
        .unwrap_or(0) as usize
}

fn write_task_state(
    run_dir: &Path,
    task: &Task,
    status: &str,
    attempts: usize,
    error: Option<&str>,
) -> Result<()> {
    write_json(
        &run_dir.join("tasks").join(format!("{}.json", task.id)),
        &json!({
            "state_schema_version": 1,
            "task_id": task.id,
            "source_id": task.source_id,
            "agent_id": task.source_id,
            "parent_task_id": task.spec.parent_task_id,
            "subagents": task.subagents,
            "provider_subagent_visibility": "not_reported",
            "provider_subagents": [],
            "stage": task.stage,
            "role": task.spec.role,
            "needs": task.spec.needs,
            "route": task.spec.route,
            "provider": task.provider.provider,
            "model": task.provider.model,
            "status": status,
            "attempts": attempts,
            "heartbeat_unix_ms": unix_ms(),
            "error": error,
        }),
    )
}

fn append_event(run_dir: &Path, event: &str, task_id: Option<&str>) -> Result<()> {
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(run_dir.join("events.jsonl"))
        .map_err(|e| e.to_string())?;
    let item = json!({
        "time_unix_ms": unix_ms(),
        "event": event,
        "task_id": task_id,
    });
    writeln!(
        file,
        "{}",
        serde_json::to_string(&item).map_err(|e| e.to_string())?
    )
    .map_err(|e| e.to_string())
}

fn run_worker(root: &Path, workspace_root: &Path, run_dir: &Path, task: Task) -> TaskResult {
    let started = SystemTime::now();
    let attempts = task_attempts(run_dir, &task) + 1;
    let checkpoint_key = checkpoint_key(&task);
    let work_dir = run_dir.join("results").join(&task.id);
    let _ = write_task_state(run_dir, &task, "in_progress", attempts, None);
    let _ = append_event(run_dir, "task_started", Some(&task.id));
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
        if matches!(task.provider.wrapper.as_str(), "gemini" | "opencode") {
            // SWARMS-RS-001: El harness coordina repositorios externos explícitos.
            command.arg("--cwd").arg(workspace_root);
        }
        if task.provider.wrapper == "opencode" {
            if let Some(variant) = &task.provider.variant {
                command.arg("--variant").arg(variant);
            }
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
        let log = fs::File::create(work_dir.join("worker.log")).map_err(|e| e.to_string())?;
        let stderr = log.try_clone().map_err(|e| e.to_string())?;
        let mut child = command
            .stdout(Stdio::from(log))
            .stderr(Stdio::from(stderr))
            .spawn()
            .map_err(|e| e.to_string())?;
        let heartbeat_seconds = positive_env_u64("SWARMS_HEARTBEAT_SECONDS", 30);
        let timeout_seconds = positive_env_u64("SWARMS_WORKER_TIMEOUT", 600);
        let clock = Instant::now();
        let mut last_heartbeat = Instant::now();
        let exit_status = loop {
            if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
                break status;
            }
            if clock.elapsed() >= Duration::from_secs(timeout_seconds) {
                child.kill().map_err(|e| e.to_string())?;
                let _ = child.wait();
                return Err(format!("worker timed out after {timeout_seconds}s"));
            }
            if last_heartbeat.elapsed() >= Duration::from_secs(heartbeat_seconds) {
                write_task_state(run_dir, &task, "in_progress", attempts, None)?;
                append_event(run_dir, "task_heartbeat", Some(&task.id))?;
                last_heartbeat = Instant::now();
            }
            thread::sleep(Duration::from_millis(100));
        };
        if !exit_status.success() {
            return Err(format!("worker exited {:?}", exit_status.code()));
        }
        Ok(())
    })();
    let duration_ms = started.elapsed().unwrap_or(Duration::ZERO).as_millis();
    let (status, error) = match result {
        Ok(()) => ("completed", None),
        Err(error) => ("failed", Some(error)),
    };
    let _ = write_task_state(run_dir, &task, status, attempts, error.as_deref());
    let _ = append_event(run_dir, "task_finished", Some(&task.id));
    let result = TaskResult {
        task_id: task.id,
        source_id: task.source_id,
        route: task.spec.route,
        provider: task.provider.provider,
        model: task.provider.model,
        checkpoint_key,
        status: status.to_string(),
        duration_ms,
        error,
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

    fn mock_task() -> Task {
        Task {
            id: "0000-root".to_string(),
            source_id: "root".to_string(),
            stage: "Build".to_string(),
            spec: TaskSpec {
                id: "root".to_string(),
                route: "mock".to_string(),
                task: "Work".to_string(),
                role: "programmer".to_string(),
                needs: Vec::new(),
                parent_task_id: None,
                tools_policy: "none".to_string(),
            },
            provider: Provider {
                enabled: true,
                provider: "mock".to_string(),
                model: "mock-worker".to_string(),
                variant: None,
                wrapper: "mock".to_string(),
            },
            subagents: Vec::new(),
        }
    }
    #[test]
    fn run_ids_are_portable() {
        assert!(safe_run_id("windows-linux_macos.1"));
        assert!(!safe_run_id("../escape"));
    }
    #[test]
    fn slug_is_safe() {
        assert_eq!(slug("hello / world"), "hello---world");
    }
    #[test]
    fn resume_and_force_are_mutually_exclusive() {
        assert!(restart_mode(true, true).is_err());
        assert!(restart_mode(true, false).is_ok());
        assert!(restart_mode(false, true).is_ok());
    }
    #[test]
    fn completed_checkpoint_is_reusable_only_for_the_same_task_definition() {
        let run_dir = env::temp_dir().join(format!("swarms-rs-checkpoint-{}", unix_ms()));
        let task = mock_task();
        let result = TaskResult {
            task_id: task.id.clone(),
            source_id: task.source_id.clone(),
            route: task.spec.route.clone(),
            provider: task.provider.provider.clone(),
            model: task.provider.model.clone(),
            checkpoint_key: checkpoint_key(&task),
            status: "completed".to_string(),
            duration_ms: 1,
            error: None,
        };
        write_json(
            &run_dir.join("results/0000-root/result-rs.json"),
            &serde_json::to_value(result).unwrap(),
        )
        .unwrap();

        assert!(load_completed_checkpoint(&run_dir, &task).is_some());
        let mut changed = mock_task();
        changed.spec.task = "Different work".to_string();
        assert!(load_completed_checkpoint(&run_dir, &changed).is_none());

        fs::remove_dir_all(run_dir).unwrap();
    }
}
