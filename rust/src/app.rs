//! Feature-gated read-only egui/eframe observer window for SWARMS runs.
//!
//! Compiled only when `--features ui-egui` is on (see `rust/Cargo.toml`).
//! This module NEVER writes run state, NEVER claims tasks and NEVER spawns
//! workers; it renders `RunReader` output into a single native window with
//! virtualized rows, on-demand repaints and an in-window detail panel.
//!
//! See `docs/SWARM_UI.md` for usage and the exact Windows toolchain blocker.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use eframe::egui;

use crate::{
    flatten, heartbeat_age_seconds, list_runs, read_worker_log_tail, unix_ms, EventRow, FlatRow,
    RowKind, RunContract, RunIndex, RunReader, RunStatus, SubagentNode, TaskNode, MAX_LOG_BYTES,
};

/// Poll cadence while a run is active (focused work). Per UI_RUNTIME_EVALUATION.
const ACTIVE_POLL: Duration = Duration::from_millis(500);
/// Poll cadence when idle / no active run. Never spin a fixed 60 FPS loop.
const IDLE_POLL: Duration = Duration::from_millis(2000);
/// Max lifecycle events retained resident from `events.jsonl`.
const MAX_EVENT_ROWS: usize = 500;
/// Virtualized row height in points.
const ROW_HEIGHT: f32 = 20.0;

/// Open the read-only observer window. Blocks until the user closes it.
///
/// This is the intended entry point for the `swarms-ui` binary. The current
/// partial spike leaves wiring `fn main()` in `ui_main.rs` as a follow-up (see
/// `docs/SWARM_UI.md`); the module is otherwise complete and source-compilable.
pub fn launch(run_root: PathBuf, run_id: Option<String>) -> Result<(), String> {
    let options = eframe::NativeOptions::default();
    eframe::run_native(
        "SWARMS Run Observer",
        options,
        Box::new(move |_cc| Ok(Box::new(SwarmApp::new(run_root, run_id)) as Box<dyn eframe::App>)),
    )
    .map_err(|e| format!("eframe error: {e}"))
}

// ---------------------------------------------------------------------------
// Selection / state
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
enum DetailRef {
    Task(String),
    Subagent(String),
}

#[derive(Clone, Debug, Default)]
struct Selection {
    run_id: Option<String>,
    detail: Option<DetailRef>,
}

/// Cheap filesystem signature: if any member changed, re-read the contract.
/// `tasks/` dir mtime bumps on the atomic snapshot rename; `events.jsonl`
/// grows on append; report presence flips once at termination.
#[derive(Clone, Debug, PartialEq, Eq)]
struct FsSig {
    tasks_dir_mt: Option<SystemTime>,
    events_size: u64,
    events_mt: Option<SystemTime>,
    report_present: bool,
}

impl FsSig {
    fn read(run_dir: &Path) -> Self {
        let tasks_dir_mt = fs::metadata(run_dir.join("tasks"))
            .and_then(|m| m.modified())
            .ok();
        let (events_size, events_mt) = match fs::metadata(run_dir.join("events.jsonl")) {
            Ok(m) => (m.len(), m.modified().ok()),
            Err(_) => (0, None),
        };
        let report_present =
            run_dir.join("report.json").exists() || run_dir.join("report-rs.json").exists();
        FsSig {
            tasks_dir_mt,
            events_size,
            events_mt,
            report_present,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AppState {
    Loading,
    Empty,
    Ready,
    Error,
}

pub struct SwarmApp {
    run_root: PathBuf,
    explicit_run: Option<String>,
    current_run: Option<String>,
    reader: Option<RunReader>,
    selection: Selection,
    contract: Option<RunContract>,
    rows: Vec<FlatRow>,
    events: Vec<EventRow>,
    log_tail: Option<String>,
    log_for: Option<String>,
    filter: String,
    error: Option<String>,
    runs: Vec<RunIndex>,
    last_poll: Instant,
    last_sig: Option<FsSig>,
    pending: bool,
}

impl SwarmApp {
    pub fn new(run_root: PathBuf, run_id: Option<String>) -> Self {
        let explicit = run_id.clone();
        SwarmApp {
            run_root,
            explicit_run: explicit.clone(),
            current_run: None,
            reader: None,
            selection: Selection {
                run_id: explicit,
                detail: None,
            },
            contract: None,
            rows: Vec::new(),
            events: Vec::new(),
            log_tail: None,
            log_for: None,
            filter: String::new(),
            error: None,
            runs: Vec::new(),
            last_poll: Instant::now()
                .checked_sub(ACTIVE_POLL)
                .unwrap_or_else(Instant::now),
            last_sig: None,
            pending: true,
        }
    }

    fn state(&self) -> AppState {
        if self.error.is_some() {
            return AppState::Error;
        }
        let reader = match &self.reader {
            Some(r) => r,
            None => return AppState::Loading,
        };
        if !reader.exists() {
            return AppState::Empty;
        }
        match &self.contract {
            None => AppState::Loading,
            Some(c) => {
                let total: usize = c.stages.iter().map(|s| s.tasks.len()).sum();
                if total == 0 {
                    AppState::Empty
                } else {
                    AppState::Ready
                }
            }
        }
    }

    fn ensure_reader(&mut self) {
        let target = self
            .selection
            .run_id
            .clone()
            .or_else(|| self.explicit_run.clone());
        let target = match target {
            Some(t) => t,
            None => return,
        };
        if self.current_run.as_deref() == Some(target.as_str()) {
            return;
        }
        match RunReader::open(&self.run_root, &target, Vec::new()) {
            Ok(r) => {
                self.reader = Some(r);
                self.current_run = Some(target);
                self.contract = None;
                self.rows.clear();
                self.events.clear();
                self.log_tail = None;
                self.log_for = None;
                self.selection.detail = None;
                self.last_sig = None;
                self.error = None;
                self.pending = true;
            }
            Err(e) => {
                self.error = Some(e);
                self.reader = None;
                self.current_run = None;
            }
        }
    }

    /// Poll the filesystem at most every ACTIVE_POLL. Re-list the run index
    /// when no explicit run is pinned, and re-stat the open run dir to detect
    /// snapshots/events/report changes. No file watcher dependency.
    fn detect_change(&mut self) {
        let now = Instant::now();
        if now.duration_since(self.last_poll) < ACTIVE_POLL {
            return;
        }
        self.last_poll = now;
        if self.explicit_run.is_none() {
            self.runs = list_runs(&self.run_root);
        }
        let run_dir = match &self.reader {
            Some(r) => r.run_dir().to_path_buf(),
            None => return,
        };
        let sig = FsSig::read(&run_dir);
        if Some(&sig) != self.last_sig.as_ref() {
            self.last_sig = Some(sig);
            self.pending = true;
        }
    }

    fn maybe_refresh(&mut self) {
        if !self.pending {
            return;
        }
        self.pending = false;
        let reader = match self.reader.as_mut() {
            Some(r) => r,
            None => return,
        };
        if !reader.exists() {
            return;
        }
        let contract = reader.read();
        let now = unix_ms();
        self.rows = flatten(&contract, now, &self.filter);
        let new_events = reader.tail_events(MAX_EVENT_ROWS);
        if !new_events.is_empty() {
            self.events.extend(new_events);
            let excess = self.events.len().saturating_sub(MAX_EVENT_ROWS * 4);
            if excess > 0 {
                self.events.drain(0..excess);
            }
        }
        self.contract = Some(contract);
    }

    /// Load the worker.log tail for the selected task only when the selection
    /// changes, never proactively. Capped to MAX_LOG_BYTES by the reader.
    fn maybe_load_log(&mut self) {
        let want = match &self.selection.detail {
            Some(DetailRef::Task(id)) => Some(id.clone()),
            _ => None,
        };
        if want == self.log_for {
            return;
        }
        self.log_for = want.clone();
        self.log_tail = None;
        let (run_dir, task_id) = match (&self.reader, want) {
            (Some(r), Some(t)) => (r.run_dir().to_path_buf(), t),
            _ => return,
        };
        self.log_tail = read_worker_log_tail(&run_dir, &task_id);
    }
}

impl eframe::App for SwarmApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.ensure_reader();
        self.detect_change();
        self.maybe_refresh();
        self.maybe_load_log();

        // On-demand repaint: poll faster while a run is active; never spin 60fps.
        let active = self
            .contract
            .as_ref()
            .map_or(false, |c| c.run.status == RunStatus::Running);
        let delay = if active { ACTIVE_POLL } else { IDLE_POLL };
        ctx.request_repaint_after(delay);

        egui::TopBottomPanel::bottom("status_bar").show(ctx, |ui| {
            self.render_status_bar(ui);
        });

        if self.explicit_run.is_none() {
            egui::SidePanel::left("runs")
                .resizable(true)
                .default_width(220.0)
                .min_width(150.0)
                .show(ctx, |ui| {
                    self.render_runs_panel(ui);
                });
        }

        egui::SidePanel::right("detail")
            .resizable(true)
            .default_width(400.0)
            .min_width(240.0)
            .show(ctx, |ui| {
                self.render_detail_panel(ui);
            });

        egui::CentralPanel::default().show(ctx, |ui| {
            self.render_tree(ui);
        });
    }
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

impl SwarmApp {
    fn state_badge(&self) -> (&'static str, egui::Color32) {
        match self.state() {
            AppState::Loading => ("[loading]", egui::Color32::from_rgb(210, 170, 70)),
            AppState::Empty => ("[empty]", egui::Color32::from_rgb(150, 150, 150)),
            AppState::Error => ("[error]", egui::Color32::from_rgb(210, 90, 90)),
            AppState::Ready => ("[ready]", egui::Color32::from_rgb(100, 180, 100)),
        }
    }

    fn render_status_bar(&self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            let (label, color) = self.state_badge();
            ui.colored_label(color, label);
            ui.separator();
            if let Some(c) = &self.contract {
                ui.label(format!("run: {}", c.run.run_id));
                ui.label(format!("status: {}", c.run.status.label()));
                ui.label(format!("stages: {}", c.summary.stage_count));
                let total: usize = c.stages.iter().map(|s| s.tasks.len()).sum();
                ui.label(format!("tasks: {total}"));
                ui.label(format!("results: {}", c.summary.result_count));
                if let Some(g) = c.run.global_max_concurrency {
                    ui.label(format!("global_max_concurrency: {g}"));
                }
                if !c.run.provider_max_concurrency.is_empty() {
                    let caps: Vec<String> = c
                        .run
                        .provider_max_concurrency
                        .iter()
                        .map(|(k, v)| format!("{k}={v}"))
                        .collect();
                    ui.label(format!("provider_caps: {}", caps.join(", ")));
                }
                if let Some(hb) = c.summary.last_heartbeat_unix_ms {
                    if let Some(age) = heartbeat_age_seconds(Some(hb), unix_ms()) {
                        ui.label(format!("last_heartbeat: {age}s ago"));
                    }
                }
                if c.summary.has_real_provider {
                    ui.colored_label(
                        egui::Color32::from_rgb(210, 170, 70),
                        "real provider active",
                    );
                }
            } else if let Some(e) = &self.error {
                ui.colored_label(egui::Color32::from_rgb(210, 90, 90), format!("error: {e}"));
            } else {
                ui.weak("no run selected");
            }
            ui.separator();
            ui.label(format!("log_cap: {MAX_LOG_BYTES} B"));
            ui.label(format!("events_buffered: {}", self.events.len()));
        });
    }

    fn render_runs_panel(&mut self, ui: &mut egui::Ui) {
        ui.heading("Runs");
        ui.label(format!("root: {}", self.run_root.display()));
        ui.separator();
        let mut clicked: Option<String> = None;
        egui::ScrollArea::vertical()
            .auto_shrink([false, true])
            .show(ui, |ui| {
                if self.runs.is_empty() {
                    ui.weak("(no runs discovered)");
                }
                for r in &self.runs {
                    let selected = self.selection.run_id.as_deref() == Some(r.run_id.as_str());
                    let dot = if r.has_report { "•" } else { "▶" };
                    let head = format!("{dot} {} ({} tasks)", r.run_id, r.task_count);
                    if ui.selectable_label(selected, head).clicked() {
                        clicked = Some(r.run_id.clone());
                    }
                    ui.small(format!(
                        "runtime {} · created {}",
                        r.runtime,
                        age_label(r.created_unix_ms)
                    ));
                }
            });
        if let Some(id) = clicked {
            self.selection.run_id = Some(id);
        }
    }

    fn render_tree(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label("Filter:");
            let resp = ui.text_edit_singleline(&mut self.filter);
            if resp.changed() {
                if let Some(c) = &self.contract {
                    self.rows = flatten(c, unix_ms(), &self.filter);
                }
            }
            if ui.button("clear").clicked() {
                self.filter.clear();
                if let Some(c) = &self.contract {
                    self.rows = flatten(c, unix_ms(), &self.filter);
                }
            }
        });
        ui.separator();

        let mut clicked: Option<DetailRef> = None;
        match self.state() {
            AppState::Loading => {
                ui.label("Loading run state…");
            }
            AppState::Empty => {
                ui.label(
                    "No task checkpoints yet (empty run). Waiting for the coordinator to write tasks.",
                );
            }
            AppState::Error => {
                ui.colored_label(
                    egui::Color32::from_rgb(210, 90, 90),
                    self.error.clone().unwrap_or_else(|| "error".into()),
                );
            }
            AppState::Ready => {
                let total = self.rows.len();
                ui.label(format!("{total} rows"));
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show_rows(ui, ROW_HEIGHT, total, |ui, range| {
                        for i in range {
                            let row = &self.rows[i];
                            let (color, marker) = row_colors(row);
                            let stale_tag = if row.stale { "  \u{26A0} stale" } else { "" };
                            let text = format!(
                                "{}{} {}{}",
                                " ".repeat(row.depth.saturating_mul(2)),
                                marker,
                                row.label,
                                stale_tag,
                            );
                            let rich = egui::RichText::new(text)
                                .color(color)
                                .family(egui::FontFamily::Monospace);
                            let selected = match (&self.selection.detail, row.kind) {
                                (Some(DetailRef::Task(id)), RowKind::Task) => {
                                    Some(id.as_str()) == row.task_id.as_deref()
                                }
                                (Some(DetailRef::Subagent(id)), RowKind::Subagent) => {
                                    id.as_str() == row.label.as_str()
                                }
                                _ => false,
                            };
                            let selectable = matches!(row.kind, RowKind::Task | RowKind::Subagent);
                            if selectable {
                                let widget = egui::SelectableLabel::new(selected, rich);
                                if ui.add(widget).clicked() {
                                    clicked = Some(match row.kind {
                                        RowKind::Task => {
                                            DetailRef::Task(row.task_id.clone().unwrap_or_default())
                                        }
                                        RowKind::Subagent => DetailRef::Subagent(row.label.clone()),
                                        RowKind::Stage => unreachable!(),
                                    });
                                }
                            } else {
                                ui.label(rich);
                            }
                        }
                    });
            }
        }
        if let Some(d) = clicked {
            self.selection.detail = Some(d);
        }
    }

    fn render_detail_panel(&self, ui: &mut egui::Ui) {
        ui.heading("Detail");
        let contract = match &self.contract {
            Some(c) => c,
            None => {
                ui.weak("Select or open a run to inspect tasks.");
                return;
            }
        };
        let detail = match &self.selection.detail {
            Some(d) => d.clone(),
            None => {
                ui.weak("Select a task or subagent row to see its detail.");
                return;
            }
        };
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| match detail {
                DetailRef::Task(id) => match find_task(contract, &id) {
                    Some(t) => render_task_detail(ui, contract, t, self.log_tail.as_deref()),
                    None => ui.label(format!("Task {id} not found (it may have been removed).")),
                },
                DetailRef::Subagent(agent) => match find_subagent(contract, &agent) {
                    Some((parent, sub)) => render_subagent_detail(ui, parent, sub),
                    None => ui.label(format!("Subagent {agent} not found.")),
                },
            });
    }
}

// ---------------------------------------------------------------------------
// Pure rendering helpers
// ---------------------------------------------------------------------------

fn row_colors(row: &FlatRow) -> (egui::Color32, &'static str) {
    let color = match row.kind {
        RowKind::Stage => egui::Color32::from_rgb(180, 180, 200),
        _ => match row.status.as_str() {
            "completed" => egui::Color32::from_rgb(100, 180, 100),
            "failed" => egui::Color32::from_rgb(210, 90, 90),
            "in_progress" | "queued" => egui::Color32::from_rgb(210, 170, 70),
            _ => egui::Color32::from_rgb(150, 150, 150),
        },
    };
    let marker = match row.kind {
        RowKind::Stage => "\u{25B8}",
        RowKind::Task => "\u{2022}",
        RowKind::Subagent => "\u{2218}",
    };
    (color, marker)
}

fn age_label(created_unix_ms: Option<u128>) -> String {
    match created_unix_ms {
        Some(t) => match heartbeat_age_seconds(Some(t), unix_ms()) {
            Some(s) => format!("{s}s ago"),
            None => "?".into(),
        },
        None => "?".into(),
    }
}

fn colored_status(ui: &mut egui::Ui, status: &str) {
    let color = match status {
        "completed" => egui::Color32::from_rgb(100, 180, 100),
        "failed" => egui::Color32::from_rgb(210, 90, 90),
        "in_progress" | "queued" => egui::Color32::from_rgb(210, 170, 70),
        _ => egui::Color32::from_rgb(150, 150, 150),
    };
    ui.colored_label(color, format!("Status: {status}"));
}

fn field(ui: &mut egui::Ui, name: &str, value: Option<&str>) {
    match value {
        Some(v) if !v.is_empty() => {
            ui.label(format!("{name}: {v}"));
        }
        _ => {
            ui.weak(format!("{name}: \u{2014}"));
        }
    }
}

fn render_task_detail(ui: &mut egui::Ui, contract: &RunContract, t: &TaskNode, log: Option<&str>) {
    ui.label(format!("Task: {}", t.task_id));
    ui.label(format!("Stage index: {}", t.index));
    ui.label(format!("Role: {}", t.role));
    colored_status(ui, &t.status);
    let interval = contract.run.heartbeat_interval_seconds.unwrap_or(30);
    if t.is_stale(unix_ms(), interval) {
        ui.colored_label(
            egui::Color32::from_rgb(210, 170, 70),
            "\u{26A0} stale heartbeat",
        );
    }
    ui.label(format!("Attempts: {}", t.attempts));
    field(ui, "source_id", t.source_id.as_deref());
    field(ui, "parent_task_id", t.parent_task_id.as_deref());
    field(ui, "model", t.model.as_deref());
    field(ui, "provider", t.provider.as_deref());
    field(ui, "route", t.route.as_deref());
    field(ui, "wrapper", t.wrapper.as_deref());
    field(ui, "variant", t.variant.as_deref());
    ui.separator();
    ui.label(format!("Agent: {}", t.agent.agent_id));
    field(ui, "owner", t.agent.owner.as_deref());
    field(ui, "claimed_at", t.agent.claimed_at.as_deref());
    field(ui, "heartbeat_at", t.agent.heartbeat_at.as_deref());
    if let Some(hb) = t.heartbeat_unix_ms {
        if let Some(age) = heartbeat_age_seconds(Some(hb), unix_ms()) {
            ui.label(format!("heartbeat age: {age}s"));
        }
    }
    ui.separator();
    if t.needs.is_empty() {
        ui.weak("needs: (none)");
    } else {
        ui.label(format!("needs: {}", t.needs.join(", ")));
    }
    if !t.artifacts.is_empty() {
        ui.label("artifacts:");
        for a in &t.artifacts {
            ui.monospace(a);
        }
    }
    ui.label(format!(
        "provider_subagents: {} ({})",
        if t.provider_subagents.is_empty() {
            "(none)".into()
        } else {
            t.provider_subagents.join(", ")
        },
        t.provider_subagent_visibility,
    ));
    ui.separator();
    if t.subagents.is_empty() {
        ui.weak("subagents: (none)");
    } else {
        ui.label("subagents:");
        for s in &t.subagents {
            let m = s.model.as_deref().unwrap_or("?");
            ui.label(format!(
                "  {} \u{2014} {} \u{2014} {}",
                s.agent_id, s.status, m
            ));
        }
    }
    if let Some(err) = &t.error {
        ui.separator();
        ui.colored_label(egui::Color32::from_rgb(210, 90, 90), "error:");
        ui.monospace(err);
    }
    ui.separator();
    ui.label("worker.log tail (on-demand, capped):");
    match log {
        Some(log) if !log.is_empty() => {
            egui::ScrollArea::vertical()
                .max_height(240.0)
                .show(ui, |ui| {
                    ui.monospace(log);
                });
        }
        _ => {
            ui.weak("(no worker.log yet)");
        }
    }
}

fn render_subagent_detail(ui: &mut egui::Ui, parent: &TaskNode, s: &SubagentNode) {
    ui.label(format!("Subagent: {}", s.agent_id));
    colored_status(ui, &s.status);
    field(ui, "model", s.model.as_deref());
    field(ui, "task_id", s.task_id.as_deref());
    ui.separator();
    ui.label(format!("Declared under: {}", parent.task_id));
    ui.label(format!(
        "Parent role/provider: {} / {}",
        parent.role,
        parent.provider.as_deref().unwrap_or("?"),
    ));
    ui.weak("Subagent detail is read-only; open its task row for the full snapshot.");
}

fn find_task<'a>(contract: &'a RunContract, task_id: &str) -> Option<&'a TaskNode> {
    contract
        .stages
        .iter()
        .flat_map(|s| s.tasks.iter())
        .find(|t| t.task_id == task_id)
}

fn find_subagent<'a>(
    contract: &'a RunContract,
    agent_id: &str,
) -> Option<(&'a TaskNode, &'a SubagentNode)> {
    for stage in &contract.stages {
        for t in &stage.tasks {
            if let Some(sub) = t.subagents.iter().find(|x| x.agent_id == agent_id) {
                return Some((t, sub));
            }
        }
    }
    None
}
