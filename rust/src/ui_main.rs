//! SWARMS native runtime console.
//!
//! This file is the root of two compilation units of the `swarms-runtime`
//! package:
//!
//! * the `swarms_runtime::ui` module (always compiled, serde + std only): a pure,
//!   testable, read-only model of the on-disk run contract described in
//!   `docs/STATE_CONTRACT.md` and `docs/SWARM_UI_CONTRACT.md`;
//! * the `swarms-ui` binary (compiled only with the `ui-egui` feature): a
//!   native egui/eframe window that renders that contract.
//!
//! The UI never claims tasks or spawns workers. Writes happen only after an
//! explicit user action: steering, or project-local Skillshare init/sync.
//!
//! See `docs/SWARM_UI.md` for usage and the exact Windows toolchain blocker.

use serde_json::Value;
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub const CONTRACT_SCHEMA_VERSION: u64 = 1;
pub const MAX_ERROR_CHARS: usize = 1000;
/// SWARMS-UI: hard cap on resident worker.log bytes, per UI_RUNTIME_EVALUATION.
pub const MAX_LOG_BYTES: u64 = 256 * 1024;

// ---------------------------------------------------------------------------
// Model
// ---------------------------------------------------------------------------

/// Derived run status. `Loading` and `Error` are UI transient states; the rest
/// mirror `SWARM_UI_CONTRACT.md` run-status derivation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RunStatus {
    #[default]
    Empty,
    Loading,
    Running,
    Completed,
    Failed,
    Partial,
    Error,
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
    pub project_id: String,
    pub project_name: String,
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
        let payload = v.get("payload").unwrap_or(&Value::Null);
        EventRow {
            time_unix_ms: v
                .get("time_unix_ms")
                .and_then(Value::as_u64)
                .map(u128::from),
            event: get_str(v, "event").unwrap_or_default(),
            task_id: get_str(v, "task_id").or_else(|| get_str(payload, "task_id")),
            model: get_str(v, "model").or_else(|| get_str(payload, "model")),
            provider: get_str(v, "provider").or_else(|| get_str(payload, "provider")),
            error: sanitize_error(v.get("error").or_else(|| payload.get("error"))),
        }
    }
}

/// Compact index entry for a discovered run, for the left panel.
#[derive(Clone, Debug)]
pub struct RunIndex {
    pub run_id: String,
    pub project_id: String,
    pub project_name: String,
    pub runtime: String,
    pub created_unix_ms: Option<u128>,
    pub last_activity_unix_ms: Option<u128>,
    pub task_count: usize,
    pub has_report: bool,
}

#[derive(Clone, Debug)]
pub struct ProjectGroup {
    pub project_id: String,
    pub project_name: String,
    pub runs: Vec<RunIndex>,
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

        let (project_id, project_name) = project_meta(&workflow);
        let run = RunMeta {
            run_id: get_str(&workflow, "run_id").unwrap_or_else(|| self.run_dir_name()),
            project_id,
            project_name,
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
        use std::io::{Read, Seek, SeekFrom};
        let len = match fs::metadata(&path) {
            Ok(metadata) => metadata.len(),
            Err(_) => return Vec::new(),
        };
        if len < self.events_offset {
            self.events_offset = 0;
        }
        let mut file = match fs::File::open(&path) {
            Ok(file) => file,
            Err(_) => return Vec::new(),
        };
        if file.seek(SeekFrom::Start(self.events_offset)).is_err() {
            return Vec::new();
        }
        let mut bytes = Vec::with_capacity((len - self.events_offset).min(64 * 1024) as usize);
        if file.read_to_end(&mut bytes).is_err() {
            return Vec::new();
        }
        let slice = bytes.as_slice();
        let mut out = VecDeque::with_capacity(max);
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
                            if max > 0 {
                                if out.len() == max {
                                    out.pop_front();
                                }
                                out.push_back(EventRow::from_value(&v));
                            }
                        }
                    }
                }
            }
        }
        self.events_offset += consumed as u64;
        out.into()
    }
}

fn project_meta(workflow: &Value) -> (String, String) {
    if let Some(id) = get_str(workflow, "project_id").filter(|id| !id.trim().is_empty()) {
        let name = get_str(workflow, "project_name")
            .filter(|name| !name.trim().is_empty())
            .unwrap_or_else(|| id.clone());
        return (id, name.chars().take(80).collect());
    }
    if let Some(workspace) = get_str(workflow, "workspace_root") {
        let path = PathBuf::from(&workspace);
        let name = path
            .file_name()
            .and_then(|value| value.to_str())
            .filter(|value| !value.is_empty())
            .unwrap_or("Workspace")
            .to_string();
        return (format!("workspace:{}", workspace.to_lowercase()), name);
    }
    ("legacy".to_string(), "Legacy runs".to_string())
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
        .map(|e| {
            let dir = e.path();
            let wf = read_json(&dir.join("workflow.json")).unwrap_or(Value::Null);
            let tasks_dir = dir.join("tasks");
            let mut task_count = 0;
            let mut last_activity_unix_ms = [
                dir.join("workflow.json"),
                dir.join("events.jsonl"),
                dir.join("report.json"),
                dir.join("report-rs.json"),
            ]
            .iter()
            .filter_map(|path| modified_unix_ms(path))
            .max();
            if let Ok(entries) = fs::read_dir(&tasks_dir) {
                for entry in entries.filter_map(|entry| entry.ok()) {
                    if entry.path().extension().and_then(|value| value.to_str()) == Some("json") {
                        task_count += 1;
                        last_activity_unix_ms =
                            last_activity_unix_ms.max(modified_unix_ms(&entry.path()));
                    }
                }
            }
            let (project_id, project_name) = project_meta(&wf);
            let created_unix_ms = get_u128(&wf, "created_unix_ms");
            RunIndex {
                run_id: get_str(&wf, "run_id")
                    .unwrap_or_else(|| e.file_name().to_string_lossy().into_owned()),
                project_id,
                project_name,
                runtime: get_str(&wf, "runtime").unwrap_or_else(|| "unknown".to_string()),
                created_unix_ms,
                last_activity_unix_ms: last_activity_unix_ms.or(created_unix_ms),
                task_count,
                has_report: dir.join("report.json").exists() || dir.join("report-rs.json").exists(),
            }
        })
        .collect();
    runs.sort_by(|a, b| {
        a.has_report
            .cmp(&b.has_report)
            .then_with(|| b.last_activity_unix_ms.cmp(&a.last_activity_unix_ms))
    });
    runs
}

fn modified_unix_ms(path: &Path) -> Option<u128> {
    fs::metadata(path)
        .ok()?
        .modified()
        .ok()?
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|duration| duration.as_millis())
}

pub fn relative_age(timestamp_ms: Option<u128>, now_ms: u128) -> String {
    let Some(timestamp_ms) = timestamp_ms else {
        return "unknown".to_string();
    };
    let seconds = now_ms.saturating_sub(timestamp_ms) / 1000;
    match seconds {
        0..=59 => "now".to_string(),
        60..=3_599 => format!("{}m ago", seconds / 60),
        3_600..=86_399 => format!("{}h ago", seconds / 3_600),
        86_400..=604_799 => format!("{}d ago", seconds / 86_400),
        604_800..=2_629_799 => format!("{}w ago", seconds / 604_800),
        _ => format!("{}mo ago", seconds / 2_629_800),
    }
}

pub fn group_runs(runs: &[RunIndex]) -> Vec<ProjectGroup> {
    let mut groups: BTreeMap<(String, String), Vec<RunIndex>> = BTreeMap::new();
    for run in runs {
        groups
            .entry((run.project_name.to_lowercase(), run.project_id.clone()))
            .or_default()
            .push(run.clone());
    }
    groups
        .into_iter()
        .map(|((_, project_id), runs)| ProjectGroup {
            project_name: runs
                .first()
                .map(|run| run.project_name.clone())
                .unwrap_or_else(|| "Project".to_string()),
            project_id,
            runs,
        })
        .collect()
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
        let needs_new = stages.last().is_none_or(|s| s.name != stage_name);
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
    let cross_platform_absolute = path.is_absolute()
        || trimmed.starts_with('/')
        || trimmed.starts_with("\\\\")
        || trimmed.as_bytes().get(1) == Some(&b':');
    if !cross_platform_absolute {
        return Some(trimmed.replace('\\', "/"));
    }
    trimmed
        .rsplit(['/', '\\'])
        .find(|part| !part.is_empty())
        .map(String::from)
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
    if out.chars().count() > MAX_ERROR_CHARS {
        out = out.chars().take(MAX_ERROR_CHARS).collect();
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
        if (chars[i].is_ascii_alphabetic() || chars[i] == '_')
            && (i == 0 || !(chars[i - 1].is_ascii_alphanumeric() || chars[i - 1] == '_'))
        {
            let mut end = i + 1;
            while end < n && (chars[end].is_ascii_alphanumeric() || chars[end] == '_') {
                end += 1;
            }
            let key: String = chars[i..end]
                .iter()
                .collect::<String>()
                .to_ascii_lowercase();
            let sensitive = key == "api_key"
                || key.ends_with("_api_key")
                || key.ends_with("_token")
                || key.contains("secret")
                || key.contains("password");
            let mut sep = end;
            while sep < n && chars[sep].is_ascii_whitespace() && chars[sep] != '\n' {
                sep += 1;
            }
            if sensitive && sep < n && matches!(chars[sep], '=' | ':') {
                out.extend(chars[i..=sep].iter());
                out.push_str("***");
                i = sep + 1;
                while i < n && chars[i].is_ascii_whitespace() && chars[i] != '\n' {
                    i += 1;
                }
                let quote = chars.get(i).copied().filter(|c| matches!(c, '\'' | '"'));
                if quote.is_some() {
                    i += 1;
                }
                while i < n {
                    if quote.is_some_and(|q| chars[i] == q)
                        || (quote.is_none()
                            && (chars[i].is_ascii_whitespace()
                                || matches!(chars[i], ',' | ';' | '}' | ']')))
                    {
                        if quote.is_some() {
                            i += 1;
                        }
                        break;
                    }
                    i += 1;
                }
                continue;
            }
        }
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
    use serde_json::json;

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

        let assignments = sanitize_error(Some(&Value::String(
            "api_key=supersecret OPENAI_API_KEY='anothersecret' password: hidden".into(),
        )))
        .unwrap();
        assert!(!assignments.contains("supersecret"));
        assert!(!assignments.contains("anothersecret"));
        assert!(!assignments.contains("hidden"));

        let unicode = "🙂".repeat(MAX_ERROR_CHARS + 10);
        let capped = sanitize_error(Some(&Value::String(unicode))).unwrap();
        assert!(capped.ends_with("...[truncated]"));
    }

    #[test]
    fn path_sanitization_relativizes() {
        let root = PathBuf::from("/repo");
        assert_eq!(
            sanitize_path("/repo/docs/x.md", std::slice::from_ref(&root)).unwrap(),
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

    #[test]
    fn event_tail_keeps_only_the_newest_rows() {
        let root = std::env::temp_dir().join(format!("swarms-events-{}", unix_ms()));
        fs::create_dir_all(&root).unwrap();
        let payload = (0..8)
            .map(|id| format!("{{\"event\":\"tick\",\"payload\":{{\"task_id\":\"{id}\"}}}}\n"))
            .collect::<String>();
        fs::write(root.join("events.jsonl"), payload).unwrap();
        let mut reader = RunReader::new(&root, Vec::new());
        let events = reader.tail_events(3);
        assert_eq!(events.len(), 3);
        assert_eq!(events[0].task_id.as_deref(), Some("5"));
        assert_eq!(events[2].task_id.as_deref(), Some("7"));
        fs::remove_dir_all(root).ok();
    }
}

// ===========================================================================
// Feature-gated native UI. Everything below pulls in egui/eframe and is only
// compiled for the `swarms-ui` binary (requires --features ui-egui).
// ===========================================================================
#[cfg(feature = "ui-egui")]
pub mod ui_egui {
    use super::*;
    use crate::{config, quota, resources, steering};
    use eframe::egui;
    use std::time::Instant;

    const ROW_HEIGHT: f32 = 26.0;
    const POLL_ACTIVE: Duration = Duration::from_secs(1);
    const POLL_IDLE: Duration = Duration::from_secs(5);
    const RUN_LIST_POLL: Duration = Duration::from_secs(10);
    const QUOTA_POLL: Duration = Duration::from_secs(30);
    const MAX_EVENTS: usize = 500;

    fn accent() -> egui::Color32 {
        egui::Color32::from_rgb(128, 108, 255)
    }

    fn muted() -> egui::Color32 {
        egui::Color32::from_rgb(132, 135, 145)
    }

    fn apply_theme(ctx: &egui::Context) {
        let mut style = (*ctx.style()).clone();
        style.spacing.item_spacing = egui::vec2(8.0, 6.0);
        style.spacing.button_padding = egui::vec2(8.0, 4.0);
        style.visuals = egui::Visuals::dark();
        style.visuals.panel_fill = egui::Color32::from_rgb(14, 15, 18);
        style.visuals.window_fill = egui::Color32::from_rgb(17, 18, 22);
        style.visuals.extreme_bg_color = egui::Color32::from_rgb(9, 10, 12);
        style.visuals.faint_bg_color = egui::Color32::from_rgb(22, 24, 29);
        style.visuals.selection.bg_fill = egui::Color32::from_rgb(45, 42, 78);
        style.visuals.selection.stroke.color = egui::Color32::from_rgb(176, 166, 255);
        style.visuals.hyperlink_color = egui::Color32::from_rgb(176, 166, 255);
        style.visuals.widgets.inactive.weak_bg_fill = egui::Color32::from_rgb(22, 24, 29);
        style.visuals.widgets.hovered.weak_bg_fill = egui::Color32::from_rgb(31, 33, 40);
        ctx.set_style(style);
    }

    #[derive(Clone, Debug, PartialEq, Eq)]
    struct RunSignature {
        workflow: Option<(u64, SystemTime)>,
        tasks: Option<SystemTime>,
        claims: Option<SystemTime>,
        results: Option<SystemTime>,
        events: Option<(u64, SystemTime)>,
        report: Option<(u64, SystemTime)>,
    }

    impl RunSignature {
        fn read(run_dir: &Path) -> Self {
            Self {
                workflow: file_signature(&run_dir.join("workflow.json")),
                tasks: modified(&run_dir.join("tasks")),
                claims: modified(&run_dir.join("claims")),
                results: modified(&run_dir.join("results")),
                events: file_signature(&run_dir.join("events.jsonl")),
                report: file_signature(&run_dir.join("report.json"))
                    .or_else(|| file_signature(&run_dir.join("report-rs.json"))),
            }
        }
    }

    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    enum CenterView {
        #[default]
        Overview,
        Tasks,
        Activity,
        Resources,
    }

    fn modified(path: &Path) -> Option<SystemTime> {
        fs::metadata(path).and_then(|meta| meta.modified()).ok()
    }

    fn resource_kind_label(kind: resources::ResourceKind) -> &'static str {
        match kind {
            resources::ResourceKind::Instructions => "AGENTS",
            resources::ResourceKind::Skill => "SKILL",
            resources::ResourceKind::Mcp => "MCP",
        }
    }

    fn resource_scope_label(scope: resources::ResourceScope) -> &'static str {
        match scope {
            resources::ResourceScope::Project => "Project",
            resources::ResourceScope::Global => "Global",
        }
    }

    fn agent_label(agent: resources::AgentKind) -> &'static str {
        match agent {
            resources::AgentKind::Codex => "Codex",
            resources::AgentKind::Claude => "Claude",
            resources::AgentKind::Gemini => "Gemini",
            resources::AgentKind::OpenCode => "OpenCode",
            resources::AgentKind::Antigravity => "Antigravity",
            resources::AgentKind::Hermes => "Hermes",
            resources::AgentKind::Agy => "AGY",
        }
    }

    fn file_signature(path: &Path) -> Option<(u64, SystemTime)> {
        fs::metadata(path)
            .and_then(|meta| Ok((meta.len(), meta.modified()?)))
            .ok()
    }

    #[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
    #[serde(default)]
    struct UiPrivacyConfig {
        show_account_emails: bool,
        account_emails: BTreeMap<String, String>,
    }

    impl Default for UiPrivacyConfig {
        fn default() -> Self {
            Self {
                show_account_emails: true,
                account_emails: BTreeMap::new(),
            }
        }
    }

    fn ui_config_path(run_root: &Path) -> Option<PathBuf> {
        find_workspace_root(run_root).map(|root| root.join("config/swarm_ui.local.json"))
    }

    fn load_ui_config(run_root: &Path) -> UiPrivacyConfig {
        ui_config_path(run_root)
            .and_then(|path| fs::read_to_string(path).ok())
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    #[derive(serde::Deserialize)]
    struct QuotaIdentities {
        #[serde(default)]
        accounts: BTreeMap<String, String>,
    }

    fn load_quota_identities(snapshot_path: &Path) -> Option<BTreeMap<String, String>> {
        let path = snapshot_path.with_file_name("quota_identities.local.json");
        let text = fs::read_to_string(path).ok()?;
        serde_json::from_str::<QuotaIdentities>(&text)
            .ok()
            .map(|identities| identities.accounts)
    }

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
        rows: Vec<FlatRow>,
        rows_filter: String,
        rows_dirty: bool,
        signature: Option<RunSignature>,
        last_poll: Option<Instant>,
        last_runs_poll: Option<Instant>,
        error: Option<String>,
        filter: String,
        ready_file: Option<PathBuf>,
        ready_written: bool,
        bench_until: Option<Instant>,
        quota_snapshot: Option<quota::QuotaSnapshotView>,
        quota_error: Option<String>,
        last_quota_poll: Option<Instant>,
        steer_prompt: String,
        steer_feedback: Option<String>,
        center_view: CenterView,
        provider_icons: ProviderIcons,
        ui_privacy: UiPrivacyConfig,
        config_open: bool,
        config_feedback: Option<String>,
        resource_catalog: resources::ResourceCatalog,
        resource_scope: resources::ResourceScope,
        resource_kind: Option<resources::ResourceKind>,
        resource_filter: String,
        selected_resource: Option<String>,
        resource_root: PathBuf,
        resource_sync_feedback: Option<String>,
    }

    impl ObservabilityApp {
        pub fn new(
            run_root: PathBuf,
            active_run_id: Option<String>,
            ready_file: Option<PathBuf>,
            bench_duration_secs: Option<u64>,
        ) -> Self {
            let ui_privacy = load_ui_config(&run_root);
            let project_root = find_workspace_root(&run_root).unwrap_or_else(|| run_root.clone());
            let resource_catalog = resources::discover(&project_root);
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
                rows: Vec::new(),
                rows_filter: String::new(),
                rows_dirty: true,
                signature: None,
                last_poll: None,
                last_runs_poll: None,
                error: None,
                filter: String::new(),
                ready_file,
                ready_written: false,
                bench_until: bench_duration_secs.map(|s| Instant::now() + Duration::from_secs(s)),
                quota_snapshot: None,
                quota_error: None,
                last_quota_poll: None,
                steer_prompt: String::new(),
                steer_feedback: None,
                center_view: CenterView::Overview,
                provider_icons: ProviderIcons,
                ui_privacy,
                config_open: false,
                config_feedback: None,
                resource_catalog,
                resource_scope: resources::ResourceScope::Project,
                resource_kind: None,
                resource_filter: String::new(),
                selected_resource: None,
                resource_root: project_root,
                resource_sync_feedback: None,
            }
        }

        fn save_ui_config(&mut self) {
            let Some(path) = ui_config_path(&self.run_root) else {
                self.config_feedback = Some("No se encontró la raíz del proyecto".to_string());
                return;
            };
            let result = serde_json::to_string_pretty(&self.ui_privacy)
                .map_err(|error| error.to_string())
                .and_then(|text| fs::write(&path, text).map_err(|error| error.to_string()));
            self.config_feedback = Some(match result {
                Ok(()) => format!("Guardado en {}", path.display()),
                Err(error) => format!("No se pudo guardar: {error}"),
            });
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
            self.rows.clear();
            self.rows_dirty = true;
            match RunReader::open(&self.run_root, &run_id, Vec::new()) {
                Ok(mut reader) => {
                    if reader.exists() {
                        let project_root = read_json(&reader.run_dir().join("workflow.json"))
                            .and_then(|workflow| get_str(&workflow, "workspace_root"))
                            .map(PathBuf::from)
                            .filter(|path| path.is_dir());
                        self.error = None;
                        self.contract = Some(reader.read());
                        self.events = reader.tail_events(MAX_EVENTS);
                        if let Some(project_root) = project_root {
                            if self.resource_root != project_root {
                                self.resource_root = project_root;
                                self.selected_resource = None;
                                self.refresh_resources();
                            }
                        }
                    } else {
                        self.error = Some(format!("run not found: {run_id}"));
                    }
                    self.signature = Some(RunSignature::read(reader.run_dir()));
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
            let poll_interval = self.poll_interval();
            let reader = match self.reader.as_mut() {
                Some(r) if r.exists() => r,
                _ => return false,
            };
            let due = self.last_poll.is_none_or(|t| t.elapsed() >= poll_interval);
            if !due {
                return false;
            }
            let signature = RunSignature::read(reader.run_dir());
            let changed = self.signature.as_ref() != Some(&signature);
            if changed {
                self.contract = Some(reader.read());
                let mut new_events = reader.tail_events(MAX_EVENTS);
                self.events.append(&mut new_events);
                if self.events.len() > MAX_EVENTS {
                    let drop = self.events.len() - MAX_EVENTS;
                    self.events.drain(0..drop);
                }
                self.signature = Some(signature);
                self.rows_dirty = true;
                self.log_for = None;
            }
            self.last_poll = Some(Instant::now());
            if !changed
                && self.rows.iter().any(|row| {
                    matches!(row.status.as_str(), "in_progress" | "queued") && !row.stale
                })
            {
                let interval = self
                    .contract
                    .as_ref()
                    .and_then(|contract| contract.run.heartbeat_interval_seconds)
                    .unwrap_or(30);
                self.rows_dirty = self.contract.as_ref().is_some_and(|contract| {
                    heartbeat_age_seconds(contract.summary.last_heartbeat_unix_ms, now_ms)
                        .is_some_and(|age| age > interval)
                });
            }
            changed
        }

        fn refresh_runs_if_due(&mut self) {
            if self
                .last_runs_poll
                .is_none_or(|last| last.elapsed() >= RUN_LIST_POLL)
            {
                self.runs = list_runs(&self.run_root);
                self.last_runs_poll = Some(Instant::now());
            }
        }

        fn refresh_quotas_if_due(&mut self) {
            if self
                .last_quota_poll
                .is_some_and(|last| last.elapsed() < QUOTA_POLL)
            {
                return;
            }
            self.last_quota_poll = Some(Instant::now());
            let Some(root) = find_workspace_root(&self.run_root) else {
                self.quota_error = Some("workspace config not found".to_string());
                return;
            };
            match config::load_router(&root) {
                Ok(router) if router.quota_policy.enabled => {
                    let configured = Path::new(&router.quota_policy.snapshot_path);
                    let path = if configured.is_absolute() {
                        configured.to_path_buf()
                    } else {
                        root.join(configured)
                    };
                    match quota::load_snapshot_view(&path) {
                        Ok(snapshot) => {
                            self.quota_snapshot = Some(snapshot);
                            self.quota_error = None;
                            if let Some(accounts) = load_quota_identities(&path) {
                                self.ui_privacy.account_emails.extend(accounts);
                            }
                        }
                        Err(error) => self.quota_error = Some(error),
                    }
                }
                Ok(_) => self.quota_error = Some("quota tracking disabled".to_string()),
                Err(error) => self.quota_error = Some(error),
            }
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
            self.refresh_runs_if_due();
            self.refresh_quotas_if_due();

            egui::TopBottomPanel::top("app_header")
                .exact_height(46.0)
                .show(ctx, |ui| {
                    ui.horizontal_centered(|ui| {
                        ui.label(egui::RichText::new("SWARMS").strong().size(18.0));
                        ui.label(egui::RichText::new("RUNTIME").small().color(muted()));
                        if let Some(contract) = &self.contract {
                            ui.separator();
                            ui.label(
                                egui::RichText::new(&contract.run.project_name)
                                    .strong()
                                    .color(egui::Color32::from_rgb(220, 220, 226)),
                            );
                            ui.label(egui::RichText::new("/").color(muted()));
                            ui.label(egui::RichText::new(&contract.run.run_id).color(muted()));
                            ui.label(
                                egui::RichText::new(format!("● {}", contract.run.status.label()))
                                    .small()
                                    .color(status_color(contract.run.status.label(), false)),
                            );
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.button("Config").clicked() {
                                self.config_open = true;
                            }
                            ui.label(
                                egui::RichText::new("local  •  native Rust")
                                    .small()
                                    .color(muted()),
                            );
                        });
                    });
                });

            if self.config_open {
                let mut open = self.config_open;
                egui::Window::new("Configuración")
                    .open(&mut open)
                    .resizable(false)
                    .default_width(420.0)
                    .show(ctx, |ui| self.render_config(ui));
                self.config_open = open;
            }

            // Compact bottom status line.
            egui::TopBottomPanel::bottom("footer")
                .exact_height(38.0)
                .show(ctx, |ui| {
                    let contract = self.contract.as_ref();
                    let status = contract.map_or(RunStatus::Loading, |c| c.run.status);
                    ui.horizontal_centered(|ui| {
                        let status_color = status_color(status.label(), false);
                        let (dot, _) =
                            ui.allocate_exact_size(egui::vec2(8.0, 8.0), egui::Sense::hover());
                        ui.painter().circle_filled(dot.center(), 3.0, status_color);
                        ui.label(
                            egui::RichText::new(format!("status: {}", status.label()))
                                .small()
                                .color(status_color),
                        );
                        ui.separator();
                        self.render_quota_strip(ui);
                        if let Some(c) = contract {
                            ui.separator();
                            ui.label(
                                egui::RichText::new(format!(
                                    "{} stages · {} tasks · {} events",
                                    c.summary.stage_count,
                                    c.run.task_count,
                                    self.events.len()
                                ))
                                .small()
                                .color(muted()),
                            );
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(egui::RichText::new("local").small().color(muted()));
                        });
                    });
                });

            // Left: project -> run navigation.
            egui::SidePanel::left("runs")
                .resizable(true)
                .default_width(285.0)
                .show(ctx, |ui| {
                    ui.heading("Projects");
                    ui.label(
                        egui::RichText::new(format!("{}", self.run_root.display()))
                            .small()
                            .color(muted()),
                    );
                    ui.separator();
                    let runs = self.runs.clone();
                    let groups = group_runs(&runs);
                    let mut to_activate: Option<String> = None;
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        if groups.is_empty() {
                            ui.label(egui::RichText::new("No projects yet").color(muted()));
                        }
                        for group in &groups {
                            egui::CollapsingHeader::new(
                                egui::RichText::new(format!(
                                    "{}  {}",
                                    group.project_name,
                                    group.runs.len()
                                ))
                                .strong(),
                            )
                            .id_salt(&group.project_id)
                            .default_open(true)
                            .show(ui, |ui| {
                                for run in &group.runs {
                                    let selected =
                                        self.active_run_id.as_deref() == Some(run.run_id.as_str());
                                    let dot = if run.has_report { "●" } else { "○" };
                                    if ui
                                        .selectable_label(
                                            selected,
                                            format!("{dot}  {}", run.run_id),
                                        )
                                        .clicked()
                                    {
                                        to_activate = Some(run.run_id.clone());
                                    }
                                    ui.horizontal(|ui| {
                                        ui.add_space(22.0);
                                        ui.label(
                                            egui::RichText::new(format!(
                                                "{}  ·  {} tasks",
                                                run.runtime, run.task_count
                                            ))
                                            .small()
                                            .color(muted()),
                                        );
                                        ui.with_layout(
                                            egui::Layout::right_to_left(egui::Align::Center),
                                            |ui| {
                                                ui.label(
                                                    egui::RichText::new(relative_age(
                                                        run.last_activity_unix_ms,
                                                        unix_ms(),
                                                    ))
                                                    .small()
                                                    .color(muted()),
                                                )
                                                .on_hover_text("Last swarm activity");
                                            },
                                        );
                                    });
                                }
                            });
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
                self.render_center(ui, now_ms);
            });

            // On-demand repaint: never run a fixed 60 FPS loop.
            ctx.request_repaint_after(self.poll_interval());
        }
    }

    impl ObservabilityApp {
        fn render_center(&mut self, ui: &mut egui::Ui, now_ms: u128) {
            ui.horizontal(|ui| {
                for (view, label) in [
                    (CenterView::Overview, "Overview"),
                    (CenterView::Tasks, "Tasks"),
                    (CenterView::Activity, "Activity"),
                    (CenterView::Resources, "Resources"),
                ] {
                    if ui
                        .selectable_label(self.center_view == view, label)
                        .clicked()
                    {
                        self.center_view = view;
                    }
                }
            });
            ui.separator();
            match self.center_view {
                CenterView::Overview => self.render_overview(ui),
                CenterView::Tasks => self.render_tree(ui, now_ms),
                CenterView::Activity => self.render_activity(ui),
                CenterView::Resources => self.render_resources(ui),
            }
        }

        fn refresh_resources(&mut self) {
            self.resource_catalog = resources::discover(&self.resource_root);
        }

        fn sync_project_skills(&mut self) {
            if !self.resource_root.join(".skillshare/config.yaml").is_file() {
                self.resource_sync_feedback = Some(
                    "This project has no .skillshare/config.yaml; initialize project sharing first."
                        .to_string(),
                );
                return;
            }
            let result = std::process::Command::new("skillshare")
                .args(["sync", "-p", "--json"])
                .current_dir(&self.resource_root)
                .output();
            self.resource_sync_feedback = Some(match result {
                Ok(output) if output.status.success() => "Project skills synced".to_string(),
                Ok(output) => format!(
                    "Skill sync failed: {}",
                    String::from_utf8_lossy(&output.stderr)
                        .lines()
                        .next()
                        .unwrap_or("unknown error")
                ),
                Err(error) => format!("Skillshare unavailable: {error}"),
            });
            self.refresh_resources();
        }

        fn initialize_project_skills(&mut self) {
            let result = std::process::Command::new("skillshare")
                .args([
                    "init",
                    "-p",
                    "--targets",
                    "codex,gemini,antigravity,opencode",
                ])
                .current_dir(&self.resource_root)
                .output();
            self.resource_sync_feedback = Some(match result {
                Ok(output) if output.status.success() => {
                    "Project sharing enabled for Codex, Gemini, Antigravity, and OpenCode"
                        .to_string()
                }
                Ok(output) => format!(
                    "Skillshare init failed: {}",
                    String::from_utf8_lossy(&output.stderr)
                        .lines()
                        .next()
                        .unwrap_or("unknown error")
                ),
                Err(error) => format!("Skillshare unavailable: {error}"),
            });
            self.refresh_resources();
        }

        fn render_resources(&mut self, ui: &mut egui::Ui) {
            let mut filters_changed = false;
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("Agent resources").strong().size(16.0));
                for (scope, label) in [
                    (resources::ResourceScope::Project, "Project"),
                    (resources::ResourceScope::Global, "Global"),
                ] {
                    if ui
                        .selectable_label(self.resource_scope == scope, label)
                        .clicked()
                    {
                        self.resource_scope = scope;
                        filters_changed = true;
                    }
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(
                        egui::RichText::new(self.resource_root.display().to_string())
                            .small()
                            .color(muted()),
                    );
                });
            });
            ui.add_space(3.0);
            ui.horizontal_wrapped(|ui| {
                for (kind, label) in [
                    (None, "All"),
                    (Some(resources::ResourceKind::Mcp), "MCP"),
                    (Some(resources::ResourceKind::Skill), "Skills"),
                    (Some(resources::ResourceKind::Instructions), "AGENTS"),
                ] {
                    if ui
                        .selectable_label(self.resource_kind == kind, label)
                        .clicked()
                    {
                        self.resource_kind = kind;
                        filters_changed = true;
                    }
                }
                ui.add(
                    egui::TextEdit::singleline(&mut self.resource_filter)
                        .hint_text("Search resources…")
                        .desired_width(180.0),
                );
                if ui.small_button("Refresh").clicked() {
                    self.refresh_resources();
                }
                if self.resource_scope == resources::ResourceScope::Project {
                    if self.resource_root.join(".skillshare/config.yaml").is_file() {
                        if ui.button("Sync project skills").clicked() {
                            self.sync_project_skills();
                        }
                    } else if ui.button("Enable project sharing").clicked() {
                        self.initialize_project_skills();
                    }
                }
            });
            if filters_changed {
                self.selected_resource = None;
            }
            if let Some(feedback) = &self.resource_sync_feedback {
                ui.label(egui::RichText::new(feedback).small().color(muted()));
            }
            ui.separator();

            let needle = self.resource_filter.trim().to_lowercase();
            let visible: Vec<_> = self
                .resource_catalog
                .entries
                .iter()
                .filter(|entry| entry.scope == self.resource_scope)
                .filter(|entry| self.resource_kind.is_none_or(|kind| entry.kind == kind))
                .filter(|entry| {
                    needle.is_empty()
                        || entry.name.to_lowercase().contains(&needle)
                        || entry
                            .path
                            .to_string_lossy()
                            .to_lowercase()
                            .contains(&needle)
                })
                .cloned()
                .collect();
            let selected = self.selected_resource.clone();
            ui.columns(2, |columns| {
                columns[0].label(
                    egui::RichText::new(format!("{} resources", visible.len()))
                        .small()
                        .color(muted()),
                );
                columns[0].separator();
                egui::ScrollArea::vertical()
                    .auto_shrink([false; 2])
                    .show(&mut columns[0], |ui| {
                        if visible.is_empty() {
                            ui.label(
                                egui::RichText::new("No resources in this scope").color(muted()),
                            );
                        }
                        for entry in &visible {
                            let active = selected.as_deref() == Some(entry.id.as_str());
                            let consumers = if entry.shared_with.len() > 1 {
                                format!("  ·  {} agents", entry.shared_with.len())
                            } else {
                                String::new()
                            };
                            let response = ui.selectable_label(
                                active,
                                egui::RichText::new(format!(
                                    "{}   {}{}",
                                    resource_kind_label(entry.kind),
                                    entry.name,
                                    consumers
                                )),
                            );
                            response
                                .clone()
                                .on_hover_text(entry.path.display().to_string());
                            if response.clicked() {
                                self.selected_resource = Some(entry.id.clone());
                            }
                        }
                    });
                columns[1].vertical(|ui| {
                    ui.label(egui::RichText::new("Resource inspector").strong());
                    ui.separator();
                    let entry = self
                        .selected_resource
                        .as_ref()
                        .and_then(|id| visible.iter().find(|entry| &entry.id == id));
                    if let Some(entry) = entry {
                        ui.label(egui::RichText::new(&entry.name).strong().size(15.0));
                        ui.add_space(6.0);
                        egui::Grid::new("resource_detail")
                            .num_columns(2)
                            .spacing([12.0, 7.0])
                            .show(ui, |ui| {
                                ui.label(egui::RichText::new("Type").color(muted()));
                                ui.label(resource_kind_label(entry.kind));
                                ui.end_row();
                                ui.label(egui::RichText::new("Scope").color(muted()));
                                ui.label(resource_scope_label(entry.scope));
                                ui.end_row();
                                ui.label(egui::RichText::new("Agent").color(muted()));
                                ui.label(entry.agent.map(agent_label).unwrap_or("Shared"));
                                ui.end_row();
                                ui.label(egui::RichText::new("Status").color(muted()));
                                ui.label(format!("{:?}", entry.status));
                                ui.end_row();
                                ui.label(egui::RichText::new("Path").color(muted()));
                                ui.add(egui::Label::new(entry.path.display().to_string()).wrap());
                                ui.end_row();
                            });
                        if !entry.shared_with.is_empty() {
                            ui.add_space(8.0);
                            ui.label(egui::RichText::new("Used by").color(muted()));
                            ui.label(
                                entry
                                    .shared_with
                                    .iter()
                                    .copied()
                                    .map(agent_label)
                                    .collect::<Vec<_>>()
                                    .join(" · "),
                            );
                        }
                    } else {
                        ui.label(
                            egui::RichText::new(
                                "Select a resource to inspect its scope and consumers.",
                            )
                            .color(muted()),
                        );
                    }
                });
            });
        }

        fn render_overview(&self, ui: &mut egui::Ui) {
            let Some(contract) = &self.contract else {
                empty_state(
                    ui,
                    "Select a swarm run",
                    "Choose a run from a project to inspect it.",
                );
                return;
            };
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("Swarm map").strong().size(16.0));
                ui.label(
                    egui::RichText::new("stages flow left to right")
                        .small()
                        .color(muted()),
                );
            });
            let column_width = 190.0;
            let node_height = 58.0;
            let gap = 18.0;
            let max_tasks = contract
                .stages
                .iter()
                .map(|stage| stage.tasks.len())
                .max()
                .unwrap_or(1);
            let size = egui::vec2(
                contract.stages.len().max(1) as f32 * (column_width + gap),
                54.0 + max_tasks as f32 * (node_height + gap),
            );
            egui::ScrollArea::both().show(ui, |ui| {
                let (response, painter) = ui.allocate_painter(size, egui::Sense::hover());
                let origin = response.rect.min;
                let mut positions: HashMap<String, egui::Rect> = HashMap::new();
                for (stage_index, stage) in contract.stages.iter().enumerate() {
                    let x = origin.x + stage_index as f32 * (column_width + gap);
                    painter.text(
                        egui::pos2(x, origin.y + 8.0),
                        egui::Align2::LEFT_TOP,
                        &stage.name,
                        egui::FontId::proportional(14.0),
                        accent(),
                    );
                    for (task_index, task) in stage.tasks.iter().enumerate() {
                        let y = origin.y + 38.0 + task_index as f32 * (node_height + gap);
                        let rect = egui::Rect::from_min_size(
                            egui::pos2(x, y),
                            egui::vec2(column_width, node_height),
                        );
                        positions.insert(task.task_id.clone(), rect);
                        if let Some(source_id) = &task.source_id {
                            positions.insert(source_id.clone(), rect);
                        }
                    }
                }
                for stage in &contract.stages {
                    for task in &stage.tasks {
                        let Some(target) = positions.get(&task.task_id) else {
                            continue;
                        };
                        for dependency in &task.needs {
                            if let Some(source) = positions.get(dependency) {
                                painter.line_segment(
                                    [source.right_center(), target.left_center()],
                                    egui::Stroke::new(1.0, egui::Color32::from_rgb(62, 64, 74)),
                                );
                            }
                        }
                    }
                }
                for stage in &contract.stages {
                    for task in &stage.tasks {
                        let Some(rect) = positions.get(&task.task_id) else {
                            continue;
                        };
                        painter.rect_filled(*rect, 7.0, egui::Color32::from_rgb(23, 25, 31));
                        painter.rect_stroke(
                            *rect,
                            7.0,
                            egui::Stroke::new(1.0, egui::Color32::from_rgb(46, 48, 58)),
                            egui::StrokeKind::Inside,
                        );
                        painter.circle_filled(
                            egui::pos2(rect.left() + 13.0, rect.top() + 16.0),
                            4.0,
                            status_color(&task.status, false),
                        );
                        painter.text(
                            egui::pos2(rect.left() + 24.0, rect.top() + 9.0),
                            egui::Align2::LEFT_TOP,
                            task.source_id.as_deref().unwrap_or(&task.task_id),
                            egui::FontId::proportional(13.0),
                            egui::Color32::from_rgb(224, 225, 230),
                        );
                        painter.text(
                            egui::pos2(rect.left() + 13.0, rect.top() + 34.0),
                            egui::Align2::LEFT_TOP,
                            format!(
                                "{}  ·  {}",
                                task.status,
                                task.model.as_deref().unwrap_or("local")
                            ),
                            egui::FontId::proportional(11.0),
                            muted(),
                        );
                    }
                }
            });
        }

        fn render_activity(&self, ui: &mut egui::Ui) {
            ui.label(egui::RichText::new("Activity").strong().size(16.0));
            if self.events.is_empty() {
                empty_state(
                    ui,
                    "No activity yet",
                    "Events appear here while the swarm runs.",
                );
                return;
            }
            egui::ScrollArea::vertical().show_rows(ui, 30.0, self.events.len(), |ui, range| {
                for index in range {
                    let event = &self.events[self.events.len() - 1 - index];
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new("●").color(accent()));
                        ui.label(egui::RichText::new(&event.event).strong());
                        if let Some(task_id) = &event.task_id {
                            ui.label(egui::RichText::new(task_id).small().color(muted()));
                        }
                        if let Some(provider) = &event.provider {
                            ui.label(egui::RichText::new(provider).small().color(muted()));
                        }
                    });
                }
            });
        }

        fn render_quota_strip(&self, ui: &mut egui::Ui) {
            if let Some(snapshot) = &self.quota_snapshot {
                for entry in &snapshot.entries {
                    let remaining = quota_remaining(&entry.windows);
                    let color = quota_color(remaining);
                    let response = egui::Frame::new()
                        .fill(egui::Color32::from_rgb(20, 22, 27))
                        .stroke(egui::Stroke::new(1.0, egui::Color32::from_rgb(42, 45, 54)))
                        .corner_radius(4.0)
                        .inner_margin(egui::Margin::symmetric(6, 3))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                quota_mark(ui, &entry.key, color, &self.provider_icons);
                                ui.label(
                                    egui::RichText::new(quota_short_label(&entry.key))
                                        .small()
                                        .color(egui::Color32::from_rgb(205, 207, 214)),
                                );
                                ui.label(
                                    egui::RichText::new(format!("{remaining:.0}%"))
                                        .small()
                                        .strong()
                                        .color(color),
                                );
                            });
                        })
                        .response;
                    response.on_hover_ui(|ui| {
                        ui.set_min_width(250.0);
                        render_quota_popover(
                            ui,
                            snapshot.generated_at_epoch,
                            entry,
                            &self.provider_icons,
                            &self.ui_privacy,
                        );
                    });
                }
            } else if let Some(error) = &self.quota_error {
                ui.label(
                    egui::RichText::new(format!("quotas unavailable · {error}"))
                        .small()
                        .color(muted()),
                );
            }
        }

        fn render_config(&mut self, ui: &mut egui::Ui) {
            ui.label(egui::RichText::new("Privacidad de cuentas").strong());
            ui.checkbox(
                &mut self.ui_privacy.show_account_emails,
                "Mostrar correos en el detalle de cuotas",
            );
            ui.label(
                egui::RichText::new("Los correos se guardan sólo en config/swarm_ui.local.json.")
                    .small()
                    .color(muted()),
            );
            ui.separator();
            for key in [
                "codex:Codex",
                "codex:Hermes",
                "agy:claude_gpt",
                "agy:gemini",
                "zai:coding",
            ] {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(quota_short_label(key)).small());
                    let email = self
                        .ui_privacy
                        .account_emails
                        .entry(key.to_string())
                        .or_default();
                    ui.add(
                        egui::TextEdit::singleline(email)
                            .hint_text("correo no configurado")
                            .desired_width(270.0),
                    );
                });
            }
            ui.add_space(6.0);
            if ui.button("Guardar").clicked() {
                self.save_ui_config();
            }
            if let Some(feedback) = &self.config_feedback {
                ui.label(egui::RichText::new(feedback).small().color(muted()));
            }
        }

        fn render_tree(&mut self, ui: &mut egui::Ui, now_ms: u128) {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("Execution").strong().size(16.0));
                ui.add(
                    egui::TextEdit::singleline(&mut self.filter)
                        .hint_text("Filter tasks…")
                        .desired_width(220.0),
                );
                if !self.filter.is_empty() && ui.small_button("Clear").clicked() {
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

            if self.rows_dirty || self.rows_filter != self.filter {
                self.rows = flatten(contract, now_ms, &self.filter);
                self.rows_filter.clone_from(&self.filter);
                self.rows_dirty = false;
            }
            let total = self.rows.len();
            let selected = self.selected_task.clone();
            let mut new_selection: Option<String> = None;
            egui::ScrollArea::vertical()
                .auto_shrink([false; 2])
                .show_rows(ui, ROW_HEIGHT, total, |ui, range| {
                    for idx in range {
                        let row = &self.rows[idx];
                        let indent = row.depth as f32 * 16.0;
                        let response = match row.kind {
                            RowKind::Stage => {
                                let counts = format_counts(&row.counts);
                                ui.horizontal(|ui| {
                                    ui.add_space(indent);
                                    ui.label(
                                        egui::RichText::new(&row.label).strong().color(accent()),
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
                                    "{}    {}",
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
                                        ui.selectable_label(is_sel, rich)
                                    })
                                    .inner;
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

            ui.label(
                egui::RichText::new(format!("● {}", node.status))
                    .small()
                    .color(status_color(&node.status, node.is_stale(now_ms, interval))),
            );
            ui.add_space(4.0);
            egui::Grid::new("task_metadata")
                .num_columns(2)
                .spacing([12.0, 7.0])
                .show(ui, |ui| {
                    let mut row = |key: &str, value: &str| {
                        ui.label(egui::RichText::new(key).small().color(muted()));
                        ui.label(value);
                        ui.end_row();
                    };
                    row("task", &node.task_id);
                    if let Some(source) = &node.source_id {
                        row("source", source);
                    }
                    row("role", &node.role);
                    row("provider", node.provider.as_deref().unwrap_or("—"));
                    row("model", node.model.as_deref().unwrap_or("—"));
                    if let Some(wrapper) = &node.wrapper {
                        row("wrapper", wrapper);
                    }
                    row("attempts", &node.attempts.to_string());
                    if let Some(heartbeat) = node.heartbeat_unix_ms {
                        let age = heartbeat_age_seconds(Some(heartbeat), now_ms).unwrap_or(0);
                        row("heartbeat", &format!("{age}s ago"));
                    }
                    row("agent", &node.agent.agent_id);
                    if let Some(owner) = &node.agent.owner {
                        row("owner", owner);
                    }
                    if !node.needs.is_empty() {
                        row("needs", &node.needs.join(", "));
                    }
                });
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

            ui.separator();
            ui.label(egui::RichText::new("Steer agent").strong());
            let steerable =
                node.status == "in_progress" && node.wrapper.as_deref().is_some_and(steer_capable);
            if steerable {
                ui.add(
                    egui::TextEdit::multiline(&mut self.steer_prompt)
                        .hint_text("Add direction for the agent's next turn…")
                        .desired_rows(3),
                );
                ui.horizontal(|ui| {
                    let send = ui.add_enabled(
                        !self.steer_prompt.trim().is_empty(),
                        egui::Button::new("Send steer").fill(accent()),
                    );
                    ui.label(
                        egui::RichText::new(format!(
                            "{}/{}",
                            self.steer_prompt.chars().count(),
                            steering::MAX_STEER_PROMPT_CHARS
                        ))
                        .small()
                        .color(muted()),
                    );
                    if send.clicked() {
                        let run_dir = self
                            .active_run_id
                            .as_ref()
                            .map(|run_id| self.run_root.join(run_id));
                        self.steer_feedback = run_dir.map_or_else(
                            || Some("no active run".to_string()),
                            |run_dir| match steering::enqueue(
                                &run_dir,
                                &node.task_id,
                                &self.steer_prompt,
                                "swarms-ui",
                            ) {
                                Ok(_) => {
                                    self.steer_prompt.clear();
                                    Some("queued for the next agent turn".to_string())
                                }
                                Err(error) => Some(error),
                            },
                        );
                    }
                });
            } else {
                ui.label(
                    egui::RichText::new(
                        "Available while a Codex, OpenCode or Kilo task is running.",
                    )
                    .small()
                    .color(muted()),
                );
            }
            if let Some(feedback) = &self.steer_feedback {
                ui.label(egui::RichText::new(feedback).small().color(muted()));
            }
            if let Some(reader) = &self.reader {
                for applied in steering::history(reader.run_dir(), &node.task_id)
                    .iter()
                    .rev()
                    .take(3)
                {
                    ui.label(
                        egui::RichText::new(format!(
                            "{}  {}",
                            applied.status,
                            applied.message.prompt.chars().take(72).collect::<String>()
                        ))
                        .small()
                        .color(muted()),
                    );
                }
            }
            if !node.artifacts.is_empty() {
                ui.separator();
                ui.label("artifacts:");
                for a in &node.artifacts {
                    ui.label(format!("  {a}"));
                }
            }

            // Worker log tail (loaded only for the selected task, 256 KiB cap).
            ui.separator();
            ui.label("worker.log (cap 256 KiB):");
            self.ensure_log();
            match &self.log_text {
                Some(log) => {
                    let line_count = log.lines().count();
                    let row_height = ui.text_style_height(&egui::TextStyle::Monospace);
                    egui::ScrollArea::vertical()
                        .id_salt("worker_log")
                        .max_height(220.0)
                        .show_rows(ui, row_height, line_count, |ui, range| {
                            for line in log.lines().skip(range.start).take(range.len()) {
                                ui.label(egui::RichText::new(line).monospace().small());
                            }
                        });
                }
                None => {
                    ui.label(egui::RichText::new("no worker.log").color(egui::Color32::GRAY));
                }
            }
        }
    }

    fn ctx_request(ctx: &egui::Context) {
        ctx.request_repaint();
    }

    fn empty_state(ui: &mut egui::Ui, title: &str, detail: &str) {
        ui.add_space(48.0);
        ui.vertical_centered(|ui| {
            ui.label(egui::RichText::new(title).strong().size(16.0));
            ui.label(egui::RichText::new(detail).color(muted()));
        });
    }

    fn steer_capable(wrapper: &str) -> bool {
        matches!(wrapper, "codex" | "opencode" | "kilo" | "mock")
    }

    fn quota_label(key: &str) -> &str {
        match key {
            "agy:claude_gpt" => "AGY · Claude + GPT",
            "agy:gemini" => "AGY · Gemini",
            "codex:Codex" => "Codex · Codex",
            "codex:Hermes" => "Codex · Account 2",
            "zai:coding" => "Z.AI Coding Plan",
            _ => key,
        }
    }

    fn quota_short_label(key: &str) -> &str {
        match key {
            "agy:claude_gpt" => "Claude",
            "agy:gemini" => "Gemini",
            "codex:Codex" => "Codex",
            "codex:Hermes" => "Codex 2",
            "zai:coding" => "Coding",
            _ => key,
        }
    }

    fn quota_account_label(key: &str) -> &str {
        match key {
            "codex:Codex" => "Codex · Cuenta principal",
            "codex:Hermes" => "Codex · Cuenta 2",
            "agy:claude_gpt" => "AGY · Claude + GPT",
            "agy:gemini" => "AGY · Gemini",
            "zai:coding" => "Z.AI · Coding",
            _ => key,
        }
    }

    fn quota_remaining(windows: &BTreeMap<String, f64>) -> f64 {
        windows
            .values()
            .copied()
            .filter(|value| value.is_finite())
            .min_by(f64::total_cmp)
            .unwrap_or(0.0)
            .clamp(0.0, 100.0)
    }

    #[derive(Default)]
    struct ProviderIcons;

    impl ProviderIcons {
        fn source(key: &str) -> Option<egui::ImageSource<'static>> {
            if key == "zai:coding" {
                return Some(egui::ImageSource::Bytes {
                    uri: "bytes://provider-icons/zcode.png".into(),
                    bytes: egui::load::Bytes::Static(include_bytes!(
                        "../assets/provider-icons/zcode.png"
                    )),
                });
            }
            let (uri, bytes): (&str, &'static [u8]) = match key {
                "agy:claude_gpt" => (
                    "bytes://provider-icons/anthropic.svg",
                    include_bytes!("../assets/provider-icons/anthropic.svg"),
                ),
                "agy:gemini" => (
                    "bytes://provider-icons/gemini.svg",
                    include_bytes!("../assets/provider-icons/gemini.svg"),
                ),
                "codex:Codex" | "codex:Hermes" => (
                    "bytes://provider-icons/openai.svg",
                    include_bytes!("../assets/provider-icons/openai.svg"),
                ),
                _ => return None,
            };
            Some(egui::ImageSource::Bytes {
                uri: uri.into(),
                bytes: egui::load::Bytes::Static(bytes),
            })
        }
    }

    fn quota_mark(ui: &mut egui::Ui, key: &str, color: egui::Color32, _icons: &ProviderIcons) {
        if let Some(source) = ProviderIcons::source(key) {
            ui.add(
                egui::Image::new(source)
                    .fit_to_exact_size(egui::vec2(18.0, 18.0))
                    .tint(egui::Color32::WHITE),
            );
            return;
        }
        let (rect, _) = ui.allocate_exact_size(egui::vec2(16.0, 16.0), egui::Sense::hover());
        let painter = ui.painter();
        let center = rect.center();
        painter.circle_filled(center, 4.0, color);
    }

    fn render_quota_popover(
        ui: &mut egui::Ui,
        generated_at_epoch: u64,
        entry: &quota::QuotaViewEntry,
        icons: &ProviderIcons,
        privacy: &UiPrivacyConfig,
    ) {
        let remaining = quota_remaining(&entry.windows);
        let color = quota_color(remaining);
        let age = unix_ms().saturating_sub(u128::from(generated_at_epoch) * 1000) / 1000;
        ui.horizontal(|ui| {
            quota_mark(ui, &entry.key, color, icons);
            ui.label(
                egui::RichText::new(quota_label(&entry.key))
                    .strong()
                    .size(13.0),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(
                    egui::RichText::new(format!("updated {age}s ago"))
                        .small()
                        .color(muted()),
                );
            });
        });
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("Cuenta").small().color(muted()));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(egui::RichText::new(quota_account_label(&entry.key)).small());
            });
        });
        if privacy.show_account_emails {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("Correo").small().color(muted()));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let email = privacy
                        .account_emails
                        .get(&entry.key)
                        .map(String::as_str)
                        .unwrap_or("no disponible");
                    ui.label(egui::RichText::new(email).small());
                });
            });
        }
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("Available").small().color(muted()));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(
                    egui::RichText::new(format!("{remaining:.0}%"))
                        .strong()
                        .color(color),
                );
            });
        });
        let (rect, _) =
            ui.allocate_exact_size(egui::vec2(ui.available_width(), 5.0), egui::Sense::hover());
        ui.painter()
            .rect_filled(rect, 2.5, egui::Color32::from_rgb(47, 50, 59));
        ui.painter().rect_filled(
            egui::Rect::from_min_size(
                rect.min,
                egui::vec2(rect.width() * remaining as f32 / 100.0, rect.height()),
            ),
            2.5,
            color,
        );
        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            for (window, value) in &entry.windows {
                ui.label(
                    egui::RichText::new(format!("{window} {value:.0}%"))
                        .small()
                        .color(quota_color(*value)),
                );
            }
        });
    }

    fn quota_color(remaining: f64) -> egui::Color32 {
        if remaining < 15.0 {
            egui::Color32::from_rgb(235, 91, 91)
        } else if remaining < 35.0 {
            egui::Color32::from_rgb(225, 170, 78)
        } else {
            egui::Color32::from_rgb(85, 198, 118)
        }
    }

    fn find_workspace_root(run_root: &Path) -> Option<PathBuf> {
        run_root.ancestors().find_map(|ancestor| {
            ancestor
                .join("config/swarm_router.json")
                .is_file()
                .then(|| ancestor.to_path_buf())
        })
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
        let executable = std::env::current_exe().ok();
        let mut run_root = default_run_root(&cwd, executable.as_deref());
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

    fn default_run_root(cwd: &Path, executable: Option<&Path>) -> PathBuf {
        for start in std::iter::once(cwd).chain(executable.and_then(Path::parent)) {
            for ancestor in start.ancestors() {
                let candidate = ancestor.join(".agent/swarm/runs");
                if candidate.is_dir() {
                    return candidate;
                }
            }
        }
        cwd.join(".agent/swarm/runs")
    }

    pub fn run() -> eframe::Result {
        let (run_root, run_id, ready_file, bench) = parse_args();
        let runs = list_runs(&run_root);
        let initial = run_id.or_else(|| (runs.len() == 1).then(|| runs[0].run_id.clone()));
        let mut app = ObservabilityApp::new(run_root, None, ready_file, bench);
        app.runs = runs;
        app.last_runs_poll = Some(Instant::now());
        if let Some(run_id) = initial {
            app.activate(run_id);
        }
        let options = eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default()
                .with_inner_size([1200.0, 800.0])
                .with_title("SWARMS"),
            ..Default::default()
        };
        eframe::run_native(
            "SWARMS",
            options,
            Box::new(move |cc| {
                egui_extras::install_image_loaders(&cc.egui_ctx);
                apply_theme(&cc.egui_ctx);
                Ok(Box::new(app))
            }),
        )
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn poll_interval_is_low_frequency() {
            let mut app = ObservabilityApp::new(PathBuf::new(), None, None, None);
            assert_eq!(app.poll_interval(), POLL_IDLE);
            let mut contract = RunContract::default();
            contract.run.status = RunStatus::Running;
            app.contract = Some(contract);
            assert_eq!(app.poll_interval(), POLL_ACTIVE);
        }

        #[test]
        fn quota_presentation_uses_human_names_and_risk_thresholds() {
            assert_eq!(quota_label("agy:claude_gpt"), "AGY · Claude + GPT");
            assert_eq!(quota_label("zai:coding"), "Z.AI Coding Plan");
            assert_eq!(quota_label("custom:plan"), "custom:plan");
            assert_eq!(quota_short_label("codex:Codex"), "Codex");
            assert_eq!(quota_short_label("codex:Hermes"), "Codex 2");
            assert_eq!(
                quota_account_label("codex:Codex"),
                "Codex · Cuenta principal"
            );
            assert_eq!(quota_account_label("codex:Hermes"), "Codex · Cuenta 2");
            assert_eq!(
                quota_remaining(&BTreeMap::from([
                    ("5h".to_string(), 47.0),
                    ("7d".to_string(), 100.0),
                ])),
                47.0
            );
            assert_ne!(quota_color(13.0), quota_color(53.0));
        }

        #[test]
        fn quota_identities_are_loaded_from_the_private_sibling_file() {
            let root = std::env::temp_dir().join(format!("swarms-identities-{}", unix_ms()));
            fs::create_dir_all(&root).unwrap();
            let snapshot = root.join("quota_snapshot.json");
            fs::write(
                root.join("quota_identities.local.json"),
                r#"{"version":1,"accounts":{"codex:Codex":"one@example.com"}}"#,
            )
            .unwrap();

            let accounts = load_quota_identities(&snapshot).unwrap();

            assert_eq!(accounts.get("codex:Codex").unwrap(), "one@example.com");
            fs::remove_dir_all(root).ok();
        }

        #[test]
        fn ready_file_is_written_once_without_starting_workers() {
            let root = std::env::temp_dir().join(format!("swarms-ready-{}", unix_ms()));
            let ready = root.join("ready.json");
            let mut app = ObservabilityApp::new(
                root.clone(),
                Some("run-1".to_string()),
                Some(ready.clone()),
                None,
            );
            app.write_ready();
            let payload = fs::read_to_string(&ready).unwrap();
            assert!(payload.contains(r#""ready":true"#));
            assert!(payload.contains("run-1"));
            fs::remove_dir_all(root).ok();
        }

        #[test]
        fn run_signature_changes_only_with_observed_state() {
            let root = std::env::temp_dir().join(format!("swarms-signature-{}", unix_ms()));
            fs::create_dir_all(&root).unwrap();
            fs::write(root.join("workflow.json"), "{}").unwrap();
            let first = RunSignature::read(&root);
            assert_eq!(first, RunSignature::read(&root));
            fs::write(root.join("workflow.json"), r#"{"run_id":"changed"}"#).unwrap();
            assert_ne!(first, RunSignature::read(&root));
            fs::remove_dir_all(root).ok();
        }

        #[test]
        fn activating_completed_run_loads_existing_events() {
            let root = std::env::temp_dir().join(format!("swarms-events-{}", unix_ms()));
            let run_dir = root.join("done");
            fs::create_dir_all(&run_dir).unwrap();
            fs::write(
                run_dir.join("workflow.json"),
                r#"{"run_id":"done","runtime":"rust"}"#,
            )
            .unwrap();
            fs::write(
                run_dir.join("events.jsonl"),
                "{\"event\":\"workflow_finished\",\"payload\":{}}\n",
            )
            .unwrap();
            let mut reader = RunReader::open(&root, "done", Vec::new()).unwrap();
            assert_eq!(reader.tail_events(MAX_EVENTS).len(), 1);
            let mut app = ObservabilityApp::new(root.clone(), None, None, None);
            app.activate("done".to_string());
            assert_eq!(app.events.len(), 1, "activation error: {:?}", app.error);
            fs::remove_dir_all(root).ok();
        }

        #[test]
        fn activating_run_switches_resources_to_its_workspace() {
            let root = std::env::temp_dir().join(format!("swarms-resource-root-{}", unix_ms()));
            let runs = root.join("runs");
            let project = root.join("other-project");
            let run_dir = runs.join("project-run");
            fs::create_dir_all(&run_dir).unwrap();
            fs::create_dir_all(&project).unwrap();
            fs::write(project.join("AGENTS.md"), "project instructions").unwrap();
            fs::write(
                run_dir.join("workflow.json"),
                serde_json::to_string(&serde_json::json!({
                    "run_id": "project-run",
                    "runtime": "rust",
                    "workspace_root": project,
                }))
                .unwrap(),
            )
            .unwrap();

            let mut app = ObservabilityApp::new(runs, None, None, None);
            app.activate("project-run".to_string());

            assert_eq!(app.resource_root, project);
            assert!(app
                .resource_catalog
                .entries
                .iter()
                .any(|entry| entry.name == "AGENTS.md"));
            fs::remove_dir_all(root).ok();
        }

        #[test]
        fn default_run_root_finds_repo_from_release_executable() {
            let root = std::env::temp_dir().join(format!("swarms-root-{}", unix_ms()));
            let runs = root.join(".agent/swarm/runs");
            let executable = root.join("rust/target/release/swarms-ui.exe");
            fs::create_dir_all(&runs).unwrap();
            fs::create_dir_all(executable.parent().unwrap()).unwrap();
            assert_eq!(
                default_run_root(Path::new("C:/unrelated"), Some(&executable)),
                runs
            );
            fs::remove_dir_all(root).ok();
        }
    }
}
