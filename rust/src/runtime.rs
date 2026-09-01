//! Deterministic DAG scheduler with persisted state, retries, and resume.

use crate::acp::{self, Event};
use crate::adapter::{self, AdapterKind, CliSpec};
use crate::model::{
    find_dependency_task, ExecutionFallback, ExecutionTransport, Plan, Router, SessionMode, Task,
    ThinkingLevel,
};
use crate::quota::QuotaGuard;
use crate::session::{self, SessionDecision, SessionStore};
use crate::steering::{self, AppliedSteer, SteeringMode};
use crate::telemetry::{self, Report, TaskState, TaskStatus, Usage};
use crate::{claude_stream, codex_app_server, opencode_server};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::sync::Arc;
#[cfg(windows)]
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

type Result<T> = std::result::Result<T, String>;

const MAX_DEP_CONTEXT_CHARS: usize = 12_000;
const WORKER_CONSOLE_FINISHED_SENTINEL: &str = "__SWARMS_WORKER_FINISHED__";

#[derive(Clone, Debug)]
struct WorkerTerminal {
    backend: String,
    session: Option<String>,
    workspace_id: Option<String>,
    tab_id: Option<String>,
    pane_id: Option<String>,
}

// ---------------------------------------------------------------------------
// Atomic file writes
// ---------------------------------------------------------------------------

pub fn write_json_atomic(path: &Path, content: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    }
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, content).map_err(|e| format!("{}: {e}", tmp.display()))?;
    for attempt in 0..5u32 {
        match fs::rename(&tmp, path) {
            Ok(()) => return Ok(()),
            Err(_) if attempt < 4 => {
                thread::sleep(Duration::from_millis(20 * u64::from(attempt + 1)));
            }
            Err(e) => {
                let _ = fs::remove_file(&tmp);
                return Err(format!("{}: {e}", path.display()));
            }
        }
    }
    unreachable!()
}

fn write_json_value(path: &Path, value: &Value) -> Result<()> {
    let text = serde_json::to_string_pretty(value).map_err(|e| e.to_string())?;
    write_json_atomic(path, &text)
}

/// Append the canonical event envelope to `events.jsonl`.
///
/// The canonical top-level shape is:
/// ```jsonc
/// {"time": "<iso8601>", "time_unix_ms": 0, "event": "<type>", "task_id": null, "payload": {}}
/// ```
/// `task_id` is hoisted out of the payload when present so consumers (the UI)
/// can read it at one stable location instead of scanning both levels.
pub(crate) fn append_event(run_dir: &Path, event_type: &str, payload: Value) {
    let task_id = payload
        .get("task_id")
        .and_then(Value::as_str)
        .map(str::to_string);
    let mut item = json!({
        "time": now_iso(),
        "time_unix_ms": unix_ms(),
        "event": event_type,
        "payload": payload,
    });
    if let Some(id) = task_id {
        item["task_id"] = Value::String(id);
    }
    let path = run_dir.join("events.jsonl");
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(&path) {
        let _ = writeln!(file, "{item}");
    }
}

fn now_iso() -> String {
    session::now_iso()
}

fn unix_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

// ---------------------------------------------------------------------------
// Task state persistence
// ---------------------------------------------------------------------------

fn task_state_path(run_dir: &Path, task_id: &str) -> PathBuf {
    run_dir.join("tasks").join(format!("{task_id}.json"))
}

fn save_task_state(run_dir: &Path, state: &TaskState) -> Result<()> {
    let value = serde_json::to_value(state).map_err(|e| e.to_string())?;
    write_json_value(&task_state_path(run_dir, &state.task_id), &value)
}

fn load_task_states(run_dir: &Path) -> Result<HashMap<String, TaskState>> {
    let tasks_dir = run_dir.join("tasks");
    let mut states = HashMap::new();
    if !tasks_dir.exists() {
        return Ok(states);
    }
    for entry in fs::read_dir(&tasks_dir).map_err(|e| format!("{}: {e}", tasks_dir.display()))? {
        let entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("json") {
            let text = fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
            if let Ok(state) = serde_json::from_str::<TaskState>(&text) {
                states.insert(state.task_id.clone(), state);
            }
        }
    }
    Ok(states)
}

fn init_states(
    run_dir: &Path,
    tasks: &[Task],
    plan: &Plan,
    force: bool,
    resume: bool,
) -> Result<HashMap<String, TaskState>> {
    let existed = run_dir.exists();
    if resume && !existed {
        return Err(format!("cannot resume missing run: {}", run_dir.display()));
    }
    if existed && !force && !resume {
        return Err(format!(
            "run already exists: {}; use --resume or --force",
            run_dir.display()
        ));
    }
    if force && run_dir.exists() {
        fs::remove_dir_all(run_dir).map_err(|e| format!("remove {}: {e}", run_dir.display()))?;
    }
    fs::create_dir_all(run_dir).map_err(|e| format!("{}: {e}", run_dir.display()))?;

    let mut states = load_task_states(run_dir)?;
    for task in tasks {
        let checkpoint_key = task_checkpoint_key(task, plan);
        let legacy_checkpoint_key = task_checkpoint_key_with_attempts(task, plan);
        let state = states.entry(task.id.clone()).or_insert_with(|| {
            let mut s = TaskState::new(&task.id, &task.source_id, &task.stage, &task.spec.route);
            s.effective_route = task.effective_route.clone();
            s.provider = task.provider.provider.clone();
            s.model = task.provider.model.clone();
            s.role = task.spec.role.clone();
            s.thinking = Some(task.spec.effective_thinking(plan));
            s.checkpoint_key = Some(checkpoint_key.clone());
            s
        });
        if state.effective_route.is_empty() {
            state.effective_route = task.effective_route.clone();
        }
        let checkpoint_matches = state.checkpoint_key.as_deref() == Some(&checkpoint_key)
            || state.checkpoint_key.as_deref() == Some(&legacy_checkpoint_key);
        if !state.status.is_completed() || !checkpoint_matches {
            state.status = TaskStatus::Pending;
            state.error = None;
            state.verified = None;
            state.verify_error = None;
            state.started_at = None;
            state.heartbeat_unix_ms = None;
            state.worker_log_bytes = 0;
            state.last_progress_unix_ms = None;
            state.worker_log_modified_unix_ms = None;
            state.ended_at = None;
        }
        state.checkpoint_key = Some(checkpoint_key);
    }
    Ok(states)
}

fn task_checkpoint_key(task: &Task, plan: &Plan) -> String {
    task_checkpoint_key_for_definition(task, plan, false)
}

/// Compatibility key for run state written before retry policy was separated
/// from a task's semantic definition. It is used only while resuming an older
/// run; subsequent state is rewritten with the current semantic key.
fn task_checkpoint_key_with_attempts(task: &Task, plan: &Plan) -> String {
    task_checkpoint_key_for_definition(task, plan, true)
}

fn task_checkpoint_key_for_definition(
    task: &Task,
    plan: &Plan,
    include_max_attempts: bool,
) -> String {
    let session = task.spec.effective_session(plan);
    let mut definition = json!({
        "source_id": task.source_id,
        "stage": task.stage,
        "stage_parallel": task.stage_parallel,
        "route": task.spec.route,
        "effective_route": task.effective_route,
        "provider": task.provider.provider,
        "model": task.provider.model,
        "wrapper": task.provider.wrapper,
        "role": task.spec.role,
        "task": task.spec.task,
        "needs": task.spec.needs,
        "tools_policy": task.spec.tools_policy,
        "artifacts": task.spec.artifacts,
        "verify": task.spec.verify,
        "thinking": task.spec.effective_thinking(plan),
        "session": session,
        "execution": plan.execution,
        "terminal": plan.terminal,
        "execution_timeout": "disabled",
    });
    if include_max_attempts {
        definition["max_attempts"] = json!(task.spec.effective_max_attempts(plan));
    }
    let definition = definition.to_string();
    let hash = fnv1a64(definition.as_bytes());
    format!("fnv1a64:{hash:016x}")
}

fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

fn resolve_project(plan: &Plan, workspace_root: &Path) -> (String, String) {
    if let Some(project) = &plan.project {
        let name = project
            .name
            .as_deref()
            .filter(|name| !name.trim().is_empty())
            .unwrap_or(&project.id);
        return (project.id.clone(), name.to_string());
    }
    let stable_path = workspace_root
        .canonicalize()
        .unwrap_or_else(|_| workspace_root.to_path_buf())
        .to_string_lossy()
        .to_lowercase();
    let name = workspace_root
        .file_name()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty())
        .unwrap_or("Workspace")
        .to_string();
    (
        format!("workspace:{:016x}", fnv1a64(stable_path.as_bytes())),
        name,
    )
}

#[allow(clippy::too_many_arguments)]
fn save_workflow(
    run_dir: &Path,
    workspace_root: &Path,
    run_id: &str,
    task_count: usize,
    global_cap: usize,
    caps: &HashMap<String, usize>,
    heartbeat_interval_seconds: u64,
    project_id: &str,
    project_name: &str,
    execution: &crate::model::ExecutionConfig,
    terminal: &crate::model::TerminalConfig,
) -> Result<()> {
    let wf = json!({
        "run_id": run_id,
        "runtime": "rust",
        "state_schema_version": 1,
        "created_at": now_iso(),
        "created_unix_ms": unix_ms(),
        "workspace_root": workspace_root,
        "project_id": project_id,
        "project_name": project_name,
        "heartbeat_interval_seconds": heartbeat_interval_seconds,
        "task_count": task_count,
        "global_max_concurrency": global_cap,
        "provider_max_concurrency": caps,
        "execution": execution,
        "terminal": terminal,
    });
    write_json_value(&run_dir.join("workflow.json"), &wf)
}

fn heartbeat_interval_seconds() -> u64 {
    std::env::var("SWARMS_HEARTBEAT_SECONDS")
        .ok()
        .and_then(|value| value.parse().ok())
        .filter(|value| *value > 0)
        .unwrap_or(30)
}

// ---------------------------------------------------------------------------
// Scheduler: find ready tasks
// ---------------------------------------------------------------------------

pub(crate) struct ReadyResult {
    pub(crate) selected: Vec<Task>,
    pub(crate) blocked: Vec<(String, String)>,
}

pub(crate) fn find_ready(
    tasks: &[Task],
    states: &HashMap<String, TaskState>,
    global_cap: usize,
    caps: &HashMap<String, usize>,
    plan: &Plan,
    router: &Router,
    quotas: &QuotaGuard,
) -> ReadyResult {
    let mut selected = Vec::new();
    let mut blocked = Vec::new();
    let mut active_by_route: HashMap<String, usize> = HashMap::new();
    let mut active_keys: HashSet<String> = HashSet::new();
    let mut serial_stages: HashSet<String> = HashSet::new();
    let mut active_count = 0usize;

    // A heartbeat re-enters this function while workers are still running.
    // Seed every occupancy guard from persisted non-terminal state so a
    // pending task cannot exceed global, route, serial-stage, or session-key
    // capacity just because the scheduler was re-entered.
    for task in tasks {
        let Some(state) = states.get(&task.id) else {
            continue;
        };
        if !matches!(state.status, TaskStatus::Queued | TaskStatus::InProgress) {
            continue;
        }
        active_count += 1;
        let route = if state.effective_route.is_empty() {
            task.effective_route.as_str()
        } else {
            state.effective_route.as_str()
        };
        *active_by_route.entry(route.to_string()).or_default() += 1;
        if !task.stage_parallel {
            serial_stages.insert(task.stage.clone());
        }
        let session = task.spec.effective_session(plan);
        if session.mode != SessionMode::Disabled {
            if let Some(key) = session.key {
                active_keys.insert(key);
            }
        }
    }

    for task in tasks {
        let state = states.get(&task.id);
        // Only pending tasks may be launched. In-progress tasks are not
        // terminal, but selecting them again on each heartbeat duplicates
        // workers, viewers, and provider usage until the host collapses.
        if state.is_some_and(|s| !matches!(s.status, TaskStatus::Pending)) {
            continue;
        }

        let mut deps_ok = true;
        let mut dep_failed = false;
        for dep in &task.spec.needs {
            match find_dependency_task(tasks, dep) {
                Some(dep_task) => match states.get(&dep_task.id) {
                    Some(s) if s.status.is_completed() => {}
                    Some(s) if s.status.is_failed() => {
                        dep_failed = true;
                        deps_ok = false;
                    }
                    _ => deps_ok = false,
                },
                None => deps_ok = false,
            }
        }

        if dep_failed {
            blocked.push((
                task.id.clone(),
                "dependency failed — blocking downstream task".to_string(),
            ));
            continue;
        }
        if !deps_ok {
            continue;
        }

        if active_count >= global_cap {
            continue;
        }
        if !task.stage_parallel && serial_stages.contains(&task.stage) {
            continue;
        }

        let mut candidates = vec![task.effective_route.as_str()];
        candidates.extend(task.provider.fallback_routes.iter().map(String::as_str));
        if let Some(fallback) = router.fallback_route.as_deref() {
            candidates.push(fallback);
        }
        let mut reasons = Vec::new();
        let mut capacity_wait = false;
        let mut chosen = None;
        let mut seen = HashSet::new();
        for candidate in candidates {
            let route = router.resolve_route(candidate);
            if !seen.insert(route) {
                continue;
            }
            let Some(provider) = router.providers.get(route) else {
                reasons.push(format!("route '{route}' is unknown"));
                continue;
            };
            if !provider.enabled {
                reasons.push(format!("route '{route}' is disabled"));
                continue;
            }
            if let Err(reason) = quotas.check(provider) {
                reasons.push(reason);
                continue;
            }
            let cap = caps.get(route).copied().unwrap_or(1);
            if cap == 0 {
                reasons.push(format!("route '{route}' has concurrency cap 0"));
                continue;
            }
            if active_by_route.get(route).copied().unwrap_or(0) >= cap {
                capacity_wait = true;
                continue;
            }
            chosen = Some((route.to_string(), provider.clone()));
            break;
        }
        let Some((route, provider)) = chosen else {
            if !capacity_wait {
                blocked.push((task.id.clone(), reasons.join("; ")));
            }
            continue;
        };

        let session = task.spec.effective_session(plan);
        if let Some(key) = &session.key {
            if session.mode != SessionMode::Disabled && !active_keys.insert(key.clone()) {
                continue;
            }
        }

        *active_by_route.entry(route.clone()).or_default() += 1;
        if !task.stage_parallel {
            serial_stages.insert(task.stage.clone());
        }
        active_count += 1;
        let mut selected_task = task.clone();
        selected_task.effective_route = route;
        selected_task.provider = provider;
        selected.push(selected_task);
    }

    ReadyResult { selected, blocked }
}

// ---------------------------------------------------------------------------
// Top-level execute
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
pub fn execute(
    root: &Path,
    workspace_root: &Path,
    tasks: &[Task],
    plan: &Plan,
    router: &Router,
    global_cap: usize,
    caps: &HashMap<String, usize>,
    run_id: &str,
    force: bool,
    resume: bool,
) -> Result<Report> {
    let run_dir = workspace_root.join(".agent/swarm/runs").join(run_id);
    let mut states = init_states(&run_dir, tasks, plan, force, resume)?;
    for state in states.values() {
        save_task_state(&run_dir, state)?;
    }
    let heartbeat_seconds = heartbeat_interval_seconds();
    let (project_id, project_name) = resolve_project(plan, workspace_root);
    if resume {
        let completed = states
            .values()
            .filter(|state| state.status.is_completed())
            .count();
        append_event(
            &run_dir,
            "workflow_resumed",
            json!({"task_count": tasks.len(), "completed": completed}),
        );
    } else {
        save_workflow(
            &run_dir,
            workspace_root,
            run_id,
            tasks.len(),
            global_cap,
            caps,
            heartbeat_seconds,
            &project_id,
            &project_name,
            &plan.execution,
            &plan.terminal,
        )?;
        append_event(
            &run_dir,
            "workflow_initialized",
            json!({"task_count": tasks.len()}),
        );
    }

    let session_store = Arc::new(SessionStore::open(&run_dir)?);
    let (sender, receiver) = mpsc::channel::<(String, TaskState)>();
    let mut active_ids: HashSet<String> = HashSet::new();
    let heartbeat_interval = Duration::from_secs(heartbeat_seconds);
    let mut last_heartbeat = Instant::now();

    // Continuous permit scheduling: launch work whenever capacity is free,
    // instead of waiting for a whole wave to drain. Each time a task finishes
    // we re-evaluate readiness and launch whatever newly fits, so a fast task's
    // freed permit is reused immediately while unrelated slow tasks continue.
    loop {
        let quotas = QuotaGuard::load(root, &router.quota_policy);
        let ready = find_ready(tasks, &states, global_cap, caps, plan, router, &quotas);

        for (id, msg) in &ready.blocked {
            if let Some(state) = states.get_mut(id) {
                state.status = TaskStatus::Blocked;
                state.error = Some(msg.clone());
                state.ended_at = Some(now_iso());
                let _ = save_task_state(&run_dir, state);
            }
            append_event(
                &run_dir,
                "task_blocked",
                json!({"task_id": id, "error": msg}),
            );
        }

        for task in &ready.selected {
            let prompt = build_task_prompt(&run_dir, workspace_root, task, tasks, &states);
            let work_dir = run_dir.join("results").join(&task.id);
            let _ = fs::create_dir_all(&work_dir);
            let _ = fs::write(work_dir.join("prompt.txt"), &prompt);
            let terminal = start_visible_worker_console(
                workspace_root,
                &run_dir,
                &work_dir,
                task,
                &plan.terminal,
            );

            let sender = sender.clone();
            let task = task.clone();
            let workspace_root = workspace_root.to_path_buf();
            let run_dir = run_dir.clone();
            let console_log = work_dir.join("worker.log");
            let plan = plan.clone();
            let router = router.clone();
            let caps = caps.clone();
            let store = Arc::clone(&session_store);

            if let Some(state) = states.get_mut(&task.id) {
                state.status = TaskStatus::InProgress;
                state.started_at = Some(now_iso());
                state.heartbeat_unix_ms = Some(unix_ms());
                state.last_progress_unix_ms = state.heartbeat_unix_ms;
                state.worker_log_bytes = 0;
                state.worker_log_modified_unix_ms = None;
                state.transport = Some(
                    match plan.execution.transport {
                        ExecutionTransport::Auto => "auto",
                        ExecutionTransport::Acp => "acp",
                        ExecutionTransport::CliBatch => "cli_batch",
                    }
                    .to_string(),
                );
                state.terminal_backend = terminal.as_ref().map(|value| value.backend.clone());
                state.terminal_session = terminal.as_ref().and_then(|value| value.session.clone());
                state.terminal_workspace_id = terminal
                    .as_ref()
                    .and_then(|value| value.workspace_id.clone());
                state.terminal_tab_id = terminal.as_ref().and_then(|value| value.tab_id.clone());
                state.terminal_pane_id = terminal.as_ref().and_then(|value| value.pane_id.clone());
                save_task_state(&run_dir, state)?;
            }
            active_ids.insert(task.id.clone());

            append_event(
                &run_dir,
                "task_started",
                json!({"task_id": task.id, "requested_route": task.spec.route, "effective_route": task.effective_route, "model": task.provider.model}),
            );

            thread::spawn(move || {
                let state = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    if task.spec.effective_scaling(&plan).mode != crate::model::ScalingMode::Single
                    {
                        crate::scaling::run_scaled_task(
                            &workspace_root,
                            &run_dir,
                            &task,
                            &plan,
                            &prompt,
                            &router,
                            global_cap,
                            &caps,
                        )
                    } else {
                        run_task(&workspace_root, &run_dir, &task, &plan, &prompt, &store)
                    }
                }))
                .unwrap_or_else(|_| {
                    failed_state(
                        &task,
                        task.spec.effective_thinking(&plan),
                        Instant::now(),
                        1,
                        "worker thread panicked",
                        &Usage::missing(),
                    )
                });
                signal_worker_console_finished(&console_log);
                let _ = sender.send((task.id.clone(), state));
            });
        }

        if active_ids.is_empty() {
            // Nothing running and nothing newly ready: the run is done.
            break;
        }

        let wait = heartbeat_interval.saturating_sub(last_heartbeat.elapsed());
        match receiver.recv_timeout(wait) {
            Ok((task_id, mut state)) => {
                active_ids.remove(&task_id);
                if let Some(previous) = states.get(&task_id) {
                    state.started_at.clone_from(&previous.started_at);
                    state.checkpoint_key.clone_from(&previous.checkpoint_key);
                    state
                        .terminal_backend
                        .clone_from(&previous.terminal_backend);
                    state
                        .terminal_session
                        .clone_from(&previous.terminal_session);
                    state
                        .terminal_workspace_id
                        .clone_from(&previous.terminal_workspace_id);
                    state.terminal_tab_id.clone_from(&previous.terminal_tab_id);
                    state
                        .terminal_pane_id
                        .clone_from(&previous.terminal_pane_id);
                }
                refresh_worker_progress(&run_dir, &mut state, unix_ms());
                states.insert(task_id.clone(), state.clone());
                save_task_state(&run_dir, &states[&task_id])?;
                append_event(
                    &run_dir,
                    "task_finished",
                    json!({"task_id": task_id, "status": format!("{:?}", state.status).to_lowercase()}),
                );
                // Loop back: find_ready runs again and launches whatever now
                // fits in the freed permit, without waiting for the rest.
                continue;
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                for task_id in &active_ids {
                    if let Some(state) = states.get_mut(task_id) {
                        state.status = TaskStatus::Failed;
                        state.error = Some("worker channel disconnected".to_string());
                        state.ended_at = Some(now_iso());
                        save_task_state(&run_dir, state)?;
                    }
                }
                break;
            }
        }

        if !active_ids.is_empty() && last_heartbeat.elapsed() >= heartbeat_interval {
            let heartbeat = unix_ms();
            for task_id in &active_ids {
                if let Some(state) = states.get_mut(task_id) {
                    state.heartbeat_unix_ms = Some(heartbeat);
                    refresh_worker_progress(&run_dir, state, heartbeat);
                    save_task_state(&run_dir, state)?;
                }
            }
            append_event(
                &run_dir,
                "tasks_heartbeat",
                json!({"task_ids": active_ids, "heartbeat_unix_ms": heartbeat}),
            );
            last_heartbeat = Instant::now();
        }
    }
    drop(sender);
    drop(receiver);

    let all_states: Vec<TaskState> = tasks
        .iter()
        .filter_map(|t| states.get(&t.id))
        .cloned()
        .collect();

    let report = telemetry::build_report(
        run_id,
        &run_dir.to_string_lossy(),
        &all_states,
        global_cap,
        caps,
        Vec::new(),
    );
    let report_value = serde_json::to_value(&report).map_err(|e| e.to_string())?;
    write_json_value(&run_dir.join("report.json"), &report_value)?;
    write_json_value(&run_dir.join("report-rs.json"), &report_value)?;
    append_event(
        &run_dir,
        "workflow_finished",
        json!({"status": report.status.clone()}),
    );
    close_herdr_workspaces(&all_states);

    Ok(report)
}

// ---------------------------------------------------------------------------
// Prompt generation
// ---------------------------------------------------------------------------

pub(crate) fn build_task_prompt(
    run_dir: &Path,
    workspace_root: &Path,
    task: &Task,
    all_tasks: &[Task],
    states: &HashMap<String, TaskState>,
) -> String {
    let dep_context = dependency_outputs(run_dir, task, all_tasks, states);
    let task_text = format!(
        "{}\n\nWORKSPACE BOUNDARY: {}. Work only within this directory. \
         Do not write or create artifacts outside it; allowed paths are relative to this workspace. \
         Provider configuration, credentials, and tool state outside this workspace are out of scope. \
         If a tool denies access to an external path, continue with repository contents instead of treating that denial as a blocker.",
        task.spec.task,
        workspace_root.display()
    );
    adapter::build_prompt(
        &task.spec.role,
        &task_text,
        &task.spec.artifacts,
        &dep_context,
    )
}

pub(crate) fn dependency_outputs(
    run_dir: &Path,
    task: &Task,
    all_tasks: &[Task],
    states: &HashMap<String, TaskState>,
) -> String {
    let mut sections = Vec::new();
    let mut remaining = MAX_DEP_CONTEXT_CHARS;

    for dep in &task.spec.needs {
        let dep_task = match find_dependency_task(all_tasks, dep) {
            Some(t) => t,
            None => continue,
        };
        let dep_state = states.get(&dep_task.id);
        match dep_state {
            Some(s) if s.status.is_completed() => {}
            _ => continue,
        };
        let log = run_dir
            .join("results")
            .join(&dep_task.id)
            .join("worker.log");
        if let Ok(content) = fs::read_to_string(&log) {
            if remaining == 0 {
                break;
            }
            let readable = readable_worker_output(&content);
            let mut start = readable.len().saturating_sub(remaining);
            while start < readable.len() && !readable.is_char_boundary(start) {
                start += 1;
            }
            let excerpt = &readable[start..];
            sections.push(format!("Dependency {} output:\n{excerpt}", dep_task.id));
            remaining = remaining.saturating_sub(excerpt.len());
        }
    }
    sections.join("\n\n")
}

/// Extracts human-authored worker messages from OpenCode/Codex JSONL logs.
///
/// Worker logs remain complete for Herd and audit. Dependency prompts instead
/// receive only text messages, avoiding tool payloads that can consume the
/// entire context window and slow down a dependent task. Plain-text adapters
/// retain their output with the viewer sentinel removed.
fn readable_worker_output(content: &str) -> String {
    let mut messages = Vec::new();
    for line in content.lines() {
        let Ok(item) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let text = item
            .pointer("/part/text")
            .and_then(Value::as_str)
            .or_else(|| item.pointer("/message/content").and_then(Value::as_str))
            .or_else(|| item.get("text").and_then(Value::as_str));
        if let Some(text) = text {
            let text = text.replace(WORKER_CONSOLE_FINISHED_SENTINEL, "");
            if !text.trim().is_empty() {
                messages.push(text.trim().to_string());
            }
        }
    }
    if !messages.is_empty() {
        return messages.join("\n\n");
    }

    content
        .lines()
        .filter(|line| line.trim() != WORKER_CONSOLE_FINISHED_SENTINEL)
        .collect::<Vec<_>>()
        .join("\n")
}

// ---------------------------------------------------------------------------
// Single task execution (with retries)
// ---------------------------------------------------------------------------

pub(crate) fn run_task(
    root: &Path,
    run_dir: &Path,
    task: &Task,
    plan: &Plan,
    prompt: &str,
    session_store: &SessionStore,
) -> TaskState {
    let thinking = task.spec.effective_thinking(plan);
    let max_attempts = task.spec.effective_max_attempts(plan).max(1);
    let work_dir = run_dir.join("results").join(&task.id);
    let started = Instant::now();

    let session_config = task.spec.effective_session(plan);
    let session_decision = session::decide(
        &session_config,
        session_store,
        &task.effective_route,
        &task.provider.model,
        &task.provider.wrapper,
        &root.to_string_lossy(),
    );

    let mut active_session_id = match &session_decision {
        Ok(SessionDecision::Reuse(id)) => Some(id.clone()),
        Ok(SessionDecision::Fail(msg)) => {
            return failed_state(task, thinking, started, 1, msg, &Usage::missing());
        }
        _ => None,
    };
    let session_reused = matches!(session_decision, Ok(SessionDecision::Reuse(_)));
    let adapter_kind =
        AdapterKind::from_wrapper(&task.provider.wrapper).unwrap_or(AdapterKind::Mock);
    let mut session_resume_count = u32::from(session_reused);

    // Snapshot artifact/protected mtimes before the worker runs, so the
    // completion gate can prove the worker actually produced each declared
    // artifact and did not modify a protected path.
    let artifact_snapshot = capture_artifact_snapshot(root, task);

    let mut attempt = 0_u32;

    let last_error = loop {
        attempt += 1;
        let exec_result = execute_adapter(
            task,
            prompt,
            thinking,
            active_session_id.as_deref(),
            root,
            run_dir,
            &work_dir,
            &plan.execution,
        );

        match exec_result {
            Ok(mut exec) => {
                let mut new_session_id = exec.session_id.clone().or_else(|| {
                    adapter::parse_session_id(
                        AdapterKind::from_wrapper(&task.provider.wrapper)
                            .unwrap_or(AdapterKind::Mock),
                        &exec.output,
                    )
                });
                if let Some(ref sid) = new_session_id {
                    if let Some(key) = &session_config.key {
                        let _ = session_store.put(session::SessionEntry {
                            key: key.clone(),
                            provider_session_id: sid.clone(),
                            route: task.effective_route.clone(),
                            model: task.provider.model.clone(),
                            adapter: task.provider.wrapper.clone(),
                            workspace: root.to_string_lossy().to_string(),
                            created_at: session::now_iso(),
                            reused_count: 0,
                        });
                    }
                }

                loop {
                    let messages = match steering::drain(run_dir, &task.id) {
                        Ok(messages) => messages,
                        Err(error) => {
                            return failed_state(
                                task,
                                thinking,
                                started,
                                attempt,
                                &error,
                                &exec.usage,
                            );
                        }
                    };
                    if messages.is_empty() {
                        break;
                    }
                    for message in messages {
                        let kind = AdapterKind::from_wrapper(&task.provider.wrapper)
                            .unwrap_or(AdapterKind::Mock);
                        if kind != AdapterKind::Mock
                            && (!kind.supports_session_reuse() || new_session_id.is_none())
                        {
                            let command_id = message.id.clone();
                            let _ = steering::mark_applied(
                                run_dir,
                                &task.id,
                                &AppliedSteer {
                                    message,
                                    status: "rejected".to_string(),
                                    error: Some(
                                        "adapter did not expose a resumable session".to_string(),
                                    ),
                                },
                            );
                            append_event(
                                run_dir,
                                "steer_rejected",
                                json!({"task_id": task.id, "command_id": command_id}),
                            );
                            continue;
                        }
                        let steer_prompt = format!(
                            "{prompt}\n\nUSER STEER PROMPT ({})\n{}\n\nApply this direction before finishing the task. This provider accepted it at a safe session boundary; do not claim an in-flight tool call was interrupted.",
                            message.mode.as_str(),
                            message.prompt
                        );
                        let previous_log =
                            fs::read_to_string(work_dir.join("worker.log")).unwrap_or_default();
                        let steered = execute_adapter(
                            task,
                            &steer_prompt,
                            thinking,
                            new_session_id.as_deref().or(active_session_id.as_deref()),
                            root,
                            run_dir,
                            &work_dir,
                            &plan.execution,
                        );
                        match steered {
                            Ok(next) => {
                                let command_id = message.id.clone();
                                let mode = message.mode;
                                let next_session_id = next.session_id.clone().or_else(|| {
                                    adapter::parse_session_id(
                                        AdapterKind::from_wrapper(&task.provider.wrapper)
                                            .unwrap_or(AdapterKind::Mock),
                                        &next.output,
                                    )
                                });
                                if next_session_id.is_some() {
                                    new_session_id = next_session_id;
                                }
                                preserve_steering_log(&work_dir, &previous_log, &message.prompt);
                                merge_usage(&mut exec.usage, &next.usage);
                                exec.output = next.output;
                                exec.transport = next.transport;
                                exec.session_id = next.session_id;
                                let _ = steering::mark_applied(
                                    run_dir,
                                    &task.id,
                                    &AppliedSteer {
                                        message,
                                        status: "applied".to_string(),
                                        error: None,
                                    },
                                );
                                append_event(
                                    run_dir,
                                    "steer_applied",
                                    json!({"task_id": task.id, "command_id": command_id, "mode": mode.as_str()}),
                                );
                            }
                            Err(error) => {
                                let command_id = message.id.clone();
                                preserve_steering_log(&work_dir, &previous_log, &message.prompt);
                                let _ = steering::mark_applied(
                                    run_dir,
                                    &task.id,
                                    &AppliedSteer {
                                        message,
                                        status: "failed".to_string(),
                                        error: Some(error.clone()),
                                    },
                                );
                                append_event(
                                    run_dir,
                                    "steer_failed",
                                    json!({"task_id": task.id, "command_id": command_id}),
                                );
                                return failed_state(
                                    task,
                                    thinking,
                                    started,
                                    attempt,
                                    &format!("steer prompt failed: {error}"),
                                    &exec.usage,
                                );
                            }
                        }
                    }
                }

                if let Err(e) = check_artifacts_with_snapshot(root, task, Some(&artifact_snapshot))
                {
                    let mut state = failed_state(task, thinking, started, attempt, &e, &exec.usage);
                    attach_session_context(
                        &mut state,
                        session_reused || session_resume_count > 0,
                        new_session_id.clone().or_else(|| active_session_id.clone()),
                        session_resume_count,
                        Some(exec.transport.clone()),
                    );
                    return state;
                }

                let (verified, verify_error) = run_verify_commands(task, root, &work_dir);

                if verified == Some(false) {
                    let err = verify_error
                        .as_deref()
                        .unwrap_or("verification command failed");
                    let mut state =
                        failed_state(task, thinking, started, attempt, err, &exec.usage);
                    attach_session_context(
                        &mut state,
                        session_reused || session_resume_count > 0,
                        new_session_id.clone().or_else(|| active_session_id.clone()),
                        session_resume_count,
                        Some(exec.transport.clone()),
                    );
                    state.verified = Some(false);
                    state.verify_error = verify_error;
                    return state;
                }

                // Feature-producing roles must reach `Completed` with successful
                // verification evidence. `verified == None` means no verify
                // command ran (static review should have caught this, but the
                // coordinator enforces the invariant independently so that a
                // plan loaded with `--force` or an edited plan cannot bypass it).
                if task.spec.requires_verification() && verified != Some(true) {
                    let mut state = failed_state(
                        task,
                        thinking,
                        started,
                        attempt,
                        "role requires verification but none passed",
                        &exec.usage,
                    );
                    attach_session_context(
                        &mut state,
                        session_reused || session_resume_count > 0,
                        new_session_id.clone().or_else(|| active_session_id.clone()),
                        session_resume_count,
                        Some(exec.transport.clone()),
                    );
                    state.verified = verified;
                    state.verify_error = verify_error;
                    return state;
                }

                return success_state(
                    task,
                    thinking,
                    started,
                    attempt,
                    session_reused || session_resume_count > 0,
                    new_session_id.or_else(|| active_session_id.clone()),
                    session_resume_count,
                    verified,
                    verify_error,
                    &exec.usage,
                    Some(exec.transport),
                );
            }
            Err(e) => {
                let recovered =
                    if session_resume_count == 0 && adapter_kind.supports_session_reuse() {
                        fresh_log_session_id(
                            adapter_kind,
                            &work_dir.join("worker.log"),
                            session_resume_window(),
                        )
                    } else {
                        None
                    };
                let recovered_retry = recovered.is_some();
                if let Some(session_id) = recovered {
                    active_session_id = Some(session_id.clone());
                    session_resume_count = 1;
                    if let Some(key) = &session_config.key {
                        let _ = session_store.put(session::SessionEntry {
                            key: key.clone(),
                            provider_session_id: session_id,
                            route: task.effective_route.clone(),
                            model: task.provider.model.clone(),
                            adapter: task.provider.wrapper.clone(),
                            workspace: root.to_string_lossy().to_string(),
                            created_at: session::now_iso(),
                            reused_count: 1,
                        });
                    }
                    append_event(
                        run_dir,
                        "provider_session_resume_started",
                        json!({"task_id": task.id, "attempt": attempt + 1}),
                    );
                }
                if attempt < max_attempts || recovered_retry {
                    let delay = retry_delay(&e, attempt);
                    append_event(
                        run_dir,
                        "task_retry_scheduled",
                        json!({
                            "task_id": task.id,
                            "attempt": attempt + 1,
                            "delay_ms": delay.as_millis(),
                            "reason": retry_reason(&e),
                        }),
                    );
                    thread::sleep(delay);
                    continue;
                }
                break e;
            }
        }
    };
    let mut state = failed_state(
        task,
        thinking,
        started,
        attempt,
        &last_error,
        &Usage::missing(),
    );
    state.session_reused = session_resume_count > 0;
    state.session_resume_count = session_resume_count;
    state.session_id = active_session_id;
    state
}

/// Returns a bounded retry delay without imposing an execution timeout.
///
/// OpenCode persists session state in one local SQLite database. Its CLI can
/// briefly reject a concurrent startup with `database is locked`; waiting a few
/// seconds lets the existing writer finish while preserving the configured
/// worker concurrency for actual agent work. All other adapter failures retain
/// the short exponential retry already used by the coordinator.
fn retry_delay(error: &str, attempt: u32) -> Duration {
    if is_transient_opencode_database_lock(error) {
        let seconds = 5u64 << (attempt.saturating_sub(1).min(3));
        return Duration::from_secs(seconds.min(40));
    }

    let millis = 100u64 << (attempt.saturating_sub(1).min(5));
    Duration::from_millis(millis.min(5000))
}

fn retry_reason(error: &str) -> &'static str {
    if is_transient_opencode_database_lock(error) {
        "transient_opencode_database_lock"
    } else {
        "adapter_error"
    }
}

fn is_transient_opencode_database_lock(error: &str) -> bool {
    error.to_ascii_lowercase().contains("database is locked")
}

fn session_resume_window() -> Duration {
    Duration::from_secs(
        std::env::var("SWARMS_SESSION_RESUME_WINDOW_SECONDS")
            .ok()
            .and_then(|value| value.parse().ok())
            .filter(|seconds: &u64| *seconds > 0)
            .unwrap_or(300),
    )
}

fn refresh_worker_progress(run_dir: &Path, state: &mut TaskState, observed_at: u128) {
    let path = run_dir
        .join("results")
        .join(&state.task_id)
        .join("worker.log");
    let Ok(metadata) = fs::metadata(path) else {
        return;
    };
    let modified = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_millis());
    let changed = metadata.len() != state.worker_log_bytes
        || modified
            .zip(state.worker_log_modified_unix_ms)
            .is_some_and(|(current, previous)| current > previous);
    state.worker_log_bytes = metadata.len();
    state.worker_log_modified_unix_ms = modified;
    if changed {
        state.last_progress_unix_ms = Some(observed_at);
    }
}

fn fresh_log_session_id(kind: AdapterKind, log_path: &Path, window: Duration) -> Option<String> {
    let modified = fs::metadata(log_path).ok()?.modified().ok()?;
    let output = fs::read_to_string(log_path).ok()?;
    session_id_if_fresh(kind, &output, modified, SystemTime::now(), window)
}

fn session_id_if_fresh(
    kind: AdapterKind,
    output: &str,
    updated: SystemTime,
    now: SystemTime,
    window: Duration,
) -> Option<String> {
    let age = now.duration_since(updated).ok()?;
    (age <= window)
        .then(|| adapter::parse_session_id(kind, output))
        .flatten()
}

fn preserve_steering_log(work_dir: &Path, previous: &str, steer_prompt: &str) {
    let path = work_dir.join("worker.log");
    let current = fs::read_to_string(&path).unwrap_or_default();
    let separator = format!(
        "\n\n--- user steer ({}) ---\n",
        steer_prompt.chars().take(120).collect::<String>()
    );
    let _ = fs::write(path, format!("{previous}{separator}{current}"));
}

pub(crate) fn merge_usage(total: &mut Usage, next: &Usage) {
    fn add(left: &str, right: &str) -> String {
        match (left.parse::<u64>(), right.parse::<u64>()) {
            (Ok(left), Ok(right)) => left.saturating_add(right).to_string(),
            _ => "missing".to_string(),
        }
    }
    total.input = add(&total.input, &next.input);
    total.cache_read = add(&total.cache_read, &next.cache_read);
    total.cache_write = add(&total.cache_write, &next.cache_write);
    total.output = add(&total.output, &next.output);
    total.reasoning = add(&total.reasoning, &next.reasoning);
}

pub(crate) struct AdapterExec {
    pub(crate) output: String,
    pub(crate) usage: Usage,
    pub(crate) session_id: Option<String>,
    pub(crate) transport: String,
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn execute_adapter(
    task: &Task,
    prompt: &str,
    thinking: ThinkingLevel,
    session_id: Option<&str>,
    root: &Path,
    run_dir: &Path,
    work_dir: &Path,
    execution: &crate::model::ExecutionConfig,
) -> Result<AdapterExec> {
    let kind = AdapterKind::from_wrapper(&task.provider.wrapper)
        .ok_or_else(|| format!("unsupported wrapper: {}", task.provider.wrapper))?;

    if matches!(execution.transport, ExecutionTransport::Auto) {
        let log_path = work_dir.join("worker.log");
        if kind == AdapterKind::Codex && codex_app_server::enabled() {
            let result = codex_app_server::run(
                task, prompt, thinking, session_id, root, &log_path, run_dir,
            )?;
            return Ok(AdapterExec {
                output: result.output,
                usage: Usage::missing(),
                session_id: result.session_id,
                transport: "codex_app_server".to_string(),
            });
        }
        if kind == AdapterKind::OpenCode && opencode_server::enabled() {
            let result =
                opencode_server::run(task, prompt, thinking, session_id, root, &log_path, run_dir)?;
            return Ok(AdapterExec {
                output: result.output,
                usage: Usage::missing(),
                session_id: result.session_id,
                transport: "opencode_server".to_string(),
            });
        }
        if kind == AdapterKind::Claude && claude_stream::enabled() {
            let result =
                claude_stream::run(task, prompt, thinking, session_id, root, &log_path, run_dir)?;
            let output = result.output;
            return Ok(AdapterExec {
                usage: adapter::parse_cli_usage(kind, &output),
                output,
                session_id: result.session_id,
                transport: "claude_stream".to_string(),
            });
        }
    }

    let acp_spec = adapter::build_acp_command(kind, &execution.acp);
    let use_acp =
        !matches!(execution.transport, ExecutionTransport::CliBatch) && acp_spec.is_some();
    if use_acp {
        let spec = acp_spec.expect("checked above");
        match execute_acp(
            task,
            prompt,
            session_id,
            root,
            run_dir,
            work_dir,
            &execution.acp,
            &spec,
        ) {
            Ok(result) => return Ok(result),
            Err(failure)
                if failure.safe_fallback
                    && matches!(execution.fallback, ExecutionFallback::CliBatch) =>
            {
                append_event(
                    run_dir,
                    "acp_fallback_to_cli",
                    json!({"task_id": task.id, "error": failure.message}),
                );
            }
            Err(failure) => return Err(failure.message),
        }
    } else if matches!(execution.transport, ExecutionTransport::Acp) {
        return Err(format!(
            "route '{}' requested ACP but wrapper '{}' has no ACP command",
            task.spec.route, task.provider.wrapper
        ));
    }

    match kind {
        AdapterKind::Mock => {
            let out = adapter::execute_mock(root, prompt)?;
            let _ = fs::write(work_dir.join("worker.log"), &out.stdout);
            Ok(AdapterExec {
                output: out.stdout,
                usage: Usage::offline_mock(),
                session_id: None,
                transport: "mock".to_string(),
            })
        }
        AdapterKind::OpenAiCompat => {
            let out = adapter::execute_openai_compat(task, prompt, thinking)?;
            let _ = fs::write(work_dir.join("worker.log"), &out.content);
            Ok(AdapterExec {
                output: out.content,
                usage: out.usage,
                session_id: None,
                transport: "http".to_string(),
            })
        }
        AdapterKind::ChatGptChat => {
            let out = adapter::execute_chatgpt_chat(task, prompt, session_id, root)?;
            let _ = fs::write(work_dir.join("worker.log"), &out.content);
            Ok(AdapterExec {
                output: out.content,
                usage: Usage::missing(),
                session_id: Some(out.worker_id),
                transport: "chatgpt_chat_broker".to_string(),
            })
        }
        _ => {
            let spec = adapter::build_cli_command(
                kind,
                task,
                prompt,
                thinking,
                session_id,
                &task.provider.provider,
            )?;
            let log_path = work_dir.join("worker.log");
            let output = execute_cli(kind, spec, root, &log_path)?;
            let usage = adapter::parse_cli_usage(kind, &output);
            Ok(AdapterExec {
                output,
                usage,
                session_id: None,
                transport: "cli_batch".to_string(),
            })
        }
    }
}

struct AcpFailure {
    message: String,
    safe_fallback: bool,
}

impl AcpFailure {
    fn safe(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            safe_fallback: true,
        }
    }

    fn unsafe_after_prompt(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            safe_fallback: false,
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn execute_acp(
    task: &Task,
    prompt: &str,
    existing_session: Option<&str>,
    root: &Path,
    run_dir: &Path,
    work_dir: &Path,
    config: &crate::model::AcpConfig,
    spec: &CliSpec,
) -> std::result::Result<AdapterExec, AcpFailure> {
    let log_path = work_dir.join("worker.log");
    let _ = fs::File::create(&log_path);
    let startup = Duration::from_secs(config.startup_timeout_seconds);
    let cancel_grace = Duration::from_secs(config.cancel_grace_seconds);
    let mut client = acp::Client::launch(
        &spec.program,
        &spec.args,
        root,
        &log_path,
        startup,
        cancel_grace,
    )
    .map_err(AcpFailure::safe)?;
    let session_id = client
        .open_session(root, existing_session, startup)
        .map_err(AcpFailure::safe)?;
    append_event(
        run_dir,
        "acp_session_opened",
        json!({"task_id": task.id, "session_id": session_id, "reused": existing_session.is_some()}),
    );
    let mut request_id = client.start_prompt(prompt).map_err(AcpFailure::safe)?;
    let mut output = String::new();
    let mut pending_steers = Vec::new();
    let mut cancel_sent = false;
    let mut cancel_deadline = None;
    let mut continuation_started = false;

    loop {
        if cancel_sent && cancel_deadline.is_some_and(|deadline| Instant::now() > deadline) {
            return Err(AcpFailure::unsafe_after_prompt(
                "ACP agent did not acknowledge cancellation within the configured grace period",
            ));
        }
        if let Some(event) = client
            .next_event(Duration::from_millis(250))
            .map_err(AcpFailure::unsafe_after_prompt)?
        {
            match event {
                Event::Update(params) => {
                    append_acp_log(&log_path, &params);
                    if let Some(text) = acp::update_text(&params) {
                        output.push_str(text);
                    }
                }
                Event::Notification { method, params } => {
                    append_acp_log(
                        &log_path,
                        &json!({"notification": method, "params": params}),
                    );
                }
                Event::Response { id, result, error } if id == request_id => {
                    if let Some(error) = error {
                        if cancel_sent && !pending_steers.is_empty() {
                            return Err(AcpFailure::unsafe_after_prompt(format!(
                                "ACP cancelled prompt failed: {error}"
                            )));
                        }
                        return Err(AcpFailure::unsafe_after_prompt(format!(
                            "ACP prompt failed: {error}"
                        )));
                    }
                    if !pending_steers.is_empty() && !continuation_started {
                        let continued = steering_continuation(prompt, &pending_steers, cancel_sent);
                        request_id = client
                            .start_prompt(&continued)
                            .map_err(AcpFailure::unsafe_after_prompt)?;
                        cancel_sent = false;
                        cancel_deadline = None;
                        continuation_started = true;
                        continue;
                    }
                    for message in pending_steers.drain(..) {
                        let _ = steering::mark_applied(
                            run_dir,
                            &task.id,
                            &AppliedSteer {
                                message,
                                status: "applied".to_string(),
                                error: None,
                            },
                        );
                    }
                    let response = result.unwrap_or(Value::Null);
                    let stop = acp::stop_reason(&response).unwrap_or("unknown");
                    append_event(
                        run_dir,
                        "acp_prompt_finished",
                        json!({"task_id": task.id, "stop_reason": stop}),
                    );
                    let final_output = if output.is_empty() {
                        fs::read_to_string(&log_path).unwrap_or_default()
                    } else {
                        output
                    };
                    return Ok(AdapterExec {
                        output: final_output,
                        usage: adapter::parse_acp_usage(
                            &fs::read_to_string(&log_path).unwrap_or_default(),
                        ),
                        session_id: Some(session_id),
                        transport: "acp".to_string(),
                    });
                }
                Event::Response { .. } => {}
            }
        }

        if !cancel_sent && !continuation_started {
            let messages =
                steering::drain(run_dir, &task.id).map_err(AcpFailure::unsafe_after_prompt)?;
            if !messages.is_empty() {
                let mut should_cancel = false;
                for message in messages {
                    append_acp_log(
                        &log_path,
                        &json!({"type": "swarms_steer", "mode": message.mode.as_str(), "prompt": message.prompt}),
                    );
                    should_cancel |= message.mode != SteeringMode::Enqueue;
                    pending_steers.push(message);
                }
                if should_cancel {
                    client.cancel().map_err(AcpFailure::unsafe_after_prompt)?;
                    cancel_sent = true;
                    cancel_deadline = Some(Instant::now() + client.cancel_grace());
                }
            }
        }
    }
}

fn append_acp_log(path: &Path, value: &Value) {
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(file, "{value}");
    }
}

fn steering_continuation(
    prompt: &str,
    messages: &[steering::SteerMessage],
    cancelled: bool,
) -> String {
    let steer = messages
        .iter()
        .map(|message| message.prompt.as_str())
        .collect::<Vec<_>>()
        .join("\n\n");
    let modes = messages
        .iter()
        .map(|message| message.mode.as_str())
        .collect::<Vec<_>>()
        .join(",");
    if cancelled {
        format!(
            "{prompt}\n\nUSER STEER PROMPT ({modes})\n{steer}\n\nRestart from the persisted task prompt and apply this direction before finishing."
        )
    } else {
        format!(
            "USER STEER PROMPT ({modes})\n{steer}\n\nThe previous turn completed. Apply this queued direction before finalizing the task."
        )
    }
}

fn execute_cli(kind: AdapterKind, spec: CliSpec, cwd: &Path, log_path: &Path) -> Result<String> {
    if let Some(parent) = log_path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let log = fs::File::create(log_path).map_err(|e| format!("{}: {e}", log_path.display()))?;
    let err = log.try_clone().map_err(|e| e.to_string())?;

    let mut cmd = Command::new(&spec.program);
    cmd.args(&spec.args)
        .current_dir(cwd)
        .stdout(Stdio::from(log))
        .stderr(Stdio::from(err));
    for (key, val) in &spec.env {
        cmd.env(key, val);
    }

    let mut child = cmd
        .spawn()
        .map_err(|e| format!("spawn '{}': {e}", spec.program))?;
    let mut terminal_event_seen_at = None;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let output = fs::read_to_string(log_path).unwrap_or_default();
                if status.success() {
                    return Ok(output);
                }
                let tail = tail_chars(&output, 2000);
                return Err(format!(
                    "process '{}' exited {:?}: {}",
                    spec.program,
                    status.code(),
                    tail
                ));
            }
            Ok(None) => {
                if (matches!(kind, AdapterKind::OpenCode | AdapterKind::Kilo)
                    && opencode_terminal_event_seen(log_path))
                    || (kind == AdapterKind::Codex && codex_terminal_event_seen(log_path))
                {
                    let seen_at = terminal_event_seen_at.get_or_insert_with(Instant::now);
                    if seen_at.elapsed() >= Duration::from_secs(3) {
                        // Prefer the real exit status if the wrapper finished
                        // during the grace period. Only reap a leaked wrapper
                        // after the provider's explicit terminal event remains
                        // the sole completion signal.
                        match child.try_wait() {
                            Ok(Some(status)) if status.success() => {
                                return Ok(fs::read_to_string(log_path).unwrap_or_default());
                            }
                            Ok(Some(status)) => {
                                let output = fs::read_to_string(log_path).unwrap_or_default();
                                return Err(format!(
                                    "process '{}' exited {:?}: {}",
                                    spec.program,
                                    status.code(),
                                    tail_chars(&output, 2000)
                                ));
                            }
                            Ok(None) => {
                                let _ = child.kill();
                                let _ = child.wait();
                                return Ok(fs::read_to_string(log_path).unwrap_or_default());
                            }
                            Err(error) => return Err(format!("wait '{}': {error}", spec.program)),
                        }
                    }
                } else {
                    terminal_event_seen_at = None;
                }
                thread::sleep(Duration::from_millis(50));
            }
            Err(e) => return Err(format!("wait '{}': {e}", spec.program)),
        }
    }
}

/// OpenCode's JSONL protocol ends a completed turn with `step_finish/stop`.
/// This is a completion signal, not an elapsed-time heuristic.
fn opencode_terminal_event_seen(log_path: &Path) -> bool {
    let Ok(content) = fs::read_to_string(log_path) else {
        return false;
    };
    content.lines().any(|line| {
        let Ok(item) = serde_json::from_str::<Value>(line) else {
            return false;
        };
        item.get("type").and_then(Value::as_str) == Some("step_finish")
            && item.pointer("/part/reason").and_then(Value::as_str) == Some("stop")
    })
}

/// Signals the read-only viewer that the worker has finished.
fn signal_worker_console_finished(log_path: &Path) {
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(log_path) {
        let _ = writeln!(file, "{WORKER_CONSOLE_FINISHED_SENTINEL}");
    }
}

/// Opens a read-only console for each real Windows worker. The coordinator and
/// worker remain in the background; the window shows context, tails
/// `worker.log`, and closes after the completion signal.
#[cfg(windows)]
fn start_visible_worker_console(
    workspace_root: &Path,
    run_dir: &Path,
    work_dir: &Path,
    task: &Task,
    terminal: &crate::model::TerminalConfig,
) -> Option<WorkerTerminal> {
    use std::os::windows::process::CommandExt;

    let backend = worker_console_backend(terminal);
    let use_herd = backend == "herdr";
    let legacy_console_hidden = std::env::var("SWARMS_WORKER_CONSOLES")
        .map(|value| value.eq_ignore_ascii_case("hidden") || value == "0")
        .unwrap_or(false);
    if backend == "hidden"
        || !should_start_worker_terminal(
            AdapterKind::from_wrapper(&task.provider.wrapper),
            use_herd,
            legacy_console_hidden,
        )
    {
        return None;
    }
    let log_path = work_dir.join("worker.log");
    if fs::File::create(&log_path).is_err() {
        return None;
    }
    let path = log_path.to_string_lossy().replace('\'', "''");
    let prompt_path = work_dir
        .join("prompt.txt")
        .to_string_lossy()
        .replace('\'', "''");
    let title = format!("SWARMS | {} | {}", task.id, task.provider.model).replace('\'', "''");
    let pane_label = herdr_pane_label(task);
    if use_herd {
        let result = start_herdr_worker_pane(
            workspace_root,
            run_dir,
            work_dir,
            terminal.workspace_scope,
            &task.stage,
            &pane_label,
            &title,
            &path,
            &prompt_path,
        );
        if result.is_some() || terminal.on_unavailable != crate::model::TerminalUnavailable::Native
        {
            return result;
        }
    }
    let script = worker_console_script(&title, &path, &prompt_path);
    const CREATE_NEW_CONSOLE: u32 = 0x0000_0010;
    let _ = Command::new("powershell")
        .args(["-NoLogo", "-NoProfile", "-Command", &script])
        .creation_flags(CREATE_NEW_CONSOLE)
        .spawn();
    Some(WorkerTerminal {
        backend: "windows_console".to_string(),
        session: None,
        workspace_id: None,
        tab_id: None,
        pane_id: None,
    })
}

#[cfg(windows)]
fn should_start_worker_terminal(
    adapter: Option<AdapterKind>,
    use_herd: bool,
    legacy_console_hidden: bool,
) -> bool {
    use_herd || (adapter != Some(AdapterKind::Mock) && !legacy_console_hidden)
}

/// Herd is opt-in while its native Windows support remains beta. The worker
/// process stays under SWARMS, preserving quotas, raw logs, retries and resume.
#[cfg(windows)]
fn worker_console_backend(terminal: &crate::model::TerminalConfig) -> String {
    std::env::var("SWARMS_TERMINAL_BACKEND")
        .ok()
        .map(|value| value.to_ascii_lowercase())
        .unwrap_or_else(|| match terminal.backend {
            crate::model::TerminalBackend::Herdr => "herdr".to_string(),
            crate::model::TerminalBackend::Hidden => "hidden".to_string(),
            crate::model::TerminalBackend::Native => "native".to_string(),
        })
}

#[cfg(windows)]
fn herdr_program() -> String {
    if let Ok(path) = std::env::var("SWARMS_HERDR_BIN") {
        return path;
    }
    if let Ok(local) = std::env::var("LOCALAPPDATA") {
        let candidate = Path::new(&local)
            .join("Programs")
            .join("Herdr")
            .join("bin")
            .join("herdr.exe");
        if candidate.is_file() {
            return candidate.to_string_lossy().to_string();
        }
    }
    "herdr".to_string()
}

#[cfg(windows)]
fn herdr_session() -> String {
    std::env::var("SWARMS_HERDR_SESSION").unwrap_or_else(|_| "swarms".to_string())
}

#[cfg(windows)]
#[derive(Clone, Debug)]
struct HerdrRunWorkspace {
    session: String,
    workspace_id: String,
    root_tab_id: String,
    root_pane_id: String,
    tabs: HashMap<String, HerdrRunTab>,
}

#[cfg(windows)]
#[derive(Clone, Debug)]
struct HerdrRunTab {
    tab_id: String,
    root_pane_id: String,
    worker_count: usize,
}

#[cfg(windows)]
#[allow(clippy::too_many_arguments)]
fn start_herdr_worker_pane(
    workspace_root: &Path,
    run_dir: &Path,
    work_dir: &Path,
    scope: crate::model::TerminalWorkspaceScope,
    stage: &str,
    pane_label: &str,
    title: &str,
    log_path: &str,
    prompt_path: &str,
) -> Option<WorkerTerminal> {
    static HERDR_START_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    static HERDR_WORKSPACES: OnceLock<Mutex<HashMap<String, HerdrRunWorkspace>>> = OnceLock::new();
    let herdr = herdr_program();
    let session = herdr_session();
    let workspace_cwd = if scope == crate::model::TerminalWorkspaceScope::Worker {
        work_dir
    } else {
        workspace_root
    };
    let root_cwd = workspace_cwd.canonicalize().ok()?;
    let run_key = herdr_workspace_key(run_dir)?;
    let _lock = HERDR_START_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .ok()?;
    let workspaces = HERDR_WORKSPACES.get_or_init(|| Mutex::new(HashMap::new()));
    let mut registry = workspaces.lock().ok()?;
    let mut workspace = if let Some(existing) = registry.get(&run_key) {
        existing.clone()
    } else {
        if !herdr_server_is_running(&herdr, &session) {
            let launch = format!(
                "Start-Process -FilePath '{}' -ArgumentList @('--session','{}','server') -WindowStyle Hidden",
                herdr.replace('\'', "''"),
                session.replace('\'', "''")
            );
            Command::new("powershell")
                .args(["-NoLogo", "-NoProfile", "-Command", &launch])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .ok()?;
            for _ in 0..10 {
                thread::sleep(Duration::from_millis(100));
                if herdr_server_is_running(&herdr, &session) {
                    break;
                }
            }
        }
        if !herdr_server_is_running(&herdr, &session) {
            return None;
        }
        let run_id = run_dir
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("run");
        let label = format!("SWARMS | {run_id}");
        let output = Command::new(&herdr)
            .args([
                "--session",
                &session,
                "workspace",
                "create",
                "--cwd",
                &root_cwd.to_string_lossy(),
                "--label",
                &label,
                "--no-focus",
            ])
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        let result = serde_json::from_slice::<Value>(&output.stdout).ok()?;
        let created = HerdrRunWorkspace {
            session: session.clone(),
            workspace_id: result
                .pointer("/result/workspace/workspace_id")?
                .as_str()?
                .to_string(),
            root_tab_id: result
                .pointer("/result/tab/tab_id")
                .or_else(|| result.pointer("/result/workspace/tab_id"))?
                .as_str()?
                .to_string(),
            root_pane_id: result
                .pointer("/result/root_pane/pane_id")?
                .as_str()?
                .to_string(),
            tabs: HashMap::new(),
        };
        registry.insert(run_key.clone(), created.clone());
        created
    };

    let stage_key = stage.to_string();
    let stage_label = herdr_label(&format!("Phase | {stage}"), 80);
    let tab = if let Some(existing) = workspace.tabs.get(&stage_key) {
        existing.clone()
    } else {
        let created = if workspace.tabs.is_empty() {
            let _ = Command::new(&herdr)
                .args([
                    "--session",
                    &workspace.session,
                    "tab",
                    "rename",
                    &workspace.root_tab_id,
                    &stage_label,
                ])
                .status();
            HerdrRunTab {
                tab_id: workspace.root_tab_id.clone(),
                root_pane_id: workspace.root_pane_id.clone(),
                worker_count: 0,
            }
        } else {
            let output = Command::new(&herdr)
                .args([
                    "--session",
                    &workspace.session,
                    "tab",
                    "create",
                    "--workspace",
                    &workspace.workspace_id,
                    "--cwd",
                    &root_cwd.to_string_lossy(),
                    "--label",
                    &stage_label,
                    "--no-focus",
                ])
                .output()
                .ok()?;
            if !output.status.success() {
                return None;
            }
            let result = serde_json::from_slice::<Value>(&output.stdout).ok()?;
            HerdrRunTab {
                tab_id: result.pointer("/result/tab/tab_id")?.as_str()?.to_string(),
                root_pane_id: result
                    .pointer("/result/root_pane/pane_id")?
                    .as_str()?
                    .to_string(),
                worker_count: 0,
            }
        };
        workspace.tabs.insert(stage_key.clone(), created.clone());
        created
    };

    let pane_id = if tab.worker_count == 0 {
        tab.root_pane_id.clone()
    } else {
        let output = Command::new(&herdr)
            .args([
                "--session",
                &workspace.session,
                "pane",
                "split",
                &tab.root_pane_id,
                "--direction",
                "down",
                "--no-focus",
            ])
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        serde_json::from_slice::<Value>(&output.stdout)
            .ok()?
            .pointer("/result/pane/pane_id")?
            .as_str()?
            .to_string()
    };
    let _ = Command::new(&herdr)
        .args([
            "--session",
            &workspace.session,
            "pane",
            "rename",
            &pane_id,
            pane_label,
        ])
        .status();
    if let Some(stage_tab) = workspace.tabs.get_mut(&stage_key) {
        stage_tab.worker_count = stage_tab.worker_count.saturating_add(1);
    }
    registry.insert(run_key, workspace.clone());
    drop(registry);
    ensure_herdr_client(&herdr, &workspace.session);

    let escaped_herdr = herdr.replace('\'', "''");
    let escaped_session = workspace.session.replace('\'', "''");
    let escaped_pane = pane_id.replace('\'', "''");
    let viewer = worker_console_script(title, log_path, prompt_path).replace(
        &format!(
            "if ($line -eq '{WORKER_CONSOLE_FINISHED_SENTINEL}') {{ Write-Host 'SWARMS worker finished. Press Enter to close.' -ForegroundColor Green; Read-Host | Out-Null; exit 0 }}"
        ),
        &format!("if ($line -eq '{WORKER_CONSOLE_FINISHED_SENTINEL}') {{ & '{escaped_herdr}' --session '{escaped_session}' pane close '{escaped_pane}'; exit 0 }}"),
    );
    let script_path = work_dir.join("herdr-viewer.ps1");
    fs::write(&script_path, viewer).ok()?;
    let script_path = script_path.canonicalize().ok()?;
    let command = format!(
        "powershell.exe -NoLogo -NoProfile -ExecutionPolicy Bypass -File \"{}\"",
        script_path.display()
    );
    if !Command::new(&herdr)
        .args([
            "--session",
            &workspace.session,
            "pane",
            "run",
            &pane_id,
            &command,
        ])
        .status()
        .is_ok_and(|status| status.success())
    {
        return None;
    }
    Some(WorkerTerminal {
        backend: "herdr".to_string(),
        session: Some(workspace.session),
        workspace_id: Some(workspace.workspace_id),
        tab_id: Some(tab.tab_id),
        pane_id: Some(pane_id),
    })
}

#[cfg(windows)]
fn ensure_herdr_client(herdr: &str, session: &str) {
    use std::os::windows::process::CommandExt;

    static HERDR_CLIENT_SESSIONS: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    let clients = HERDR_CLIENT_SESSIONS.get_or_init(|| Mutex::new(HashSet::new()));
    let Ok(mut clients) = clients.lock() else {
        return;
    };
    if clients.contains(session) {
        return;
    }
    if herdr_client_is_running(session) {
        clients.insert(session.to_string());
        return;
    }

    let escaped_herdr = herdr.replace('\'', "''");
    let escaped_session = session.replace('\'', "''");
    let command = format!("& '{escaped_herdr}' --session '{escaped_session}'");
    let title = format!("Herdr | {session}");
    let shell_args = herdr_client_shell_args(&command);
    let launched = Command::new("wt.exe")
        .args(["new-tab", "--title", &title, "powershell.exe"])
        .args(shell_args)
        .status()
        .is_ok_and(|status| status.success())
        || Command::new("powershell.exe")
            .args(shell_args)
            .creation_flags(0x0000_0010)
            .spawn()
            .is_ok();

    if launched {
        clients.insert(session.to_string());
    }
}

#[cfg(windows)]
fn herdr_client_shell_args(command: &str) -> [&str; 4] {
    ["-NoLogo", "-NoProfile", "-Command", command]
}

#[cfg(windows)]
fn herdr_client_is_running(session: &str) -> bool {
    use std::os::windows::process::CommandExt;

    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    const SCRIPT: &str = "$needle = '--session ' + $env:SWARMS_HERDR_CLIENT_SESSION; $client = Get-CimInstance Win32_Process -Filter \"Name='herdr.exe'\" | Where-Object { $_.CommandLine -and $_.CommandLine.Contains($needle) -and -not $_.CommandLine.TrimEnd().EndsWith(' server') }; if ($client) { exit 0 } else { exit 1 }";
    Command::new("powershell.exe")
        .args(["-NoLogo", "-NoProfile", "-Command", SCRIPT])
        .env("SWARMS_HERDR_CLIENT_SESSION", session)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW)
        .status()
        .is_ok_and(|status| status.success())
}

#[cfg(windows)]
fn herdr_workspace_key(run_dir: &Path) -> Option<String> {
    Some(run_dir.canonicalize().ok()?.to_string_lossy().to_string())
}

#[cfg(windows)]
fn close_herdr_workspaces(states: &[TaskState]) {
    let herdr = herdr_program();
    let mut workspaces = HashSet::new();
    for state in states {
        let (Some(session), Some(workspace_id)) = (
            state.terminal_session.as_deref(),
            state.terminal_workspace_id.as_deref(),
        ) else {
            continue;
        };
        if state.terminal_backend.as_deref() != Some("herdr")
            || !workspaces.insert((session.to_string(), workspace_id.to_string()))
        {
            continue;
        }
        let _ = Command::new(&herdr)
            .args(["--session", session, "workspace", "close", workspace_id])
            .status();
    }
}

#[cfg(not(windows))]
fn close_herdr_workspaces(_states: &[TaskState]) {}

#[cfg(windows)]
fn herdr_label(value: &str, max_chars: usize) -> String {
    let mut label = String::with_capacity(value.len());
    let mut pending_space = false;
    for ch in value.chars() {
        if ch.is_control() || ch.is_whitespace() {
            pending_space = !label.is_empty();
            continue;
        }
        if pending_space {
            label.push(' ');
            pending_space = false;
        }
        label.push(ch);
        if label.chars().count() >= max_chars {
            break;
        }
    }
    let label = label.trim().to_string();
    if label.is_empty() {
        "Unassigned".to_string()
    } else {
        label
    }
}

#[cfg(windows)]
fn herdr_pane_label(task: &Task) -> String {
    herdr_label(
        &format!(
            "Task | {} | {} | {} | {}",
            task.id, task.spec.role, task.provider.provider, task.provider.model
        ),
        80,
    )
}

#[cfg(windows)]
fn herdr_server_is_running(herdr: &str, session: &str) -> bool {
    Command::new(herdr)
        .args(["--session", session, "status", "server"])
        .output()
        .ok()
        .is_some_and(|output| {
            output.status.success()
                && String::from_utf8_lossy(&output.stdout).contains("status: running")
        })
}

/// Renders the OpenCode JSONL protocol as a readable console without changing
/// the raw `worker.log` used by the runtime for auditing.
#[cfg(windows)]
fn worker_console_script(title: &str, log_path: &str, prompt_path: &str) -> String {
    format!(
        r#"$Host.UI.RawUI.WindowTitle='{title}';
Write-Host 'SWARMS worker active: {title}' -ForegroundColor Green;
Write-Host '--- Assigned prompt ---' -ForegroundColor DarkCyan;
Get-Content -LiteralPath '{prompt_path}' -TotalCount 40;
Write-Host '--- Live agent activity ---' -ForegroundColor DarkCyan;
function Show-SwarmsEvent([string]$line) {{
    if ($line -eq '{WORKER_CONSOLE_FINISHED_SENTINEL}') {{ Write-Host 'SWARMS worker finished. Press Enter to close.' -ForegroundColor Green; Read-Host | Out-Null; exit 0 }}
    try {{ $event = $line | ConvertFrom-Json -ErrorAction Stop }} catch {{ Write-Host $line; return }}
    $part = $event.part
    switch ($event.type) {{
        'text' {{ if ($part.text) {{ Write-Host "AGENT> $($part.text)" -ForegroundColor Cyan }} }}
        'tool_use' {{
            $name = if ($part.tool) {{ $part.tool }} else {{ 'tool' }}
            $status = if ($part.state.status) {{ $part.state.status }} else {{ 'started' }}
            $input = $part.state.input
            $detail = if ($input.command) {{ $input.command }} elseif ($input.filePath) {{ $input.filePath }} elseif ($input.path) {{ $input.path }} else {{ '' }}
            Write-Host "TOOL [$name] $status $detail" -ForegroundColor Yellow
        }}
        'step_start' {{ Write-Host '--- step started ---' -ForegroundColor DarkGray }}
        'step_finish' {{ Write-Host '--- step finished ---' -ForegroundColor DarkGray }}
        'error' {{ Write-Host "ERROR> $($part.error)" -ForegroundColor Red }}
    }}
}}
Get-Content -LiteralPath '{log_path}' -Wait -Tail 50 | ForEach-Object {{ Show-SwarmsEvent $_ }}"#
    )
}

#[cfg(not(windows))]
fn start_visible_worker_console(
    _workspace_root: &Path,
    _run_dir: &Path,
    _work_dir: &Path,
    _task: &Task,
    _terminal: &crate::model::TerminalConfig,
) -> Option<WorkerTerminal> {
    None
}

fn tail_chars(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        s.chars()
            .rev()
            .take(max)
            .collect::<String>()
            .chars()
            .rev()
            .collect()
    }
}

// ---------------------------------------------------------------------------
// Artifact check, freshness, and protected-path enforcement
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq)]
struct FileStamp {
    modified: Option<SystemTime>,
    len: u64,
    is_dir: bool,
}

/// Metadata snapshot of the paths a task cares about, captured before the
/// worker runs. Full `SystemTime` precision plus length avoids the false
/// equality caused by truncating mtimes to milliseconds.
#[derive(Clone, Debug, Default)]
pub(crate) struct ArtifactSnapshot {
    entries: Vec<(String, Option<FileStamp>)>,
}

impl ArtifactSnapshot {
    fn stamp_of(&self, rel: &str) -> Option<FileStamp> {
        self.entries
            .iter()
            .find(|(path, _)| path == rel)
            .and_then(|(_, stamp)| stamp.clone())
    }
}

/// Capture metadata for the task's declared artifacts and protected paths.
/// Cheap: only `stat`s the explicitly declared set, never a workspace walk.
pub(crate) fn capture_artifact_snapshot(root: &Path, task: &Task) -> ArtifactSnapshot {
    let entries = task
        .spec
        .artifacts
        .iter()
        .chain(task.spec.protected.iter())
        .map(|rel| (rel.clone(), read_file_stamp(&root.join(rel))))
        .collect();
    ArtifactSnapshot { entries }
}

fn read_file_stamp(path: &Path) -> Option<FileStamp> {
    let meta = fs::metadata(path).ok()?;
    Some(FileStamp {
        modified: meta.modified().ok(),
        len: meta.len(),
        is_dir: meta.is_dir(),
    })
}

/// Validate declared artifacts and, when a pre-task snapshot is available,
/// enforce freshness (the worker touched each artifact) and protected-path
/// integrity (the worker did not modify any protected path).
pub(crate) fn check_artifacts_with_snapshot(
    root: &Path,
    task: &Task,
    pre: Option<&ArtifactSnapshot>,
) -> Result<()> {
    let root_canonical = root
        .canonicalize()
        .map_err(|e| format!("canonicalize root: {e}"))?;

    // Existence + containment for every declared artifact.
    for art in &task.spec.artifacts {
        let path = root.join(art);
        if !path.exists() {
            return Err(format!("declared artifact not found after task: {art}"));
        }
        let canonical = path
            .canonicalize()
            .map_err(|e| format!("canonicalize {art}: {e}"))?;
        if !canonical.starts_with(&root_canonical) {
            return Err(format!("artifact escapes workspace: {art}"));
        }
    }

    // Freshness: each declared artifact must have been created or modified by
    // this task. A pre-existing file that the worker never touched used to
    // satisfy the existence check above; the snapshot closes that hole.
    if let Some(snapshot) = pre {
        for art in &task.spec.artifacts {
            let now = read_file_stamp(&root.join(art));
            let before = snapshot.stamp_of(art);
            if before.is_some() && now == before {
                return Err(format!("declared artifact was not modified by task: {art}"));
            }
        }

        // Protected paths: if the task lists any (e.g. golden files it is
        // evaluated against), the worker must not have touched them.
        for rel in &task.spec.protected {
            let now = read_file_stamp(&root.join(rel));
            let before = snapshot.stamp_of(rel);
            if now != before {
                return Err(format!("task modified a protected path: {rel}"));
            }
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Verify commands
// ---------------------------------------------------------------------------

/// Default deadline for deterministic verification commands. These are meant
/// to be fast (compilers, test runners, linters); a command that does not
/// finish in this window is stuck, not doing productive work, and holding a
/// completion gate open indefinitely.
const VERIFY_DEADLINE: Duration = Duration::from_secs(15 * 60);

/// Poll a child process until it exits or `deadline` elapses. On timeout the
/// child is killed and reaped so no descendant lingers holding the gate open.
/// Returns `Ok(status)` on exit, or `Err` with a timeout message.
///
/// This is the single bounded-wait primitive shared by every short-lived
/// deterministic command the coordinator runs (currently verification). It
/// replaces open-ended `try_wait` loops that could block a task forever if a
/// verifier hangs.
pub(crate) fn wait_bounded(
    program: &str,
    child: &mut std::process::Child,
    deadline: Duration,
    on_still_running: Option<&dyn Fn()>,
) -> Result<std::process::ExitStatus> {
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Ok(status),
            Ok(None) => {
                if started.elapsed() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(format!(
                        "process '{}' exceeded the {}s deadline and was killed",
                        program,
                        deadline.as_secs()
                    ));
                }
                if let Some(callback) = on_still_running {
                    callback();
                }
                thread::sleep(Duration::from_millis(50));
            }
            Err(e) => return Err(format!("wait '{}': {e}", program)),
        }
    }
}

pub(crate) fn run_verify_commands(
    task: &Task,
    root: &Path,
    work_dir: &Path,
) -> (Option<bool>, Option<String>) {
    if task.spec.verify.is_empty() {
        return (None, None);
    }
    for (index, cmd_str) in task.spec.verify.iter().enumerate() {
        let log_name = format!("verify-{:03}.log", index + 1);
        let log_path = work_dir.join(&log_name);
        let started_at = now_iso();
        let result = execute_shell_bounded_status(cmd_str, root, &log_path, VERIFY_DEADLINE);
        let ended_at = now_iso();
        let log_content = fs::read_to_string(&log_path).unwrap_or_default();
        let mut evidence = json!({
            "task_id": task.id,
            "index": index + 1,
            "command": cmd_str,
            "log": log_name,
            "started_at": started_at,
            "ended_at": ended_at,
            "output_fnv1a64": format!("{:016x}", fnv1a64(log_content.as_bytes())),
        });
        match result {
            Ok(status) if status.success() => {
                evidence["success"] = json!(true);
                evidence["exit_code"] = json!(status.code());
            }
            Ok(status) => {
                evidence["success"] = json!(false);
                evidence["exit_code"] = json!(status.code());
                append_verification_evidence(work_dir, &evidence);
                let tail = tail_chars(&log_content, 2000);
                return (
                    Some(false),
                    Some(format!("verify failed (exit {:?}): {tail}", status.code())),
                );
            }
            Err(error) => {
                evidence["success"] = json!(false);
                evidence["exit_code"] = Value::Null;
                evidence["error"] = json!(error.clone());
                append_verification_evidence(work_dir, &evidence);
                return (Some(false), Some(error));
            }
        }
        append_verification_evidence(work_dir, &evidence);
    }
    (Some(true), None)
}

/// Codex CLI can emit its final JSON event before the wrapper process exits.
/// Treat that explicit protocol event as terminal after the same short grace
/// period used for OpenCode, so a leaked provider wrapper cannot strand a task
/// in `in_progress` after the turn has completed.
fn codex_terminal_event_seen(log_path: &Path) -> bool {
    let Ok(content) = fs::read_to_string(log_path) else {
        return false;
    };
    content.lines().any(|line| {
        serde_json::from_str::<Value>(line)
            .ok()
            .and_then(|item| item.get("type").and_then(Value::as_str).map(str::to_string))
            .is_some_and(|event_type| event_type == "turn.completed")
    })
}

fn append_verification_evidence(work_dir: &Path, evidence: &Value) {
    let path = work_dir.join("verification.jsonl");
    let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) else {
        return;
    };
    let _ = serde_json::to_writer(&mut file, evidence);
    let _ = writeln!(file);
}

#[allow(dead_code)]
pub(crate) fn execute_shell(cmd_str: &str, cwd: &Path, log_path: &Path) -> Result<()> {
    execute_shell_bounded(cmd_str, cwd, log_path, VERIFY_DEADLINE)
}

/// Run a shell command under an explicit deadline. Split from `execute_shell`
/// so tests can exercise the timeout path with a short deadline instead of
/// waiting for the full production `VERIFY_DEADLINE`.
#[allow(dead_code)]
pub(crate) fn execute_shell_bounded(
    cmd_str: &str,
    cwd: &Path,
    log_path: &Path,
    deadline: Duration,
) -> Result<()> {
    execute_shell_bounded_status(cmd_str, cwd, log_path, deadline).map(|_| ())
}

fn execute_shell_bounded_status(
    cmd_str: &str,
    cwd: &Path,
    log_path: &Path,
    deadline: Duration,
) -> Result<std::process::ExitStatus> {
    if let Some(parent) = log_path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let log = fs::File::create(log_path).map_err(|e| format!("{}: {e}", log_path.display()))?;
    let err = log.try_clone().map_err(|e| e.to_string())?;

    #[cfg(windows)]
    let mut command = {
        use std::os::windows::process::CommandExt;
        let mut c = Command::new("cmd");
        c.raw_arg(format!("/D /S /C \"{cmd_str}\""));
        c
    };
    #[cfg(not(windows))]
    let mut command = {
        let mut c = Command::new("sh");
        c.arg("-c").arg(cmd_str);
        c
    };
    command
        .current_dir(cwd)
        .stdout(Stdio::from(log))
        .stderr(Stdio::from(err));

    let mut child = command.spawn().map_err(|e| format!("spawn verify: {e}"))?;
    wait_bounded("verify", &mut child, deadline, None)
}

// ---------------------------------------------------------------------------
// State constructors
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
pub(crate) fn success_state(
    task: &Task,
    thinking: ThinkingLevel,
    started: Instant,
    attempts: u32,
    session_reused: bool,
    session_id: Option<String>,
    session_resume_count: u32,
    verified: Option<bool>,
    verify_error: Option<String>,
    usage: &Usage,
    transport: Option<String>,
) -> TaskState {
    let elapsed = started.elapsed().as_millis();
    TaskState {
        task_id: task.id.clone(),
        source_id: task.source_id.clone(),
        status: TaskStatus::Completed,
        attempts,
        stage: task.stage.clone(),
        route: task.spec.route.clone(),
        effective_route: task.effective_route.clone(),
        provider: task.provider.provider.clone(),
        model: task.provider.model.clone(),
        role: task.spec.role.clone(),
        thinking: Some(thinking),
        duration_ms: elapsed,
        session_created: !session_reused && session_id.is_some(),
        session_reused,
        session_resume_count,
        session_id,
        transport,
        verified,
        verify_error,
        usage: usage.clone(),
        error: None,
        started_at: Some(now_iso()),
        heartbeat_unix_ms: Some(unix_ms()),
        worker_log_bytes: 0,
        last_progress_unix_ms: None,
        worker_log_modified_unix_ms: None,
        terminal_backend: None,
        terminal_session: None,
        terminal_workspace_id: None,
        terminal_tab_id: None,
        terminal_pane_id: None,
        ended_at: Some(now_iso()),
        checkpoint_key: None,
        scaling: None,
    }
}

fn attach_session_context(
    state: &mut TaskState,
    session_reused: bool,
    session_id: Option<String>,
    session_resume_count: u32,
    transport: Option<String>,
) {
    state.session_created = !session_reused && session_id.is_some();
    state.session_reused = session_reused;
    state.session_resume_count = session_resume_count;
    state.session_id = session_id;
    if transport.is_some() {
        state.transport = transport;
    }
}

pub(crate) fn failed_state(
    task: &Task,
    thinking: ThinkingLevel,
    started: Instant,
    attempts: u32,
    error: &str,
    usage: &Usage,
) -> TaskState {
    TaskState {
        task_id: task.id.clone(),
        source_id: task.source_id.clone(),
        status: TaskStatus::Failed,
        attempts,
        stage: task.stage.clone(),
        route: task.spec.route.clone(),
        effective_route: task.effective_route.clone(),
        provider: task.provider.provider.clone(),
        model: task.provider.model.clone(),
        role: task.spec.role.clone(),
        thinking: Some(thinking),
        duration_ms: started.elapsed().as_millis(),
        session_created: false,
        session_reused: false,
        session_resume_count: 0,
        session_id: None,
        transport: None,
        verified: None,
        verify_error: None,
        usage: usage.clone(),
        error: Some(error.to_string()),
        started_at: Some(now_iso()),
        heartbeat_unix_ms: Some(unix_ms()),
        worker_log_bytes: 0,
        last_progress_unix_ms: None,
        worker_log_modified_unix_ms: None,
        terminal_backend: None,
        terminal_session: None,
        terminal_workspace_id: None,
        terminal_tab_id: None,
        terminal_pane_id: None,
        ended_at: Some(now_iso()),
        checkpoint_key: None,
        scaling: None,
    }
}

// ---------------------------------------------------------------------------
// Dry-run
// ---------------------------------------------------------------------------

pub fn dry_run(
    run_dir: &Path,
    workspace_root: &Path,
    run_id: &str,
    tasks: &[Task],
    plan: &Plan,
    global_cap: usize,
    caps: &HashMap<String, usize>,
) -> Result<Report> {
    fs::create_dir_all(run_dir).map_err(|e| format!("{}: {e}", run_dir.display()))?;
    let (project_id, project_name) = resolve_project(plan, workspace_root);
    save_workflow(
        run_dir,
        workspace_root,
        run_id,
        tasks.len(),
        global_cap,
        caps,
        heartbeat_interval_seconds(),
        &project_id,
        &project_name,
        &plan.execution,
        &plan.terminal,
    )?;

    let states: Vec<TaskState> = tasks
        .iter()
        .map(|t| {
            let mut s = TaskState::new(&t.id, &t.source_id, &t.stage, &t.spec.route);
            s.status = TaskStatus::Pending;
            s.provider = t.provider.provider.clone();
            s.effective_route = t.effective_route.clone();
            s.model = t.provider.model.clone();
            s.role = t.spec.role.clone();
            s.thinking = Some(t.spec.effective_thinking(plan));
            s
        })
        .collect();

    let mut report = telemetry::build_report(
        run_id,
        &run_dir.to_string_lossy(),
        &states,
        global_cap,
        caps,
        Vec::new(),
    );
    report.status = "planned".to_string();
    let report_value = serde_json::to_value(&report).map_err(|e| e.to_string())?;
    write_json_value(&run_dir.join("report.json"), &report_value)?;
    Ok(report)
}

#[cfg(test)]
mod auto_resume_tests {
    use super::*;

    #[test]
    fn failed_provider_session_is_only_reused_inside_the_bounded_window() {
        let output = r#"{"type":"thread.started","thread_id":"exact-session"}"#;
        let updated = UNIX_EPOCH + Duration::from_secs(1_000);
        let window = Duration::from_secs(300);

        assert_eq!(
            session_id_if_fresh(
                AdapterKind::Codex,
                output,
                updated,
                updated + window,
                window,
            )
            .as_deref(),
            Some("exact-session")
        );
        assert!(session_id_if_fresh(
            AdapterKind::Codex,
            output,
            updated,
            updated + window + Duration::from_millis(1),
            window,
        )
        .is_none());
    }

    #[test]
    fn failed_provider_session_rejects_future_or_invalid_output() {
        let now = UNIX_EPOCH + Duration::from_secs(1_000);
        assert!(session_id_if_fresh(
            AdapterKind::Codex,
            r#"{"thread_id":"future"}"#,
            now + Duration::from_secs(1),
            now,
            Duration::from_secs(300),
        )
        .is_none());
        assert!(session_id_if_fresh(
            AdapterKind::Codex,
            "not-json",
            now,
            now,
            Duration::from_secs(300),
        )
        .is_none());
    }

    #[test]
    fn worker_console_signal_is_appended_to_its_log() {
        let path = std::env::temp_dir().join(format!("swarms-console-{}.log", unix_ms()));
        fs::write(&path, "worker output\n").unwrap();
        signal_worker_console_finished(&path);
        let content = fs::read_to_string(&path).unwrap();
        assert!(content.ends_with(&format!("{WORKER_CONSOLE_FINISHED_SENTINEL}\n")));
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn opencode_database_lock_uses_visible_startup_backoff() {
        let error = "process 'opencode' exited Some(1): database is locked";
        assert!(is_transient_opencode_database_lock(error));
        assert_eq!(retry_reason(error), "transient_opencode_database_lock");
        assert_eq!(retry_delay(error, 1), Duration::from_secs(5));
        assert_eq!(retry_delay(error, 4), Duration::from_secs(40));
    }

    #[test]
    fn ordinary_adapter_error_keeps_short_retry_backoff() {
        let error = "process 'provider' exited Some(1): unavailable";
        assert!(!is_transient_opencode_database_lock(error));
        assert_eq!(retry_reason(error), "adapter_error");
        assert_eq!(retry_delay(error, 1), Duration::from_millis(100));
        assert_eq!(retry_delay(error, 8), Duration::from_millis(3200));
    }

    #[test]
    fn dependency_output_uses_text_events_not_tool_payloads() {
        let log = [
            r#"{"type":"tool_use","part":{"tool":"read","output":"very large payload"}}"#,
            r#"{"type":"text","part":{"text":"The contract is complete."}}"#,
            r#"{"type":"text","part":{"text":"12 focused tests pass."}}"#,
            WORKER_CONSOLE_FINISHED_SENTINEL,
        ]
        .join("\n");
        assert_eq!(
            readable_worker_output(&log),
            "The contract is complete.\n\n12 focused tests pass."
        );
    }

    #[test]
    fn dependency_output_preserves_plain_text_without_viewer_sentinel() {
        assert_eq!(
            readable_worker_output("plain result\n__SWARMS_WORKER_FINISHED__\n"),
            "plain result"
        );
    }

    #[test]
    fn opencode_terminal_event_requires_explicit_stop_protocol() {
        let path = std::env::temp_dir().join(format!("swarms-opencode-stop-{}.log", unix_ms()));
        fs::write(
            &path,
            r#"{"type":"step_finish","part":{"reason":"tool-calls"}}"#,
        )
        .unwrap();
        assert!(!opencode_terminal_event_seen(&path));
        fs::write(&path, r#"{"type":"step_finish","part":{"reason":"stop"}}"#).unwrap();
        assert!(opencode_terminal_event_seen(&path));
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn codex_terminal_event_requires_explicit_completion_protocol() {
        let path = std::env::temp_dir().join(format!("swarms-codex-complete-{}.log", unix_ms()));
        fs::write(&path, r#"{"type":"item.completed"}"#).unwrap();
        assert!(!codex_terminal_event_seen(&path));
        fs::write(&path, r#"{"type":"turn.completed","usage":{}}"#).unwrap();
        assert!(codex_terminal_event_seen(&path));
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn steering_continuation_distinguishes_queued_and_cancelled_delivery() {
        let message = steering::SteerMessage {
            id: "1".to_string(),
            created_at_epoch_ms: 1,
            prompt: "Use the smaller API.".to_string(),
            source: "test".to_string(),
            mode: SteeringMode::Enqueue,
        };
        let queued = steering_continuation("original", std::slice::from_ref(&message), false);
        assert!(!queued.contains("original"));
        assert!(queued.contains("previous turn completed"));

        let restarted = steering_continuation("original", &[message], true);
        assert!(restarted.contains("original"));
        assert!(restarted.contains("Restart from the persisted task prompt"));
    }

    #[cfg(windows)]
    #[test]
    fn hidden_legacy_console_keeps_herd_observability_enabled() {
        assert!(!should_start_worker_terminal(
            Some(AdapterKind::OpenCode),
            false,
            true,
        ));
        assert!(should_start_worker_terminal(
            Some(AdapterKind::OpenCode),
            true,
            true,
        ));
        assert!(should_start_worker_terminal(
            Some(AdapterKind::Mock),
            true,
            true,
        ));
    }

    #[cfg(windows)]
    #[test]
    fn worker_console_script_formats_opencode_jsonl_without_losing_the_sentinel() {
        let script = worker_console_script("SWARMS | probe", "C:/log", "C:/prompt");
        assert!(script.contains("ConvertFrom-Json"));
        assert!(script.contains("AGENT>"));
        assert!(script.contains("TOOL"));
        assert!(script.contains(WORKER_CONSOLE_FINISHED_SENTINEL));
        assert!(script.contains("Read-Host | Out-Null"));
    }

    #[cfg(windows)]
    #[test]
    fn herdr_client_exits_with_the_attached_client() {
        let args = herdr_client_shell_args("herdr --session swarms");
        assert!(!args.contains(&"-NoExit"));
    }

    #[cfg(windows)]
    #[test]
    fn herdr_workspace_key_is_run_scoped() {
        let run_dir = std::env::temp_dir().join(format!("swarms-herdr-run-{}", unix_ms()));
        fs::create_dir_all(run_dir.join("results/worker-a")).unwrap();
        fs::create_dir_all(run_dir.join("results/worker-b")).unwrap();

        let key = herdr_workspace_key(&run_dir).unwrap();
        assert_eq!(key, run_dir.canonicalize().unwrap().to_string_lossy());

        fs::remove_dir_all(run_dir).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn herdr_labels_are_bounded_and_collapse_control_whitespace() {
        assert_eq!(herdr_label("  Build\n\tworkers  ", 80), "Build workers");
        assert_eq!(herdr_label("\n\t", 80), "Unassigned");
        assert!(herdr_label("a very long stage name", 8).chars().count() <= 8);
    }
}
