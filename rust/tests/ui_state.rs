//! Integration tests for the read-only `swarms_runtime::ui` model (no window, no egui).
//!
//! These exercise `RunReader`, `flatten`, status derivation, the event tail
//! offset, log capping and sanitization against on-disk fixtures. They never
//! open a window and never require the `ui-egui` feature.

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};
use swarms_runtime::ui::{
    flatten, group_runs, list_runs, read_worker_log_tail, relative_age, safe_run_id,
    sanitize_error, sanitize_path, unix_ms, RowKind, RunReader, RunStatus, MAX_ERROR_CHARS,
    MAX_LOG_BYTES,
};

static SEQ: AtomicU64 = AtomicU64::new(0);

#[test]
fn relative_age_is_compact_and_handles_missing_timestamps() {
    let now = 10_000_000u128;
    assert_eq!(relative_age(Some(now - 30_000), now), "now");
    assert_eq!(relative_age(Some(now - 5 * 60_000), now), "5m ago");
    assert_eq!(relative_age(Some(now - 2 * 3_600_000), now), "2h ago");
    assert_eq!(relative_age(None, now), "unknown");
}

fn tmp_dir(label: &str) -> PathBuf {
    let n = SEQ.fetch_add(1, Ordering::SeqCst);
    let ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis();
    let dir = std::env::temp_dir().join(format!("swarms-ui-test-{label}-{ms}-{n}"));
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn write_json(path: &PathBuf, value: &Value) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, serde_json::to_string_pretty(value).unwrap()).unwrap();
}

#[test]
fn empty_run_reports_empty_status() {
    let root = tmp_dir("empty");
    let mut reader = RunReader::open(&root, "empty-run", Vec::new()).unwrap();
    assert!(!reader.exists());
    let contract = reader.read();
    assert_eq!(contract.run.status, RunStatus::Empty);
    assert!(contract.stages.is_empty());
    assert!(contract.read_only);
    fs::remove_dir_all(root).ok();
}

#[test]
fn unsafe_run_id_is_rejected() {
    let root = tmp_dir("unsafe");
    let err = RunReader::open(&root, "../escape", Vec::new());
    assert!(err.is_err());
    assert!(!safe_run_id("../escape"));
    fs::remove_dir_all(root).ok();
}

#[test]
fn run_id_with_windows_traversal_is_rejected() {
    assert!(!safe_run_id(r"run\..\secret"));
}

#[test]
fn completed_run_derives_status_stages_and_subagents() {
    let root = tmp_dir("done");
    let run_dir = root.join("done-run");
    fs::create_dir_all(run_dir.join("tasks")).unwrap();
    write_json(
        &run_dir.join("workflow.json"),
        &json!({
            "run_id": "done-run",
            "runtime": "rust",
            "state_schema_version": 1,
            "created_unix_ms": unix_ms(),
            "heartbeat_interval_seconds": 30,
            "global_max_concurrency": 3,
            "provider_max_concurrency": {"mock": 3},
            "task_count": 2,
        }),
    );
    write_json(
        &run_dir.join("tasks").join("0000-a.json"),
        &json!({
            "task_id": "0000-a", "source_id": "a", "agent_id": "a",
            "stage": "Build", "role": "programmer", "status": "completed",
            "provider": "mock", "model": "mock-worker", "subagents": ["b"],
        }),
    );
    write_json(
        &run_dir.join("tasks").join("0001-b.json"),
        &json!({
            "task_id": "0001-b", "source_id": "b", "agent_id": "b",
            "parent_task_id": "a", "stage": "Build", "role": "verifier",
            "status": "completed", "provider": "mock", "model": "mock-worker",
        }),
    );

    let mut reader = RunReader::open(&root, "done-run", Vec::new()).unwrap();
    let contract = reader.read();

    assert_eq!(contract.run.status, RunStatus::Completed);
    assert_eq!(contract.run.run_id, "done-run");
    assert_eq!(contract.run.runtime, "rust");
    assert_eq!(contract.run.global_max_concurrency, Some(3));
    assert_eq!(
        contract.run.provider_max_concurrency.get("mock").copied(),
        Some(3)
    );
    assert_eq!(contract.stages.len(), 1);
    assert_eq!(contract.stages[0].name, "Build");
    assert_eq!(contract.stages[0].tasks.len(), 2);
    let parent = contract.stages[0]
        .tasks
        .iter()
        .find(|t| t.task_id == "0000-a")
        .unwrap();
    assert_eq!(parent.agent.agent_id, "a");
    assert_eq!(parent.subagents.len(), 1);
    assert_eq!(parent.subagents[0].agent_id, "b");
    assert_eq!(parent.subagents[0].status, "completed");
    fs::remove_dir_all(root).ok();
}

#[test]
fn running_partial_and_failed_derivation() {
    let root = tmp_dir("states");
    let run_dir = root.join("s");
    fs::create_dir_all(run_dir.join("tasks")).unwrap();
    write_json(
        &run_dir.join("workflow.json"),
        &json!({"run_id": "s", "runtime": "rust"}),
    );
    write_json(
        &run_dir.join("tasks").join("0000-x.json"),
        &json!({"task_id": "0000-x", "status": "in_progress", "stage": "S", "heartbeat_unix_ms": unix_ms()}),
    );

    let mut reader = RunReader::open(&root, "s", Vec::new()).unwrap();
    assert_eq!(reader.read().run.status, RunStatus::Running);

    fs::remove_file(run_dir.join("tasks").join("0000-x.json")).unwrap();
    write_json(
        &run_dir.join("tasks").join("0001-y.json"),
        &json!({"task_id": "0001-y", "status": "blocked", "stage": "S"}),
    );
    assert_eq!(reader.read().run.status, RunStatus::Partial);

    fs::remove_file(run_dir.join("tasks").join("0001-y.json")).unwrap();
    write_json(
        &run_dir.join("tasks").join("0002-z.json"),
        &json!({"task_id": "0002-z", "status": "failed", "stage": "S"}),
    );
    assert_eq!(reader.read().run.status, RunStatus::Failed);
    fs::remove_dir_all(root).ok();
}

#[test]
fn flatten_orders_stage_before_tasks_and_marks_stale() {
    let root = tmp_dir("flatten");
    let run_dir = root.join("f");
    fs::create_dir_all(run_dir.join("tasks")).unwrap();
    write_json(
        &run_dir.join("workflow.json"),
        &json!({"run_id": "f", "runtime": "rust", "heartbeat_interval_seconds": 1}),
    );
    let old = unix_ms().saturating_sub(10_000);
    write_json(
        &run_dir.join("tasks").join("0000-t.json"),
        &json!({"task_id": "0000-t", "status": "in_progress", "stage": "Build", "heartbeat_unix_ms": old, "model": "glm"}),
    );

    let mut reader = RunReader::open(&root, "f", Vec::new()).unwrap();
    let contract = reader.read();
    let rows = flatten(&contract, unix_ms(), "");

    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].kind, RowKind::Stage);
    assert_eq!(rows[0].label, "Build");
    assert_eq!(rows[1].kind, RowKind::Task);
    assert!(rows[1].stale);

    let miss = flatten(&contract, unix_ms(), "nomatch");
    assert!(miss.is_empty());

    let hit = flatten(&contract, unix_ms(), "0000");
    assert_eq!(hit.len(), 2);
    fs::remove_dir_all(root).ok();
}

#[test]
fn events_tail_consumes_only_complete_new_lines() {
    let root = tmp_dir("events");
    let run_dir = root.join("e");
    fs::create_dir_all(&run_dir).unwrap();
    let path = run_dir.join("events.jsonl");
    fs::write(&path, "{\"event\":\"task_started\",\"task_id\":\"a\"}\n").unwrap();

    let mut reader = RunReader::open(&root, "e", Vec::new()).unwrap();
    let first = reader.tail_events(10);
    assert_eq!(first.len(), 1);
    assert_eq!(first[0].event, "task_started");

    let mut content = fs::read_to_string(&path).unwrap();
    content.push_str("{\"event\":\"task_finished\",\"task_id\":\"a\"}\n");
    content.push_str("{\"event\":\"task_started\",\"task_id\":\"b\"}");
    fs::write(&path, &content).unwrap();

    let second = reader.tail_events(10);
    assert_eq!(second.len(), 1, "incomplete trailing line stays buffered");
    assert_eq!(second[0].event, "task_finished");

    let mut content = fs::read_to_string(&path).unwrap();
    content.push('\n');
    fs::write(&path, &content).unwrap();
    let third = reader.tail_events(10);
    assert_eq!(third.len(), 1);
    assert_eq!(third[0].task_id.as_deref(), Some("b"));
    fs::remove_dir_all(root).ok();
}

#[test]
fn events_tail_resets_when_file_truncated() {
    let root = tmp_dir("trunc");
    let run_dir = root.join("t");
    fs::create_dir_all(&run_dir).unwrap();
    let path = run_dir.join("events.jsonl");
    fs::write(
        &path,
        "{\"event\":\"a\"}\n{\"event\":\"b\"}\n{\"event\":\"c\"}\n",
    )
    .unwrap();

    let mut reader = RunReader::open(&root, "t", Vec::new()).unwrap();
    let first = reader.tail_events(100);
    assert_eq!(first.len(), 3);

    // Replace the file with a shorter one: offset now exceeds file size.
    fs::write(&path, "{\"event\":\"x\"}\n").unwrap();
    let got = reader.tail_events(100);
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].event, "x");
    fs::remove_dir_all(root).ok();
}

#[test]
fn events_tail_sanitizes_error_fields() {
    let root = tmp_dir("event-error");
    let run_dir = root.join("e");
    fs::create_dir_all(&run_dir).unwrap();
    fs::write(
        run_dir.join("events.jsonl"),
        r#"{"event":"task_failed","error":"Bearer secret-token at C:\\tmp\\worker.log"}
"#,
    )
    .unwrap();

    let mut reader = RunReader::open(&root, "e", Vec::new()).unwrap();
    let events = reader.tail_events(10);

    assert_eq!(events.len(), 1);
    let error = events[0].error.as_deref().unwrap();
    assert_eq!(error, "Bearer *** at C:/tmp/worker.log");
    assert!(!error.contains("secret-token"));
    fs::remove_dir_all(root).ok();
}

#[test]
fn foreign_absolute_artifact_collapses_to_basename() {
    let root = tmp_dir("art");
    let run_dir = root.join("a");
    fs::create_dir_all(run_dir.join("tasks")).unwrap();
    write_json(
        &run_dir.join("workflow.json"),
        &json!({"run_id": "a", "runtime": "rust"}),
    );
    // Use an OS-absolute foreign path so sanitize_path collapses it on every
    // platform (a rooted-but-driveless path like "/tmp/x" is NOT absolute on
    // Windows and would be preserved verbatim).
    let foreign = if cfg!(windows) {
        "C:\\Windows\\Temp\\foreign\\secret.log"
    } else {
        "/tmp/foreign/secret.log"
    };
    write_json(
        &run_dir.join("tasks").join("0000-a.json"),
        &json!({
            "task_id": "0000-a", "status": "completed", "stage": "S",
            "artifacts": [foreign, "docs/x.md"],
        }),
    );

    let mut reader = RunReader::open(&root, "a", vec![root.clone()]).unwrap();
    let c = reader.read();
    let arts = &c.stages[0].tasks[0].artifacts;
    assert!(
        arts.iter().any(|a| a == "secret.log"),
        "foreign path -> basename"
    );
    assert!(
        arts.iter().any(|a| a == "docs/x.md"),
        "relative path preserved"
    );
    fs::remove_dir_all(root).ok();
}

#[test]
fn worker_log_tail_is_capped_to_configured_limit() {
    let root = tmp_dir("log");
    let run_dir = root.join("l");
    let task_id = "0000-t";
    let dir = run_dir.join("results").join(task_id);
    fs::create_dir_all(&dir).unwrap();
    let big = "x".repeat((MAX_LOG_BYTES as usize) + 1000);
    fs::write(dir.join("worker.log"), big.as_bytes()).unwrap();

    let tail = read_worker_log_tail(&run_dir, task_id).expect("log present");
    assert_eq!(tail.len(), MAX_LOG_BYTES as usize);
    fs::remove_dir_all(root).ok();
}

#[test]
fn worker_log_missing_returns_none() {
    let root = tmp_dir("nolog");
    assert!(read_worker_log_tail(&root, "ghost").is_none());
    fs::remove_dir_all(root).ok();
}

#[test]
fn sanitize_error_redacts_and_truncates() {
    let secret =
        sanitize_error(Some(&json!("Bearer abcdef123 and sk-1234567890xyz leak"))).unwrap();
    assert!(!secret.contains("abcdef123"));
    assert!(!secret.contains("xyz"));
    assert!(secret.contains("***"));

    let long = json!("a".repeat(MAX_ERROR_CHARS + 50));
    let capped = sanitize_error(Some(&long)).unwrap();
    assert!(capped.ends_with("...[truncated]"));
    assert!(capped.len() <= MAX_ERROR_CHARS + "...[truncated]".len());
}

#[test]
fn path_helpers_relativize() {
    // Relative path preserved (slash-normalized) on every OS.
    let already = sanitize_path("docs/y.md", &[]).unwrap();
    assert_eq!(already, "docs/y.md");

    // An OS-absolute foreign path collapses to its basename on every OS.
    let foreign = if cfg!(windows) {
        "C:\\Windows\\Temp\\foreign.log"
    } else {
        "/tmp/foreign.log"
    };
    let base = sanitize_path(foreign, &[PathBuf::from(".")]).unwrap();
    assert_eq!(base, "foreign.log");
}

#[test]
fn list_runs_orders_active_first_then_by_recency() {
    let root = tmp_dir("list");
    let active = root.join("active");
    fs::create_dir_all(active.join("tasks")).unwrap();
    write_json(
        &active.join("workflow.json"),
        &json!({"run_id": "active", "runtime": "rust", "created_unix_ms": 1000}),
    );
    let finished = root.join("finished");
    fs::create_dir_all(finished.join("tasks")).unwrap();
    write_json(
        &finished.join("workflow.json"),
        &json!({"run_id": "finished", "runtime": "rust", "created_unix_ms": 2000}),
    );
    write_json(
        &finished.join("report.json"),
        &json!({"status": "completed"}),
    );

    let runs = list_runs(&root);
    assert_eq!(runs.len(), 2);
    assert_eq!(runs[0].run_id, "active");
    assert!(!runs[0].has_report);
    assert_eq!(runs[1].run_id, "finished");
    assert!(runs[1].has_report);
    fs::remove_dir_all(root).ok();
}

#[test]
fn runs_are_grouped_by_project_with_legacy_fallback() {
    let root = tmp_dir("projects");
    for (run_id, workflow) in [
        (
            "alpha-1",
            json!({"run_id":"alpha-1","project_id":"alpha","project_name":"Alpha"}),
        ),
        (
            "alpha-2",
            json!({"run_id":"alpha-2","project_id":"alpha","project_name":"Alpha"}),
        ),
        (
            "workspace-old",
            json!({"run_id":"workspace-old","workspace_root":"C:/work/Beta"}),
        ),
        ("legacy", json!({"run_id":"legacy"})),
    ] {
        let run_dir = root.join(run_id);
        fs::create_dir_all(run_dir.join("tasks")).unwrap();
        write_json(&run_dir.join("workflow.json"), &workflow);
    }
    let runs = list_runs(&root);
    let groups = group_runs(&runs);
    assert_eq!(groups.len(), 3);
    assert_eq!(
        groups
            .iter()
            .find(|group| group.project_id == "alpha")
            .unwrap()
            .runs
            .len(),
        2
    );
    assert_eq!(
        groups
            .iter()
            .find(|group| group.project_name == "Beta")
            .unwrap()
            .runs[0]
            .run_id,
        "workspace-old"
    );
    assert!(groups.iter().any(|group| group.project_id == "legacy"));
    fs::remove_dir_all(root).ok();
}

#[test]
fn report_drives_terminal_status() {
    let root = tmp_dir("report");
    let run_dir = root.join("r");
    fs::create_dir_all(run_dir.join("tasks")).unwrap();
    write_json(
        &run_dir.join("workflow.json"),
        &json!({"run_id": "r", "runtime": "rust"}),
    );
    write_json(
        &run_dir.join("tasks").join("0000-a.json"),
        &json!({"task_id": "0000-a", "status": "completed", "stage": "S"}),
    );
    write_json(
        &run_dir.join("report-rs.json"),
        &json!({"status": "failed"}),
    );

    let mut reader = RunReader::open(&root, "r", Vec::new()).unwrap();
    let contract = reader.read();
    assert_eq!(contract.run.status, RunStatus::Failed);
    assert_eq!(contract.summary.report_status.as_deref(), Some("failed"));
    fs::remove_dir_all(root).ok();
}
