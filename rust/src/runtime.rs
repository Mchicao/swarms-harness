//! Deterministic DAG scheduler with persisted state, retries, and resume.

use crate::adapter::{self, AdapterKind, CliSpec};
use crate::model::{find_dependency_task, Plan, Router, SessionMode, Task, ThinkingLevel};
use crate::quota::QuotaGuard;
use crate::session::{self, SessionDecision, SessionStore};
use crate::steering::{self, AppliedSteer};
use crate::telemetry::{self, Report, TaskState, TaskStatus, Usage};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

type Result<T> = std::result::Result<T, String>;

const MAX_DEP_CONTEXT_CHARS: usize = 12_000;

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

fn append_event(run_dir: &Path, event_type: &str, payload: Value) {
    let item = json!({"time": now_iso(), "event": event_type, "payload": payload});
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
        let checkpoint_matches = state.checkpoint_key.as_deref() == Some(&checkpoint_key);
        if !state.status.is_completed() || !checkpoint_matches {
            state.status = TaskStatus::Pending;
            state.error = None;
            state.verified = None;
            state.verify_error = None;
            state.started_at = None;
            state.heartbeat_unix_ms = None;
            state.ended_at = None;
        }
        state.checkpoint_key = Some(checkpoint_key);
    }
    Ok(states)
}

fn task_checkpoint_key(task: &Task, plan: &Plan) -> String {
    let session = task.spec.effective_session(plan);
    let definition = json!({
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
        "timeout_seconds": task.spec.effective_timeout(plan),
        "max_attempts": task.spec.effective_max_attempts(plan),
    })
    .to_string();
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

    for task in tasks {
        let state = states.get(&task.id);
        if state.is_some_and(|s| s.status.is_terminal()) {
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

        if selected.len() >= global_cap {
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
    let run_dir = root.join(".agent/swarm/runs").join(run_id);
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
        )?;
        append_event(
            &run_dir,
            "workflow_initialized",
            json!({"task_count": tasks.len()}),
        );
    }

    let session_store = Arc::new(SessionStore::open(&run_dir)?);
    loop {
        // Reload each wave so long runs see the monitor's latest atomic snapshot.
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

        if ready.selected.is_empty() {
            break;
        }

        let (sender, receiver) = mpsc::channel::<(String, TaskState)>();
        let mut active_ids = HashSet::new();
        for task in &ready.selected {
            let prompt = build_task_prompt(&run_dir, task, tasks, &states);
            let work_dir = run_dir.join("results").join(&task.id);
            let _ = fs::create_dir_all(&work_dir);
            let _ = fs::write(work_dir.join("prompt.txt"), &prompt);

            let sender = sender.clone();
            let task = task.clone();
            let workspace_root = workspace_root.to_path_buf();
            let run_dir = run_dir.clone();
            let plan = plan.clone();
            let store = Arc::clone(&session_store);

            if let Some(state) = states.get_mut(&task.id) {
                state.status = TaskStatus::InProgress;
                state.started_at = Some(now_iso());
                state.heartbeat_unix_ms = Some(unix_ms());
                save_task_state(&run_dir, state)?;
            }
            active_ids.insert(task.id.clone());

            append_event(
                &run_dir,
                "task_started",
                json!({"task_id": task.id, "requested_route": task.spec.route, "effective_route": task.effective_route, "model": task.provider.model}),
            );

            thread::spawn(move || {
                let state = run_task(&workspace_root, &run_dir, &task, &plan, &prompt, &store);
                let _ = sender.send((task.id.clone(), state));
            });
        }
        drop(sender);

        let heartbeat_interval = Duration::from_secs(heartbeat_seconds);
        let mut last_heartbeat = Instant::now();
        while !active_ids.is_empty() {
            let wait = heartbeat_interval.saturating_sub(last_heartbeat.elapsed());
            match receiver.recv_timeout(wait) {
                Ok((task_id, mut state)) => {
                    active_ids.remove(&task_id);
                    if let Some(previous) = states.get(&task_id) {
                        state.started_at.clone_from(&previous.started_at);
                        state.checkpoint_key.clone_from(&previous.checkpoint_key);
                    }
                    states.insert(task_id.clone(), state.clone());
                    save_task_state(&run_dir, &states[&task_id])?;
                    append_event(
                        &run_dir,
                        "task_finished",
                        json!({"task_id": task_id, "status": format!("{:?}", state.status).to_lowercase()}),
                    );
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
    }

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

    Ok(report)
}

// ---------------------------------------------------------------------------
// Prompt generation
// ---------------------------------------------------------------------------

fn build_task_prompt(
    run_dir: &Path,
    task: &Task,
    all_tasks: &[Task],
    states: &HashMap<String, TaskState>,
) -> String {
    let dep_context = dependency_outputs(run_dir, task, all_tasks, states);
    adapter::build_prompt(
        &task.spec.role,
        &task.spec.task,
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
            let mut start = content.len().saturating_sub(remaining);
            while start < content.len() && !content.is_char_boundary(start) {
                start += 1;
            }
            let excerpt = &content[start..];
            sections.push(format!("Dependency {} output:\n{excerpt}", dep_task.id));
            remaining = remaining.saturating_sub(excerpt.len());
        }
    }
    sections.join("\n\n")
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
    let timeout_secs = task.spec.effective_timeout(plan);
    let max_attempts = task.spec.effective_max_attempts(plan).max(1);
    let timeout = Duration::from_secs(timeout_secs);
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

    let mut attempt = 0_u32;

    let last_error = loop {
        attempt += 1;
        let exec_result = execute_adapter(
            task,
            prompt,
            thinking,
            active_session_id.as_deref(),
            root,
            &work_dir,
            timeout,
        );

        match exec_result {
            Ok(mut exec) => {
                let mut new_session_id = adapter::parse_session_id(
                    AdapterKind::from_wrapper(&task.provider.wrapper).unwrap_or(AdapterKind::Mock),
                    &exec.output,
                );
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
                            "{prompt}\n\nUSER STEER PROMPT\n{}\n\nApply this new direction before finishing the task.",
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
                            &work_dir,
                            timeout,
                        );
                        match steered {
                            Ok(next) => {
                                let command_id = message.id.clone();
                                let next_session_id = adapter::parse_session_id(
                                    AdapterKind::from_wrapper(&task.provider.wrapper)
                                        .unwrap_or(AdapterKind::Mock),
                                    &next.output,
                                );
                                if next_session_id.is_some() {
                                    new_session_id = next_session_id;
                                }
                                preserve_steering_log(&work_dir, &previous_log, &message.prompt);
                                merge_usage(&mut exec.usage, &next.usage);
                                exec.output = next.output;
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
                                    json!({"task_id": task.id, "command_id": command_id}),
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

                if let Err(e) = check_artifacts(root, task) {
                    return failed_state(task, thinking, started, attempt, &e, &exec.usage);
                }

                let (verified, verify_error) = run_verify_commands(task, root, &work_dir, timeout);

                if verified == Some(false) {
                    let err = verify_error
                        .as_deref()
                        .unwrap_or("verification command failed");
                    let mut state =
                        failed_state(task, thinking, started, attempt, err, &exec.usage);
                    state.verified = Some(false);
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
                    let backoff_ms = 100u64 << (attempt - 1).min(5);
                    thread::sleep(Duration::from_millis(backoff_ms.min(5000)));
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

fn session_resume_window() -> Duration {
    Duration::from_secs(
        std::env::var("SWARMS_SESSION_RESUME_WINDOW_SECONDS")
            .ok()
            .and_then(|value| value.parse().ok())
            .filter(|seconds: &u64| *seconds > 0)
            .unwrap_or(300),
    )
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

fn merge_usage(total: &mut Usage, next: &Usage) {
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

struct AdapterExec {
    output: String,
    usage: Usage,
}

fn execute_adapter(
    task: &Task,
    prompt: &str,
    thinking: ThinkingLevel,
    session_id: Option<&str>,
    root: &Path,
    work_dir: &Path,
    timeout: Duration,
) -> Result<AdapterExec> {
    let kind = AdapterKind::from_wrapper(&task.provider.wrapper)
        .ok_or_else(|| format!("unsupported wrapper: {}", task.provider.wrapper))?;

    match kind {
        AdapterKind::Mock => {
            let out = adapter::execute_mock(root, prompt)?;
            let _ = fs::write(work_dir.join("worker.log"), &out.stdout);
            Ok(AdapterExec {
                output: out.stdout,
                usage: Usage::offline_mock(),
            })
        }
        AdapterKind::OpenAiCompat => {
            let out = adapter::execute_openai_compat(task, prompt, thinking, timeout)?;
            let _ = fs::write(work_dir.join("worker.log"), &out.content);
            Ok(AdapterExec {
                output: out.content,
                usage: out.usage,
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
            let output = execute_cli(spec, root, &log_path, timeout)?;
            let usage = adapter::parse_cli_usage(kind, &output);
            Ok(AdapterExec { output, usage })
        }
    }
}

fn execute_cli(spec: CliSpec, cwd: &Path, log_path: &Path, timeout: Duration) -> Result<String> {
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
    let deadline = Instant::now() + timeout;

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
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    let output = fs::read_to_string(log_path).unwrap_or_default();
                    let tail = tail_chars(&output, 2000);
                    return Err(format!(
                        "process '{}' timed out after {}s\n{tail}",
                        spec.program,
                        timeout.as_secs()
                    ));
                }
                thread::sleep(Duration::from_millis(50));
            }
            Err(e) => return Err(format!("wait '{}': {e}", spec.program)),
        }
    }
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
// Artifact check
// ---------------------------------------------------------------------------

pub(crate) fn check_artifacts(root: &Path, task: &Task) -> Result<()> {
    for art in &task.spec.artifacts {
        let path = root.join(art);
        if !path.exists() {
            return Err(format!("declared artifact not found after task: {art}"));
        }
        let canonical = path
            .canonicalize()
            .map_err(|e| format!("canonicalize {art}: {e}"))?;
        let root_canonical = root
            .canonicalize()
            .map_err(|e| format!("canonicalize root: {e}"))?;
        if !canonical.starts_with(&root_canonical) {
            return Err(format!("artifact escapes workspace: {art}"));
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Verify commands
// ---------------------------------------------------------------------------

fn run_verify_commands(
    task: &Task,
    root: &Path,
    work_dir: &Path,
    timeout: Duration,
) -> (Option<bool>, Option<String>) {
    if task.spec.verify.is_empty() {
        return (None, None);
    }
    for cmd_str in &task.spec.verify {
        let log_path = work_dir.join("verify.log");
        match execute_shell(cmd_str, root, &log_path, timeout) {
            Ok(()) => {}
            Err(e) => return (Some(false), Some(e)),
        }
    }
    (Some(true), None)
}

pub(crate) fn execute_shell(
    cmd_str: &str,
    cwd: &Path,
    log_path: &Path,
    timeout: Duration,
) -> Result<()> {
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
    let deadline = Instant::now() + timeout;

    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                if status.success() {
                    return Ok(());
                }
                let log_content = fs::read_to_string(log_path).unwrap_or_default();
                let tail = tail_chars(&log_content, 2000);
                return Err(format!("verify failed (exit {:?}): {tail}", status.code()));
            }
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(format!("verify timed out after {}s", timeout.as_secs()));
                }
                thread::sleep(Duration::from_millis(50));
            }
            Err(e) => return Err(format!("wait verify: {e}")),
        }
    }
}

// ---------------------------------------------------------------------------
// State constructors
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn success_state(
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
        verified,
        verify_error,
        usage: usage.clone(),
        error: None,
        started_at: Some(now_iso()),
        heartbeat_unix_ms: Some(unix_ms()),
        ended_at: Some(now_iso()),
        checkpoint_key: None,
    }
}

fn failed_state(
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
        verified: None,
        verify_error: None,
        usage: usage.clone(),
        error: Some(error.to_string()),
        started_at: Some(now_iso()),
        heartbeat_unix_ms: Some(unix_ms()),
        ended_at: Some(now_iso()),
        checkpoint_key: None,
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
}
