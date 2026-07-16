//! SWARMS read-only run observer.
//!
//! This file is the root of two compilation units of the `swarms-runtime`
//! package:
//!
//! * the `swarms_ui` library (always compiled, serde + std only): a pure,
//!   testable, read-only model of the on-disk run contract described in
//!   `docs/STATE_CONTRACT.md` and `docs/SWARM_UI_CONTRACT.md`;
//! * the `swarms-ui` binary (compiled only with the `ui-egui` feature): a
//!   native egui/eframe window that renders that contract.
//!
//! The observer NEVER writes run state, NEVER claims tasks and NEVER spawns
//! workers. It only reads `workflow.json`, `tasks/*.json`, `claims/*.lock`,
//! `events.jsonl`, `results/<task_id>/worker.log` and the terminal report.
//!
//! See `docs/SWARM_UI.md` for usage and the exact Windows toolchain blocker.

use serde_json::Value;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub const CONTRACT_SCHEMA_VERSION: u64 = 1;
const MAX_ERROR_CHARS: usize = 1000;
/// SWARMS-UI: hard cap on resident worker.log bytes, per UI_RUNTIME_EVALUATION.
pub const MAX_LOG_BYTES: u64 = 2 * 1024 * 1024;

// ---------------------------------------------------------------------------
// Model
// ---------------------------------------------------------------------------

/// Derived run status. `Loading` and `Error` are UI transient states; the rest
/// mirror `SWARM_UI_CONTRACT.md` run-status derivation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunStatus {
    Empty,
    Loading,
    Running,
    Completed,
    Failed,
    Partial,
    Error,
}

impl Default for RunStatus {
    fn default() -> Self {
        RunStatus::Empty
    }
}

impl RunStatus {
    pub fn label(self) -> &'static str {
        match self {
            RunStatus::Empty => "empty",
            RunStatus::Loading => "loading",
            RunStatus::Running => "running",
            RunStatus::Completed => "completed",
            RunStatus::Failed => "failed",
            RunStatus::Partial => "partial",
            RunStatus::Error => "error",
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct RunContract {
    pub schema_version: u64,
    pub read_only: bool,
    pub run: RunMeta,
    pub summary: Summary,
    pub stages: Vec<StageNode>,
}

#[derive(Clone, Debug, Default)]
pub struct RunMeta {
    pub run_id: String,
    pub runtime: String,
    pub state_schema_version: Option<u64>,
    pub created_unix_ms: Option<u128>,
    pub status: RunStatus,
    pub workspace_root: Option<String>,
    pub heartbeat_interval_seconds: Option<u64>,
    pub global_max_concurrency: Option<u64>,
    pub provider_max_concurrency: HashMap<String, u64>,
    pub task_count: usize,
    pub observed_unix_ms: u128,
}

#[derive(Clone, Debug, Default)]
pub struct Summary {
    pub stage_count: usize,
    pub task_status_counts: HashMap<String, usize>,
    pub has_real_provider: bool,
    pub last_heartbeat_unix_ms: Option<u128>,
    pub result_count: usize,
    pub report_status: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct StageNode {
    pub name: String,
    pub status_counts: HashMap<String, usize>,
    pub tasks: Vec<TaskNode>,
}

#[derive(Clone, Debug, Default)]
pub struct TaskNode {
    pub task_id: String,
    pub index: usize,
    pub role: String,
    pub source_id: Option<String>,
    pub parent_task_id: Option<String>,
    pub status: String,
    pub attempts: usize,
    pub model: Option<String>,
    pub provider: Option<String>,
    pub route: Option<String>,
    pub wrapper: Option<String>,
    pub variant: Option<String>,
    pub agent: AgentNode,
    pub subagents: Vec<SubagentNode>,
    pub provider_subagents: Vec<String>,
    pub provider_subagent_visibility: String,
    pub started_at: Option<String>,
    pub ended_at: Option<String>,
    pub heartbeat_unix_ms: Option<u128>,
    pub needs: Vec<String>,
    pub artifacts: Vec<String>,
    pub error: Option<String>,
}

impl TaskNode {
    /// A running/queued task is stale only relative to the run heartbeat
    /// interval, per STATE_CONTRACT. Staleness is a visual label and must not
    /// mutate the task status.
    pub fn is_stale(&self, now_ms: u128, interval_secs: u64) -> bool {
        let running = matches!(self.status.as_str(), "in_progress" | "queued");
        running
            && match self.heartbeat_unix_ms {
                Some(hb) => now_ms.saturating_sub(hb) > u128::from(interval_secs) * 1000,
                None => false,
            }
    }
}

#[derive(Clone, Debug, Default)]
pub struct AgentNode {
    pub agent_id: String,
    pub owner: Option<String>,
    pub claimed_at: Option<String>,
    pub heartbeat_at: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct SubagentNode {
    pub agent_id: String,
    pub task_id: Option<String>,
    pub status: String,
    pub model: Option<String>,
}

/// One sanitized lifecycle record from `events.jsonl`.
#[derive(Clone, Debug)]
pub struct EventRow {
    pub time_unix_ms: Option<u128>,
    pub event: String,
    pub task_id: Option<String>,
    pub model: Option<String>,
    pub provider: Option<String>,
    pub error: Option<String>,
}

impl EventRow {
    fn from_value(v: &Value) -> Self {
        EventRow {
            time_unix_ms: v
                .get("time_unix_ms")
                .and_then(Value::as_u64)
                .map(u128::from),
            event: get_str(v, "event").unwrap_or_default(),
            task_id: get_str(v, "task_id"),
            model: get_str(v, "model"),
            provider: get_str(v, "provider"),
            error: sanitize_error(v.get("error")),
        }
    }
}

/// Compact index entry for a discovered run, for the left panel.
#[derive(Clone, Debug)]
pub struct RunIndex {
    pub run_id: String,
    pub runtime: String,
    pub created_unix_ms: Option<u128>,
    pub task_count: usize,
    pub has_report: bool,
}

// ---------------------------------------------------------------------------
// Reader
// ---------------------------------------------------------------------------

/// Read-only observer for a single SWARMS run directory.
///
/// Holds the `events.jsonl` byte offset so successive `tail_events` calls only
/// decode newly-appended, complete newline-terminated records.
pub struct RunReader {
    run_dir: PathBuf,
    roots: Vec<PathBuf>,
    events_offset: u64,
}

impl RunReader {
    pub fn new(run_dir: impl Into<PathBuf>, roots: Vec<PathBuf>) -> Self {
        RunReader {
            run_dir: run_dir.into(),
            roots,
            events_offset: 0,
        }
    }

    /// Validate `run_id` exactly like the Python observer and resolve it under
    /// `run_root`, refusing path escapes.
    pub fn open(run_root: &Path, run_id: &str, mut roots: Vec<PathBuf>) -> Result<Self, String> {
        if !safe_run_id(run_id) {
            return Err(format!("unsafe run_id for observation: {run_id:?}"));
        }
        let run_root = run_root
            .canonicalize()
            .unwrap_or_else(|_| run_root.to_path_buf());
        let run_dir = run_root.join(run_id);
        if run_dir.parent() != Some(run_root.as_path()) {
            return Err(format!("run_id escapes run_root: {run_id:?}"));
        }
        if !roots.iter().any(|r| r == &run_root) {
            roots.push(run_root);
        }
        Ok(RunReader::new(run_dir, roots))
    }

    pub fn run_dir(&self) -> &Path {
        &self.run_dir
    }

    pub fn exists(&self) -> bool {
        self.run_dir.is_dir()
    }

    /// Build the full sanitized, read-only contract from disk. Never panics:
    /// missing/corrupt files degrade to defaults.
    pub fn read(&mut self) -> RunContract {
        let workflow = read_json(&self.run_dir.join("workflow.json"))
            .filter(Value::is_object)
            .unwrap_or(Value::Null);
        let tasks_raw = self.read_tasks_raw();
        let report = read_json(&self.run_dir.join("report.json"))
            .or_else(|| read_json(&self.run_dir.join("report-rs.json")))
            .filter(Value::is_object);

        let workspace_root = get_str(&workflow, "workspace_root");
        let mut roots = self.roots.clone();
        if let Some(ref ws) = workspace_root {
            roots.insert(0, PathBuf::from(ws));
        }

        let claim_index = load_claims(&self.run_dir.join("claims"));
        let agent_index = build_agent_index(&tasks_raw);
        let task_nodes: Vec<TaskNode> = tasks_raw
            .iter()
            .map(|t| build_task_node(t, &agent_index, &claim_index, &roots))
            .collect();
        let stages = group_stages(&tasks_raw, &task_nodes);

        let mut status_counts: HashMap<String, usize> = HashMap::new();
        for node in &task_nodes {
            *status_counts.entry(node.status.clone()).or_default() += 1;
        }
        let last_heartbeat = task_nodes.iter().filter_map(|n| n.heartbeat_unix_ms).max();

        let run_status = derive_run_status(&tasks_raw, report.as_ref());
        let has_real_provider = tasks_raw
            .iter()
            .any(|t| get_str(t, "provider").as_deref() != Some("mock"));

        let run = RunMeta {
            run_id: get_str(&workflow, "run_id").unwrap_or_else(|| self.run_dir_name()),
            runtime: get_str(&workflow, "runtime").unwrap_or_else(|| "unknown".to_string()),
            state_schema_version: get_u64(&workflow, "state_schema_version"),
            created_unix_ms: get_u128(&workflow, "created_unix_ms"),
            status: run_status,
            workspace_root: sanitize_path_opt(workspace_root.as_deref(), &roots),
            heartbeat_interval_seconds: get_u64(&workflow, "heartbeat_interval_seconds"),
            global_max_concurrency: get_u64(&workflow, "global_max_concurrency"),
            provider_max_concurrency: get_u64_map(&workflow, "provider_max_concurrency"),
            task_count: get_u64(&workflow, "task_count")
                .map_or_else(|| tasks_raw.len(), |c| c as usize),
            observed_unix_ms: unix_ms(),
        };

        let summary = Summary {
            stage_count: stages.len(),
            task_status_counts: status_counts,
            has_real_provider,
            last_heartbeat_unix_ms: last_heartbeat,
            result_count: count_results(&self.run_dir.join("results")),
            report_status: report.as_ref().and_then(|r| get_str(r, "status")),
        };

        RunContract {
            schema_version: CONTRACT_SCHEMA_VERSION,
            read_only: true,
            run,
            summary,
            stages,
        }
    }

    fn run_dir_name(&self) -> String {
        self.run_dir
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_string()
    }

    fn read_tasks_raw(&self) -> Vec<Value> {
        let dir = self.run_dir.join("tasks");
        let mut paths: Vec<PathBuf> = match fs::read_dir(&dir) {
            Ok(rd) => rd.filter_map(|e| e.ok().map(|e| e.path())).collect(),
            Err(_) => return Vec::new(),
        };
        paths.sort();
        let mut out = Vec::new();
        for path in paths {
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            // Retry once: the snapshot may be mid atomic-rename, per STATE_CONTRACT.
            if let Some(v) = read_task_snapshot(&path) {
                out.push(v);
            }
        }
        out
    }

    /// Decode only complete, newly-appended `events.jsonl` records since the
    /// last call. An incomplete trailing line is left for the next call; a
    /// truncated/replaced file resets the offset.
    pub fn tail_events(&mut self, max: usize) -> Vec<EventRow> {
        let path = self.run_dir.join("events.jsonl");
        let bytes = match fs::read(&path) {
            Ok(b) => b,
            Err(_) => return Vec::new(),
        };
        if (bytes.len() as u64) < self.events_offset {
            self.events_offset = 0;
        }
        let start = self.events_offset as usize;
        let slice = &bytes[start..];
        let mut out = Vec::new();
        let mut consumed = 0usize;
        let mut line_start = 0usize;
        for (i, &b) in slice.iter().enumerate() {
            if b == b'\n' {
                let line = &slice[line_start..i];
                line_start = i + 1;
                consumed = line_start;
                if let Ok(s) = std::str::from_utf8(line) {
                    let trimmed = s.trim();
                    if !trimmed.is_empty() {
                        if let Ok(v) = serde_json::from_str::<Value>(trimmed) {
                            if out.len() < max {
                                out.push(EventRow::from_value(&v));
                            }
                        }
                    }
                }
            }
        }
        self.events_offset += consumed as u64;
        out
    }
}

/// Discover every run under `run_root` (metadata only). Active runs first.
pub fn list_runs(run_root: &Path) -> Vec<RunIndex> {
    let rd = match fs::read_dir(run_root) {
        Ok(rd) => rd,
        Err(_) => return Vec::new(),
    };
    let mut runs: Vec<RunIndex> = rd
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
        .filter_map(|e| {
            let dir = e.path();
            let wf = read_json(&dir.join("workflow.json")).unwrap_or(Value::Null);
            let tasks_dir = dir.join("tasks");
            let task_count = fs::read_dir(&tasks_dir)
                .map(|rd| {
                    rd.filter_map(|e| e.ok())
                        .filter(|e| e.path().extension().and_then(|x| x.to_str()) == Some("json"))
                        .count()
                })
                .unwrap_or(0);
            Some(RunIndex {
                run_id: get_str(&wf, "run_id")
                    .unwrap_or_else(|| e.file_name().to_string_lossy().into_owned()),
                runtime: get_str(&wf, "runtime").unwrap_or_else(|| "unknown".to_string()),
                created_unix_ms: get_u128(&wf, "created_unix_ms"),
                task_count,
                has_report: dir.join("report.json").exists() || dir.join("report-rs.json").exists(),
            })
        })
        .collect();
    runs.sort_by(|a, b| {
        b.has_report
            .cmp(&a.has_report)
            .then_with(|| b.created_unix_ms.cmp(&a.created_unix_ms))
    });
    runs
}

/// Last `MAX_LOG_BYTES` of a task's `worker.log`, loaded only on demand.
pub fn read_worker_log_tail(run_dir: &Path, task_id: &str) -> Option<String> {
    let path = run_dir.join("results").join(task_id).join("worker.log");
    let len = fs::metadata(&path).ok()?.len();
    let bytes = if len > MAX_LOG_BYTES {
        use std::io::{Read, Seek, SeekFrom};
        let mut file = fs::File::open(&path).ok()?;
        file.seek(SeekFrom::End(-(MAX_LOG_BYTES as i64))).ok()?;
        let mut buf = vec![0u8; MAX_LOG_BYTES as usize];
        file.read_exact(&mut buf).ok()?;
        buf
    } else {
        fs::read(&path).ok()?
    };
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

// ---------------------------------------------------------------------------
// Flattened virtualized view
// ---------------------------------------------------------------------------

/// A single renderable row of the task tree, cheap to compute and recompute.
#[derive(Clone, Debug)]
pub struct FlatRow {
    pub kind: RowKind,
    pub depth: usize,
    pub label: String,
    pub status: String,
    pub model: Option<String>,
    pub stale: bool,
    pub task_id: Option<String>,
    pub counts: HashMap<String, usize>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowKind {
    Stage,
    Task,
    Subagent,
}

/// Flatten `run -> stages -> tasks -> subagents` into virtualizable rows, stage
/// header first in each block. `filter` (case-insensitive substring) hides
/// non-matching tasks/subagents; a stage with no matches is hidden while
/// filtering, shown in full when the filter is empty.
pub fn flatten(contract: &RunContract, now_ms: u128, filter: &str) -> Vec<FlatRow> {
    let interval = contract.run.heartbeat_interval_seconds.unwrap_or(30);
    let needle = filter.trim().to_ascii_lowercase();
    let matches = |haystacks: &[&str]| -> bool {
        needle.is_empty()
            || haystacks
                .iter()
                .any(|h| h.to_ascii_lowercase().contains(&needle))
    };
    let mut rows = Vec::new();
    for stage in &contract.stages {
        let mut block = Vec::new();
        for task in &stage.tasks {
            if !matches(&[
                &task.task_id,
                task.source_id.as_deref().unwrap_or(""),
                &task.role,
                &task.status,
                task.model.as_deref().unwrap_or(""),
                task.provider.as_deref().unwrap_or(""),
            ]) {
                continue;
            }
            block.push(FlatRow {
                kind: RowKind::Task,
                depth: 1,
                label: task.task_id.clone(),
                status: task.status.clone(),
                model: task.model.clone(),
                stale: task.is_stale(now_ms, interval),
                task_id: Some(task.task_id.clone()),
                counts: HashMap::new(),
            });
            for sub in &task.subagents {
                if !matches(&[
                    &sub.agent_id,
                    sub.task_id.as_deref().unwrap_or(""),
                    &sub.status,
                    sub.model.as_deref().unwrap_or(""),
                ]) {
                    continue;
                }
                block.push(FlatRow {
                    kind: RowKind::Subagent,
                    depth: 2,
                    label: sub.agent_id.clone(),
                    status: sub.status.clone(),
                    model: sub.model.clone(),
                    stale: false,
                    task_id: sub.task_id.clone(),
                    counts: HashMap::new(),
                });
            }
        }
        if needle.is_empty() || !block.is_empty() {
            rows.push(FlatRow {
                kind: RowKind::Stage,
                depth: 0,
                label: stage.name.clone(),
                status: String::new(),
                model: None,
                stale: false,
                task_id: None,
                counts: stage.status_counts.clone(),
            });
            rows.extend(block);
        }
    }
    rows
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

pub fn unix_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

pub fn heartbeat_age_seconds(hb_ms: Option<u128>, now_ms: u128) -> Option<u64> {
    hb_ms.map(|hb| (now_ms.saturating_sub(hb) / 1000) as u64)
}

pub fn safe_run_id(value: &str) -> bool {
    if value.is_empty() || value.len() > 128 {
        return false;
    }
    let mut chars = value.chars();
    let first = chars.next().unwrap();
    if !first.is_ascii_alphanumeric() {
        return false;
    }
    value
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

fn read_json(path: &Path) -> Option<Value> {
    let text = fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

fn read_task_snapshot(path: &Path) -> Option<Value> {
    for attempt in 0..2u8 {
        if let Ok(text) = fs::read_to_string(path) {
            if let Ok(v) = serde_json::from_str::<Value>(&text) {
                return Some(v);
            }
        }
        if attempt == 0 {
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    None
}

fn get_str(v: &Value, key: &str) -> Option<String> {
    v.get(key)?.as_str().map(|s| s.to_string())
}

fn get_u64(v: &Value, key: &str) -> Option<u64> {
    v.get(key)?.as_u64()
}

fn get_u128(v: &Value, key: &str) -> Option<u128> {
    v.get(key)?.as_u64().map(u128::from)
}

fn get_u64_map(v: &Value, key: &str) -> HashMap<String, u64> {
    let mut out = HashMap::new();
    if let Some(obj) = v.get(key).and_then(Value::as_object) {
        for (k, val) in obj {
            if let Some(n) = val.as_u64() {
                out.insert(k.clone(), n);
            }
        }
    }
    out
}

fn load_claims(claims_dir: &Path) -> HashMap<String, Value> {
    let mut index = HashMap::new();
    let rd = match fs::read_dir(claims_dir) {
        Ok(rd) => rd,
        Err(_) => return index,
    };
    let mut paths: Vec<PathBuf> = rd
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("lock"))
        .collect();
    paths.sort();
    for path in paths {
        if let Some(v) = read_json(&path) {
            if v.is_object() {
                let task_id = get_str(&v, "task_id").unwrap_or_else(|| {
                    path.file_stem()
                        .and_then(|s| s.to_str())
                        .unwrap_or("")
                        .to_string()
                });
                index.insert(task_id, v);
            }
        }
    }
    index
}

fn build_agent_index(tasks: &[Value]) -> HashMap<String, Value> {
    let mut idx = HashMap::new();
    for task in tasks {
        let agent_id = get_str(task, "agent_id")
            .or_else(|| get_str(task, "source_id"))
            .or_else(|| get_str(task, "task_id"));
        if let Some(id) = agent_id {
            idx.insert(id, task.clone());
        }
    }
    idx
}

fn build_task_node(
    task: &Value,
    agent_index: &HashMap<String, Value>,
    claim_index: &HashMap<String, Value>,
    roots: &[PathBuf],
) -> TaskNode {
    let task_id = get_str(task, "task_id").unwrap_or_default();
    let claim = claim_index.get(&task_id);
    let owner = claim.and_then(|c| get_str(c, "owner"));

    let mut subagents = Vec::new();
    if let Some(arr) = task.get("subagents").and_then(Value::as_array) {
        for child_id in arr {
            if let Some(id) = child_id.as_str() {
                let child = agent_index.get(id);
                subagents.push(SubagentNode {
                    agent_id: id.to_string(),
                    task_id: child.and_then(|c| get_str(c, "task_id")),
                    status: child
                        .and_then(|c| get_str(c, "status"))
                        .unwrap_or_else(|| "unknown".to_string()),
                    model: child.and_then(|c| get_str(c, "model")),
                });
            }
        }
    }

    let mut artifacts = Vec::new();
    if let Some(arr) = task.get("artifacts").and_then(Value::as_array) {
        for a in arr {
            let raw = match a {
                Value::String(s) => s.clone(),
                other => other.to_string(),
            };
            artifacts.push(sanitize_path_opt(Some(&raw), roots).unwrap_or(raw));
        }
    }

    TaskNode {
        agent: AgentNode {
            agent_id: get_str(task, "agent_id")
                .or_else(|| get_str(task, "source_id"))
                .unwrap_or_else(|| task_id.clone()),
            owner,
            claimed_at: claim.and_then(|c| get_str(c, "claimed_at")),
            heartbeat_at: claim.and_then(|c| get_str(c, "heartbeat_at")),
        },
        task_id: task_id.clone(),
        index: get_u64(task, "index").unwrap_or(0) as usize,
        role: get_str(task, "role").unwrap_or_else(|| "general".to_string()),
        source_id: get_str(task, "source_id"),
        parent_task_id: get_str(task, "parent_task_id"),
        status: get_str(task, "status").unwrap_or_else(|| "pending".to_string()),
        attempts: get_u64(task, "attempts").unwrap_or(0) as usize,
        model: get_str(task, "model"),
        provider: get_str(task, "provider"),
        route: get_str(task, "route"),
        wrapper: get_str(task, "wrapper"),
        variant: get_str(task, "variant"),
        subagents,
        provider_subagents: task
            .get("provider_subagents")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default(),
        provider_subagent_visibility: get_str(task, "provider_subagent_visibility")
            .unwrap_or_else(|| "not_reported".to_string()),
        started_at: get_str(task, "started_at"),
        ended_at: get_str(task, "ended_at"),
        heartbeat_unix_ms: get_u128(task, "heartbeat_unix_ms"),
        needs: task
            .get("needs")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default(),
        artifacts,
        error: sanitize_error(task.get("error")),
    }
}

fn group_stages(tasks_raw: &[Value], task_nodes: &[TaskNode]) -> Vec<StageNode> {
    let node_by_id: HashMap<String, &TaskNode> =
        task_nodes.iter().map(|n| (n.task_id.clone(), n)).collect();
    let mut sorted: Vec<&Value> = tasks_raw.iter().collect();
    sorted.sort_by_key(|t| {
        (
            get_u64(t, "index").unwrap_or(0),
            get_str(t, "task_id").unwrap_or_default(),
        )
    });
    let mut stages: Vec<StageNode> = Vec::new();
    for raw in sorted {
        let task_id = get_str(raw, "task_id").unwrap_or_default();
        let node = match node_by_id.get(&task_id) {
            Some(n) => n,
            None => continue,
        };
        let stage_name = get_str(raw, "stage").unwrap_or_else(|| "Unnamed".to_string());
        let needs_new = stages.last().map_or(true, |s| s.name != stage_name);
        if needs_new {
            stages.push(StageNode {
                name: stage_name.clone(),
                status_counts: HashMap::new(),
                tasks: Vec::new(),
            });
        }
        let s = stages.last_mut().unwrap();
        *s.status_counts.entry(node.status.clone()).or_default() += 1;
        s.tasks.push((*node).clone());
    }
    stages
}

fn derive_run_status(tasks_raw: &[Value], report: Option<&Value>) -> RunStatus {
    if let Some(status) = report.and_then(|r| get_str(r, "status")) {
        return match status.as_str() {
            "completed" => RunStatus::Completed,
            "failed" => RunStatus::Failed,
            "planned" => RunStatus::Partial,
            _ => RunStatus::Partial,
        };
    }
    if tasks_raw.is_empty() {
        return RunStatus::Empty;
    }
    let any_running = tasks_raw.iter().any(|t| {
        matches!(
            get_str(t, "status").as_deref(),
            Some("in_progress") | Some("queued")
        )
    });
    if any_running {
        return RunStatus::Running;
    }
    let all_completed = tasks_raw
        .iter()
        .all(|t| get_str(t, "status").as_deref() == Some("completed"));
    if all_completed {
        return RunStatus::Completed;
    }
    let any_failed = tasks_raw
        .iter()
        .any(|t| get_str(t, "status").as_deref() == Some("failed"));
    if any_failed {
        return RunStatus::Failed;
    }
    RunStatus::Partial
}

fn count_results(results_dir: &Path) -> usize {
    let rd = match fs::read_dir(results_dir) {
        Ok(rd) => rd,
        Err(_) => return 0,
    };
    rd.filter_map(|e| e.ok())
        .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
        .filter(|e| {
            e.path().join("result.json").exists() || e.path().join("result-rs.json").exists()
        })
        .count()
}

/// Relativize a path against known roots; foreign absolute paths collapse to
/// their basename, matching SWARM_UI_CONTRACT sanitization.
pub fn sanitize_path(value: &str, roots: &[PathBuf]) -> Option<String> {
    sanitize_path_opt(Some(value), roots)
}

fn sanitize_path_opt(value: Option<&str>, roots: &[PathBuf]) -> Option<String> {
    let trimmed = value?.trim();
    if trimmed.is_empty() {
        return None;
    }
    let path = PathBuf::from(trimmed);
    for root in roots {
        if let Ok(rel) = path.strip_prefix(root) {
            return Some(rel.to_string_lossy().replace('\\', "/"));
        }
    }
    if !path.is_absolute() {
        return Some(trimmed.replace('\\', "/"));
    }
    path.file_name().and_then(|n| n.to_str()).map(String::from)
}

/// Cap length and scrub secret-like substrings from an error string.
pub fn sanitize_error(value: Option<&Value>) -> Option<String> {
    let text = match value? {
        Value::String(s) => s.clone(),
        Value::Null => return None,
        other => other.to_string(),
    };
    if text.is_empty() {
        return None;
    }
    let mut out = text.replace('\\', "/");
    if out.len() > MAX_ERROR_CHARS {
        out.truncate(MAX_ERROR_CHARS);
        out.push_str("...[truncated]");
    }
    Some(redact(&out))
}

fn redact(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let n = chars.len();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < n {
        if matches_at(&chars, i, &['s', 'k', '-']) {
            out.push_str("sk-");
            i += 3;
            let mut kept = 0usize;
            while i < n && kept < 6 && is_token_char(chars[i]) {
                out.push(chars[i]);
                i += 1;
                kept += 1;
            }
            out.push_str("***");
            while i < n && is_token_char(chars[i]) {
                i += 1;
            }
            continue;
        }
        if matches_ci_at(&chars, i, &['b', 'e', 'a', 'r', 'e', 'r', ' ']) {
            out.push_str("Bearer ***");
            i += 7;
            while i < n && chars[i] != ' ' && chars[i] != '\n' {
                i += 1;
            }
            continue;
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

fn matches_at(chars: &[char], i: usize, pat: &[char]) -> bool {
    pat.iter()
        .enumerate()
        .all(|(k, &c)| chars.get(i + k) == Some(&c))
}

fn matches_ci_at(chars: &[char], i: usize, pat: &[char]) -> bool {
    pat.iter().enumerate().all(|(k, &c)| {
        chars
            .get(i + k)
            .map(|x| x.eq_ignore_ascii_case(&c))
            .unwrap_or(false)
    })
}

fn is_token_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '_' | '-')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_run_id_rules() {
        assert!(safe_run_id("windows-linux_macos.1"));
        assert!(safe_run_id("a"));
        assert!(!safe_run_id(""));
        assert!(!safe_run_id("../escape"));
        assert!(!safe_run_id(&"9".repeat(129)));
        assert!(!safe_run_id("has space"));
    }

    #[test]
    fn error_sanitization_caps_and_redacts() {
        let long = "x".repeat(MAX_ERROR_CHARS + 50);
        let s = sanitize_error(Some(&Value::String(long))).unwrap();
        assert!(s.ends_with("...[truncated]"));

        let secret = sanitize_error(Some(&Value::String(
            "Bearer abcdef123 sk-1234567890xyz".to_string(),
        )))
        .unwrap();
        assert!(!secret.contains("abcdef123"));
        assert!(!secret.contains("xyz"));
        assert!(secret.contains("Bearer"));
        assert!(secret.contains("sk-"));
        assert!(secret.contains("***"));
    }

    #[test]
    fn path_sanitization_relativizes() {
        let root = PathBuf::from("/repo");
        assert_eq!(
            sanitize_path("/repo/docs/x.md", &[root.clone()]).unwrap(),
            "docs/x.md"
        );
        assert_eq!(
            sanitize_path("/tmp/foreign.log", &[root]).unwrap(),
            "foreign.log"
        );
        assert_eq!(sanitize_path("docs/y.md", &[]).unwrap(), "docs/y.md");
    }

    #[test]
    fn derive_status_covers_cases() {
        assert_eq!(
            derive_run_status(&[], Some(&json!({"status": "completed"}))),
            RunStatus::Completed
        );
        assert_eq!(derive_run_status(&[], None), RunStatus::Empty);
        assert_eq!(
            derive_run_status(&[json!({"status": "completed"})], None),
            RunStatus::Completed
        );
        assert_eq!(
            derive_run_status(&[json!({"status": "in_progress"})], None),
            RunStatus::Running
        );
        assert_eq!(
            derive_run_status(&[json!({"status": "failed"})], None),
            RunStatus::Failed
        );
        assert_eq!(
            derive_run_status(&[json!({"status": "blocked"})], None),
            RunStatus::Partial
        );
    }

    #[test]
    fn flatten_puts_stage_header_before_tasks() {
        let mut contract = RunContract::default();
        contract.run.heartbeat_interval_seconds = Some(30);
        contract.stages.push(StageNode {
            name: "Build".into(),
            status_counts: HashMap::new(),
            tasks: vec![TaskNode {
                task_id: "0001-a".into(),
                status: "in_progress".into(),
                model: Some("glm".into()),
                heartbeat_unix_ms: Some(unix_ms()),
                ..Default::default()
            }],
        });
        let rows = flatten(&contract, unix_ms(), "");
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].kind, RowKind::Stage);
        assert_eq!(rows[0].label, "Build");
        assert_eq!(rows[1].kind, RowKind::Task);
        assert_eq!(rows[1].task_id.as_deref(), Some("0001-a"));
    }

    #[test]
    fn stale_detection_uses_heartbeat_interval() {
        let now = 10_000_000u128;
        let mut task = TaskNode {
            status: "in_progress".into(),
            heartbeat_unix_ms: Some(now - 5_000),
            ..Default::default()
        };
        assert!(task.is_stale(now, 1));
        assert!(!task.is_stale(now, 30));
        task.status = "completed".into();
        assert!(!task.is_stale(now, 1));
    }
}

// ===========================================================================
// Feature-gated native UI. Everything below pulls in egui/eframe and is only
// compiled for the `swarms-ui` binary (requires --features ui-egui).
// ===========================================================================
#[cfg(feature = "ui-egui")]
pub mod ui_egui {
    use crate::*;
    use eframe::egui;
    use std::time::Instant;

    const ROW_HEIGHT: f32 = 20.0;
    const POLL_ACTIVE: Duration = Duration::from_millis(500);
    const POLL_IDLE: Duration = Duration::from_millis(2000);
    const MAX_EVENTS: usize = 500;

    pub struct ObservabilityApp {
        run_root: PathBuf,
        runs: Vec<RunIndex>,
        active_run_id: Option<String>,
        reader: Option<RunReader>,
        contract: Option<RunContract>,
        events: Vec<EventRow>,
        selected_task: Option<String>,
        log_for: Option<String>,
        log_text: Option<String>,
        last_poll: Option<Instant>,
        error: Option<String>,
        filter: String,
        ready_file: Option<PathBuf>,
        ready_written: bool,
        bench_until: Option<Instant>,
    }

    impl ObservabilityApp {
        pub fn new(
            run_root: PathBuf,
            active_run_id: Option<String>,
            ready_file: Option<PathBuf>,
            bench_duration_secs: Option<u64>,
        ) -> Self {
            ObservabilityApp {
                run_root,
                runs: Vec::new(),
                active_run_id,
                reader: None,
                contract: None,
                events: Vec::new(),
                selected_task: None,
                log_for: None,
                log_text: None,
                last_poll: None,
                error: None,
                filter: String::new(),
                ready_file,
                ready_written: false,
                bench_until: bench_duration_secs.map(|s| Instant::now() + Duration::from_secs(s)),
            }
        }

        fn activate(&mut self, run_id: String) {
            if self.active_run_id.as_ref() == Some(&run_id) && self.reader.is_some() {
                return;
            }
            self.active_run_id = Some(run_id.clone());
            self.events.clear();
            self.events_offset_reset();
            self.contract = None;
            self.selected_task = None;
            self.log_for = None;
            self.log_text = None;
            match RunReader::open(&self.run_root, &run_id, Vec::new()) {
                Ok(mut reader) => {
                    if reader.exists() {
                        self.error = None;
                        self.contract = Some(reader.read());
                    } else {
                        self.error = Some(format!("run not found: {run_id}"));
                    }
                    self.reader = Some(reader);
                }
                Err(e) => {
                    self.reader = None;
                    self.error = Some(e);
                }
            }
            self.last_poll = Some(Instant::now());
        }

        fn events_offset_reset(&mut self) {
            // SWARMS-UI: handled inside RunReader on truncation; nothing else.
        }

        fn poll_if_due(&mut self, now_ms: u128) -> bool {
            let reader = match self.reader.as_mut() {
                Some(r) if r.exists() => r,
                _ => return false,
            };
            let due = self.last_poll.map_or(true, |t| t.elapsed() >= POLL_IDLE);
            if !due {
                return false;
            }
            self.contract = Some(reader.read());
            let mut new_events = reader.tail_events(MAX_EVENTS);
            self.events.append(&mut new_events);
            if self.events.len() > MAX_EVENTS * 2 {
                let drop = self.events.len() - MAX_EVENTS;
                self.events.drain(0..drop);
            }
            self.last_poll = Some(Instant::now());
            let _ = now_ms;
            true
        }

        fn poll_interval(&self) -> Duration {
            let active = self
                .contract
                .as_ref()
                .map(|c| {
                    c.run.status == RunStatus::Running
                        || c.summary
                            .task_status_counts
                            .get("in_progress")
                            .copied()
                            .unwrap_or(0)
                            > 0
                })
                .unwrap_or(false);
            if active {
                POLL_ACTIVE
            } else {
                POLL_IDLE
            }
        }

        fn ensure_log(&mut self) {
            let task_id = match self.selected_task.clone() {
                Some(t) => Some(t),
                None => return,
            };
            if self.log_for.as_ref() == task_id.as_ref() {
                return;
            }
            let reader = match &self.reader {
                Some(r) => r,
                None => return,
            };
            self.log_text =
                read_worker_log_tail(reader.run_dir(), task_id.as_deref().unwrap_or(""));
            self.log_for = task_id;
        }

        fn write_ready(&mut self) {
            if self.ready_written {
                return;
            }
            if let Some(path) = &self.ready_file {
                let payload = format!(
                    "{{\"ready\":true,\"run_id\":{:?},\"time_unix_ms\":{}}}\n",
                    self.active_run_id.as_deref().unwrap_or(""),
                    unix_ms()
                );
                let _ = write_atomic(path, payload.as_bytes());
            }
            self.ready_written = true;
        }
    }

    fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let tmp = path.with_extension("ready.tmp");
        std::fs::write(&tmp, bytes)?;
        std::fs::rename(&tmp, path)
    }

    impl eframe::App for ObservabilityApp {
        fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
            self.write_ready();

            // Benchmark auto-exit.
            if let Some(until) = self.bench_until {
                if Instant::now() >= until {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    return;
                }
            }

            // Refresh the run list cheaply on each poll.
            let now_ms = unix_ms();
            let _changed = self.poll_if_due(now_ms);
            if self.runs.is_empty() {
                self.runs = list_runs(&self.run_root);
            }

            // Bottom: counters and reader state.
            egui::TopBottomPanel::bottom("footer").show(ctx, |ui| {
                let contract = self.contract.as_ref();
                let status = contract.map_or(RunStatus::Loading, |c| c.run.status);
                ui.horizontal(|ui| {
                    ui.label(format!("status: {}", status.label()));
                    if let Some(c) = contract {
                        ui.separator();
                        ui.label(format!("stages: {}", c.summary.stage_count));
                        ui.label(format!("tasks: {}", c.run.task_count));
                        ui.label(format!("results: {}", c.summary.result_count));
                        if let Some(gmc) = c.run.global_max_concurrency {
                            ui.separator();
                            ui.label(format!("global_max_concurrency: {gmc}"));
                        }
                        if !c.run.provider_max_concurrency.is_empty() {
                            let caps: Vec<String> = c
                                .run
                                .provider_max_concurrency
                                .iter()
                                .map(|(k, v)| format!("{k}={v}"))
                                .collect();
                            ui.label(format!("caps: {}", caps.join(", ")));
                        }
                        ui.separator();
                        ui.label(format!("events: {}", self.events.len()));
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label("read-only observer");
                    });
                });
            });

            // Left: discovered runs.
            egui::SidePanel::left("runs")
                .resizable(true)
                .default_width(240.0)
                .show(ctx, |ui| {
                    ui.heading("Runs");
                    ui.label(
                        egui::RichText::new(format!("{}", self.run_root.display()))
                            .small()
                            .color(egui::Color32::GRAY),
                    );
                    ui.separator();
                    let runs = self.runs.clone();
                    let mut to_activate: Option<String> = None;
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        if runs.is_empty() {
                            ui.label(
                                egui::RichText::new("no runs found").color(egui::Color32::GRAY),
                            );
                        }
                        for run in &runs {
                            let selected =
                                self.active_run_id.as_deref() == Some(run.run_id.as_str());
                            let dot = if run.has_report { "●" } else { "○" };
                            let text = format!(
                                "{} {}  [{} · {} tasks]",
                                dot, run.run_id, run.runtime, run.task_count
                            );
                            if ui.selectable_label(selected, text).clicked() {
                                to_activate = Some(run.run_id.clone());
                            }
                        }
                    });
                    if let Some(id) = to_activate {
                        self.activate(id);
                        ctx.request_repaint();
                    }
                });

            // Right: selected task detail.
            egui::SidePanel::right("detail")
                .resizable(true)
                .default_width(380.0)
                .show(ctx, |ui| {
                    self.render_detail(ui, now_ms);
                });

            // Center: virtualized task tree.
            egui::CentralPanel::default().show(ctx, |ui| {
                self.render_tree(ui, now_ms);
            });

            // On-demand repaint: never run a fixed 60 FPS loop.
            ctx.request_repaint_after(self.poll_interval());
        }
    }

    impl ObservabilityApp {
        fn render_tree(&mut self, ui: &mut egui::Ui, now_ms: u128) {
            ui.horizontal(|ui| {
                ui.label("Tasks");
                ui.text_edit_singleline(&mut self.filter);
                if ui.button("clear").clicked() {
                    self.filter.clear();
                }
                if let Some(err) = &self.error {
                    ui.label(egui::RichText::new(err).color(egui::Color32::from_rgb(220, 80, 80)));
                }
            });
            ui.separator();

            let contract = match self.contract.as_ref() {
                Some(c) => c,
                None => {
                    ui.label(egui::RichText::new("loading…").color(egui::Color32::GRAY));
                    return;
                }
            };
            if let Some(err) = &self.error {
                ui.vertical_centered(|ui| {
                    ui.label(
                        egui::RichText::new(format!("error: {err}"))
                            .color(egui::Color32::from_rgb(220, 80, 80)),
                    );
                });
                return;
            }
            if contract.run.status == RunStatus::Empty {
                ui.vertical_centered(|ui| {
                    ui.label(
                        egui::RichText::new("run has no task snapshots yet")
                            .color(egui::Color32::GRAY),
                    );
                });
                return;
            }

            let rows = flatten(contract, now_ms, &self.filter);
            let total = rows.len();
            let selected = self.selected_task.clone();
            let mut new_selection: Option<String> = None;
            egui::ScrollArea::vertical()
                .auto_shrink([false; 2])
                .show_rows(ui, ROW_HEIGHT, total, |ui, range| {
                    for idx in range {
                        let row = &rows[idx];
                        let indent = row.depth as f32 * 16.0;
                        let response = match row.kind {
                            RowKind::Stage => {
                                let counts = format_counts(&row.counts);
                                ui.horizontal(|ui| {
                                    ui.add_space(indent);
                                    ui.label(
                                        egui::RichText::new(&row.label)
                                            .strong()
                                            .color(egui::Color32::from_rgb(180, 200, 230)),
                                    );
                                    ui.label(
                                        egui::RichText::new(counts)
                                            .small()
                                            .color(egui::Color32::GRAY),
                                    );
                                })
                                .response
                            }
                            _ => {
                                let is_sel = selected.as_deref() == row.task_id.as_deref();
                                let color = status_color(&row.status, row.stale);
                                let label = format!(
                                    "{} {}",
                                    &row.label,
                                    row.model.as_deref().unwrap_or("")
                                );
                                let mut rich = egui::RichText::new(&label);
                                if is_sel {
                                    rich = rich.strong();
                                }
                                let resp = ui
                                    .horizontal(|ui| {
                                        ui.add_space(indent);
                                        ui.label(egui::RichText::new("●").color(color));
                                        if row.stale {
                                            ui.label(
                                                egui::RichText::new("stale")
                                                    .small()
                                                    .color(egui::Color32::from_rgb(190, 130, 220)),
                                            );
                                        }
                                        ui.selectable_label(is_sel, rich);
                                    })
                                    .response;
                                if resp.clicked() {
                                    new_selection = row.task_id.clone();
                                }
                                resp
                            }
                        };
                        let _ = response;
                    }
                });
            if let Some(id) = new_selection {
                self.selected_task = Some(id);
                ctx_request(ui.ctx());
            }
        }

        fn render_detail(&mut self, ui: &mut egui::Ui, now_ms: u128) {
            ui.heading("Detail");
            let task_id = match self.selected_task.clone() {
                Some(t) => t,
                None => {
                    ui.label(
                        egui::RichText::new("select a task to see its detail")
                            .color(egui::Color32::GRAY),
                    );
                    return;
                }
            };
            let interval = self
                .contract
                .as_ref()
                .and_then(|c| c.run.heartbeat_interval_seconds)
                .unwrap_or(30);
            // Find the node by task_id across all stages.
            let node = self.contract.as_ref().and_then(|c| {
                c.stages
                    .iter()
                    .flat_map(|s| s.tasks.iter())
                    .find(|t| t.task_id == task_id)
            });
            let node = match node {
                Some(n) => n.clone(),
                None => {
                    ui.label(
                        egui::RichText::new("task not present in current snapshot")
                            .color(egui::Color32::GRAY),
                    );
                    return;
                }
            };

            let kv = |ui: &mut egui::Ui, k: &str, v: &str| {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(k).color(egui::Color32::GRAY));
                    ui.label(v);
                });
            };
            kv(ui, "task_id", &node.task_id);
            if let Some(s) = &node.source_id {
                kv(ui, "source_id", s);
            }
            kv(ui, "role", &node.role);
            kv(ui, "status", &node.status);
            kv(ui, "provider", node.provider.as_deref().unwrap_or("—"));
            kv(ui, "model", node.model.as_deref().unwrap_or("—"));
            if let Some(w) = &node.wrapper {
                kv(ui, "wrapper", w);
            }
            kv(ui, "attempts", &node.attempts.to_string());
            if let Some(hb) = node.heartbeat_unix_ms {
                let age = heartbeat_age_seconds(Some(hb), now_ms).unwrap_or(0);
                kv(ui, "heartbeat", &format!("{}s ago", age));
                if node.is_stale(now_ms, interval) {
                    ui.label(
                        egui::RichText::new("STALE").color(egui::Color32::from_rgb(190, 130, 220)),
                    );
                }
            }
            kv(ui, "agent_id", &node.agent.agent_id);
            if let Some(o) = &node.agent.owner {
                kv(ui, "owner", o);
            }
            if !node.needs.is_empty() {
                kv(ui, "needs", &node.needs.join(", "));
            }
            if let Some(e) = &node.error {
                ui.separator();
                ui.label(egui::RichText::new("error").color(egui::Color32::from_rgb(220, 80, 80)));
                ui.label(egui::RichText::new(e).monospace());
            }

            if !node.subagents.is_empty() {
                ui.separator();
                ui.label("subagents:");
                for sub in &node.subagents {
                    ui.label(format!(
                        "  {} — {} — {}",
                        sub.agent_id,
                        sub.status,
                        sub.model.as_deref().unwrap_or("—")
                    ));
                }
            }
            if !node.artifacts.is_empty() {
                ui.separator();
                ui.label("artifacts:");
                for a in &node.artifacts {
                    ui.label(format!("  {a}"));
                }
            }

            // Worker log tail (loaded only for the selected task, 2 MiB cap).
            ui.separator();
            ui.label(format!("worker.log (cap 2 MiB):"));
            self.ensure_log();
            match &self.log_text {
                Some(log) => {
                    egui::ScrollArea::vertical()
                        .id_source("worker_log")
                        .max_height(220.0)
                        .show(ui, |ui| {
                            ui.label(egui::RichText::new(log).monospace().small());
                        });
                }
                None => ui.label(egui::RichText::new("no worker.log").color(egui::Color32::GRAY)),
            }
        }
    }

    fn ctx_request(ctx: &egui::Context) {
        ctx.request_repaint();
    }

    fn format_counts(counts: &HashMap<String, usize>) -> String {
        let mut entries: Vec<(&String, &usize)> = counts.iter().collect();
        entries.sort_by(|a, b| b.1.cmp(a.1));
        entries
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join(" ")
    }

    fn status_color(status: &str, stale: bool) -> egui::Color32 {
        if stale {
            return egui::Color32::from_rgb(190, 130, 220);
        }
        match status {
            "completed" => egui::Color32::from_rgb(90, 190, 110),
            "in_progress" => egui::Color32::from_rgb(90, 160, 230),
            "failed" => egui::Color32::from_rgb(220, 80, 80),
            "queued" => egui::Color32::from_rgb(150, 150, 150),
            "blocked" => egui::Color32::from_rgb(230, 170, 70),
            _ => egui::Color32::from_rgb(150, 150, 150),
        }
    }

    /// Parse the minimal CLI: --run-root, --run-id, --ready-file,
    /// --bench-duration. All optional.
    pub fn parse_args() -> (PathBuf, Option<String>, Option<PathBuf>, Option<u64>) {
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let mut run_root = cwd.join(".agent/swarm/runs");
        let mut run_id: Option<String> = None;
        let mut ready_file: Option<PathBuf> = None;
        let mut bench: Option<u64> = None;
        let mut args = std::env::args().skip(1);
        while let Some(a) = args.next() {
            match a.as_str() {
                "--run-root" => run_root = PathBuf::from(args.next().unwrap_or_default()),
                "--run-id" => run_id = args.next(),
                "--ready-file" => ready_file = args.next().map(PathBuf::from),
                "--bench-duration" => bench = args.next().and_then(|s| s.parse().ok()),
                _ => {}
            }
        }
        (run_root, run_id, ready_file, bench)
    }

    pub fn run() -> eframe::Result {
        let (run_root, run_id, ready_file, bench) = parse_args();
        let app = ObservabilityApp::new(run_root.clone(), run_id.clone(), ready_file, bench);
        // Pre-select an active run if requested or if there is exactly one.
        let initial = app.active_run_id.clone().or_else(|| {
            let runs = list_runs(&run_root);
            if runs.len() == 1 {
                Some(runs[0].run_id.clone())
            } else {
                None
            }
        });
        let app = ObservabilityApp::new(run_root, initial, None, bench);
        let options = eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default()
                .with_inner_size([1200.0, 800.0])
                .with_title("SWARMS observer (read-only)"),
            ..Default::default()
        };
        eframe::run_native(
            "SWARMS observer",
            options,
            Box::new(move |cc| {
                let _ = cc;
                Ok(Box::new(app))
            }),
        )
    }
}
