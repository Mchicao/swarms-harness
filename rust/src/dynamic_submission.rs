//! Durable, runtime-owned admission queue for tasks submitted while a run is active.
//!
//! External planners may propose more work, but they do not mutate scheduler
//! state directly. The coordinator re-runs the ordinary plan review before an
//! accepted submission becomes a schedulable task.

use crate::config;
use crate::model::{Plan, Router, Stage, Task, TaskSpec};
use crate::review;
use serde::Deserialize;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

type Result<T> = std::result::Result<T, String>;

const SUBMISSION_VERSION: u32 = 1;

#[derive(Clone, Debug, Deserialize)]
pub struct TaskSubmission {
    #[serde(default = "default_submission_version")]
    pub submission_version: u32,
    #[serde(default = "default_stage_name")]
    pub stage: String,
    #[serde(default = "default_parallel")]
    pub parallel: bool,
    pub task: TaskSpec,
}

fn default_submission_version() -> u32 {
    SUBMISSION_VERSION
}

fn default_stage_name() -> String {
    "Dynamic".to_string()
}

fn default_parallel() -> bool {
    true
}

#[derive(Debug)]
pub enum SubmissionDecision {
    Accepted { task: Box<Task>, file_name: String },
    Rejected { file_name: String, error: String },
}

fn queue_root(run_dir: &Path) -> PathBuf {
    run_dir.join("submissions")
}

fn queue_dir(run_dir: &Path, state: &str) -> PathBuf {
    queue_root(run_dir).join(state)
}

fn ensure_queue_dirs(run_dir: &Path) -> Result<()> {
    for state in ["pending", "accepted", "rejected"] {
        let path = queue_dir(run_dir, state);
        fs::create_dir_all(&path).map_err(|error| format!("{}: {error}", path.display()))?;
    }
    Ok(())
}

fn parse_submission(path: &Path) -> Result<TaskSubmission> {
    let text = fs::read_to_string(path).map_err(|error| format!("{}: {error}", path.display()))?;
    let submission: TaskSubmission =
        serde_json::from_str(&text).map_err(|error| format!("{}: {error}", path.display()))?;
    if submission.submission_version != SUBMISSION_VERSION {
        return Err(format!(
            "unsupported submission_version {}; expected {}",
            submission.submission_version, SUBMISSION_VERSION
        ));
    }
    if submission.stage.trim().is_empty() {
        return Err("submission stage must not be empty".to_string());
    }
    Ok(submission)
}

fn sorted_json_files(dir: &Path) -> Result<Vec<PathBuf>> {
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut paths = fs::read_dir(dir)
        .map_err(|error| format!("{}: {error}", dir.display()))?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.extension().and_then(|value| value.to_str()) == Some("json"))
        .collect::<Vec<_>>();
    paths.sort();
    Ok(paths)
}

fn validate_stage_policy(current_tasks: &[Task], submission: &TaskSubmission) -> Result<()> {
    if let Some(existing) = current_tasks
        .iter()
        .find(|task| task.stage == submission.stage && task.stage_parallel != submission.parallel)
    {
        return Err(format!(
            "submission stage '{}' declares parallel={} but existing stage uses parallel={}",
            submission.stage, submission.parallel, existing.stage_parallel
        ));
    }
    Ok(())
}

fn validation_plan(plan: &Plan, tasks: &[Task], submission: &TaskSubmission) -> Plan {
    let mut candidate = plan.clone();
    candidate.stages = tasks
        .iter()
        .map(|task| Stage {
            name: task.stage.clone(),
            parallel: task.stage_parallel,
            tasks: vec![task.spec.clone()],
        })
        .collect();
    candidate.stages.push(Stage {
        name: submission.stage.clone(),
        parallel: submission.parallel,
        tasks: vec![submission.task.clone()],
    });
    candidate
}

fn validate_and_build(
    plan: &Plan,
    router: &Router,
    current_tasks: &[Task],
    submission: &TaskSubmission,
) -> Result<Task> {
    validate_stage_policy(current_tasks, submission)?;
    let candidate_plan = validation_plan(plan, current_tasks, submission);
    let candidate_tasks = config::build_tasks(&candidate_plan, router)?;
    let result = review::review_plan(&candidate_plan, router, &candidate_tasks);
    if !result.ok {
        let message = result
            .findings
            .iter()
            .filter(|finding| finding.severity == review::Severity::Error)
            .map(|finding| format!("{}: {}", finding.code, finding.message))
            .collect::<Vec<_>>()
            .join("; ");
        return Err(if message.is_empty() {
            "dynamic task review failed".to_string()
        } else {
            message
        });
    }
    candidate_tasks
        .last()
        .cloned()
        .ok_or_else(|| "dynamic task review produced no candidate task".to_string())
}

/// Persist a submission into a run's pending queue. This does not grant it
/// scheduler authority; admission happens inside `runtime::execute`. A queue
/// may also be populated between attempts and consumed by the next `--resume`.
///
/// The JSON becomes visible to the scheduler only after it is fully written
/// and synced: a non-JSON temporary file is atomically renamed into `pending`.
pub fn enqueue_file(run_dir: &Path, source: &Path) -> Result<PathBuf> {
    if !run_dir.is_dir() {
        return Err(format!(
            "run directory does not exist: {}",
            run_dir.display()
        ));
    }
    ensure_queue_dirs(run_dir)?;
    let submission = parse_submission(source)?;
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let stem = format!(
        "{nanos:020}-{}-{}",
        std::process::id(),
        crate::model::slug(&submission.task.id)
    );
    let pending = queue_dir(run_dir, "pending");
    let temporary = pending.join(format!(".{stem}.tmp"));
    let destination = pending.join(format!("{stem}.json"));
    let content = fs::read(source).map_err(|error| format!("{}: {error}", source.display()))?;
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temporary)
        .map_err(|error| format!("{}: {error}", temporary.display()))?;
    file.write_all(&content)
        .map_err(|error| format!("{}: {error}", temporary.display()))?;
    file.sync_all()
        .map_err(|error| format!("{}: {error}", temporary.display()))?;
    drop(file);
    fs::rename(&temporary, &destination).map_err(|error| {
        let _ = fs::remove_file(&temporary);
        format!(
            "publish submission {} -> {}: {error}",
            temporary.display(),
            destination.display()
        )
    })?;
    Ok(destination)
}

/// Rebuild already accepted dynamic tasks during resume. Accepted files are
/// authoritative queue history, but are revalidated against the current plan
/// so an incompatible plan edit fails closed.
pub fn load_accepted(
    run_dir: &Path,
    plan: &Plan,
    router: &Router,
    initial_tasks: &[Task],
) -> Result<Vec<Task>> {
    ensure_queue_dirs(run_dir)?;
    let mut all = initial_tasks.to_vec();
    let base_len = all.len();
    for path in sorted_json_files(&queue_dir(run_dir, "accepted"))? {
        let submission = parse_submission(&path)?;
        let task = validate_and_build(plan, router, &all, &submission).map_err(|error| {
            format!(
                "accepted submission {} is no longer valid: {error}",
                path.display()
            )
        })?;
        all.push(task);
    }
    Ok(all.split_off(base_len))
}

/// Drain the current pending snapshot in deterministic filename order. New
/// files arriving during the drain remain pending until the next scheduler pass.
pub fn drain_pending(
    run_dir: &Path,
    plan: &Plan,
    router: &Router,
    current_tasks: &[Task],
) -> Result<Vec<SubmissionDecision>> {
    ensure_queue_dirs(run_dir)?;
    let mut tasks = current_tasks.to_vec();
    let mut decisions = Vec::new();

    for path in sorted_json_files(&queue_dir(run_dir, "pending"))? {
        let file_name = path
            .file_name()
            .and_then(|value| value.to_str())
            .ok_or_else(|| format!("invalid submission filename: {}", path.display()))?
            .to_string();
        let decision = parse_submission(&path)
            .and_then(|submission| validate_and_build(plan, router, &tasks, &submission));
        match decision {
            Ok(task) => {
                let destination = queue_dir(run_dir, "accepted").join(&file_name);
                fs::rename(&path, &destination).map_err(|error| {
                    format!(
                        "move {} -> {}: {error}",
                        path.display(),
                        destination.display()
                    )
                })?;
                let task = Box::new(task);
                tasks.push(task.as_ref().clone());
                decisions.push(SubmissionDecision::Accepted { task, file_name });
            }
            Err(error) => {
                let destination = queue_dir(run_dir, "rejected").join(&file_name);
                fs::rename(&path, &destination).map_err(|move_error| {
                    format!(
                        "move {} -> {}: {move_error}",
                        path.display(),
                        destination.display()
                    )
                })?;
                let error_path = destination.with_extension("error.txt");
                fs::write(&error_path, &error)
                    .map_err(|write_error| format!("{}: {write_error}", error_path.display()))?;
                decisions.push(SubmissionDecision::Rejected { file_name, error });
            }
        }
    }

    Ok(decisions)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_run(label: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "swarms-submit-{label}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn enqueue_publishes_only_a_complete_json_file() {
        let run_dir = temp_run("enqueue");
        let source = run_dir.join("task.json");
        fs::write(
            &source,
            r#"{"submission_version":1,"task":{"id":"follow-up","route":"mock","task":"Inspect the result"}}"#,
        )
        .unwrap();
        fs::write(run_dir.join("report.json"), "{}").unwrap();

        let queued = enqueue_file(&run_dir, &source).unwrap();
        assert!(queued.exists());
        assert_eq!(parse_submission(&queued).unwrap().task.id, "follow-up");
        let pending = queue_dir(&run_dir, "pending");
        assert_eq!(sorted_json_files(&pending).unwrap(), vec![queued]);
        assert!(!fs::read_dir(&pending)
            .unwrap()
            .filter_map(|entry| entry.ok())
            .any(|entry| entry.path().extension().and_then(|value| value.to_str()) == Some("tmp")));
        fs::remove_dir_all(run_dir).unwrap();
    }

    #[test]
    fn pending_files_are_sorted_for_deterministic_admission() {
        let run_dir = temp_run("sort");
        let pending = queue_dir(&run_dir, "pending");
        fs::create_dir_all(&pending).unwrap();
        fs::write(pending.join("0002-b.json"), "{}").unwrap();
        fs::write(pending.join("0001-a.json"), "{}").unwrap();

        let names = sorted_json_files(&pending)
            .unwrap()
            .into_iter()
            .map(|path| path.file_name().unwrap().to_string_lossy().to_string())
            .collect::<Vec<_>>();
        assert_eq!(names, vec!["0001-a.json", "0002-b.json"]);
        fs::remove_dir_all(run_dir).unwrap();
    }
}
