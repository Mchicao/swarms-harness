//! Token usage normalisation, task results, and report generation.

use crate::model::ThinkingLevel;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// Usage
// ---------------------------------------------------------------------------

/// Normalised token usage.  Fields use the string `"missing"` when the
/// adapter did not report telemetry — never fabricated zeros.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Usage {
    #[serde(default = "default_missing")]
    pub input: String,
    #[serde(default = "default_missing")]
    pub cache_read: String,
    #[serde(default = "default_missing")]
    pub cache_write: String,
    #[serde(default = "default_missing")]
    pub output: String,
    #[serde(default = "default_missing")]
    pub reasoning: String,
}

fn default_missing() -> String {
    "missing".to_string()
}

impl Usage {
    pub fn missing() -> Self {
        Self {
            input: "missing".to_string(),
            cache_read: "missing".to_string(),
            cache_write: "missing".to_string(),
            output: "missing".to_string(),
            reasoning: "missing".to_string(),
        }
    }

    pub fn offline_mock() -> Self {
        Self {
            input: "0".to_string(),
            cache_read: "0".to_string(),
            cache_write: "0".to_string(),
            output: "0".to_string(),
            reasoning: "0".to_string(),
        }
    }
}

// ---------------------------------------------------------------------------
// Task status
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Pending,
    Queued,
    InProgress,
    Completed,
    Failed,
    Blocked,
}

impl TaskStatus {
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Blocked)
    }
    pub fn is_completed(&self) -> bool {
        *self == Self::Completed
    }
    pub fn is_failed(&self) -> bool {
        matches!(self, Self::Failed | Self::Blocked)
    }
}

// ---------------------------------------------------------------------------
// Task state (persisted per task)
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TaskState {
    pub task_id: String,
    pub source_id: String,
    pub status: TaskStatus,
    #[serde(default)]
    pub attempts: u32,
    pub stage: String,
    /// Route requested by the plan.
    pub route: String,
    /// Concrete route used after quota/capacity fallback.
    #[serde(default)]
    pub effective_route: String,
    pub provider: String,
    pub model: String,
    #[serde(default)]
    pub role: String,
    #[serde(default)]
    pub thinking: Option<ThinkingLevel>,
    #[serde(default)]
    pub duration_ms: u128,
    #[serde(default)]
    pub session_created: bool,
    #[serde(default)]
    pub session_reused: bool,
    #[serde(default)]
    pub session_resume_count: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verified: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verify_error: Option<String>,
    #[serde(default)]
    pub usage: Usage,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub heartbeat_unix_ms: Option<u128>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ended_at: Option<String>,
    /// Stable hash of the task definition used to validate resume checkpoints.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checkpoint_key: Option<String>,
}

impl TaskState {
    pub fn new(task_id: &str, source_id: &str, stage: &str, route: &str) -> Self {
        Self {
            task_id: task_id.to_string(),
            source_id: source_id.to_string(),
            status: TaskStatus::Pending,
            attempts: 0,
            stage: stage.to_string(),
            route: route.to_string(),
            effective_route: route.to_string(),
            provider: String::new(),
            model: String::new(),
            role: "general".to_string(),
            thinking: None,
            duration_ms: 0,
            session_created: false,
            session_reused: false,
            session_resume_count: 0,
            session_id: None,
            verified: None,
            verify_error: None,
            usage: Usage::missing(),
            error: None,
            started_at: None,
            heartbeat_unix_ms: None,
            ended_at: None,
            checkpoint_key: None,
        }
    }
}

// ---------------------------------------------------------------------------
// Report
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Serialize)]
pub struct Report {
    pub run_id: String,
    pub status: String,
    pub run_dir: String,
    pub task_counts: HashMap<String, usize>,
    pub global_max_concurrency: usize,
    pub provider_max_concurrency: HashMap<String, usize>,
    pub results: Vec<TaskState>,
    pub token_usage: Usage,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub errors: Vec<String>,
}

impl Report {
    pub fn is_completed(&self) -> bool {
        self.status == "completed"
    }
}

pub fn build_report(
    run_id: &str,
    run_dir: &str,
    states: &[TaskState],
    global_cap: usize,
    caps: &HashMap<String, usize>,
    errors: Vec<String>,
) -> Report {
    let mut counts: HashMap<String, usize> = HashMap::new();
    let mut aggregate = Usage::missing();
    let all_mock = states.iter().all(|s| s.provider == "mock");

    for s in states {
        let key = format!("{:?}", s.status).to_lowercase();
        *counts.entry(key).or_default() += 1;
    }

    if all_mock {
        aggregate = Usage::offline_mock();
    } else {
        for s in states {
            merge_usage(&mut aggregate, &s.usage);
        }
    }

    let all_ok = states.iter().all(|s| s.status == TaskStatus::Completed);

    let status = if all_ok && !states.is_empty() {
        "completed".to_string()
    } else {
        "failed".to_string()
    };

    Report {
        run_id: run_id.to_string(),
        status,
        run_dir: run_dir.to_string(),
        task_counts: counts,
        global_max_concurrency: global_cap,
        provider_max_concurrency: caps.clone(),
        results: states.to_vec(),
        token_usage: aggregate,
        errors,
    }
}

fn merge_usage(agg: &mut Usage, other: &Usage) {
    for (dst, src) in [
        &mut agg.input,
        &mut agg.cache_read,
        &mut agg.cache_write,
        &mut agg.output,
        &mut agg.reasoning,
    ]
    .into_iter()
    .zip([
        &other.input,
        &other.cache_read,
        &other.cache_write,
        &other.output,
        &other.reasoning,
    ]) {
        if let (Ok(a), Ok(b)) = (dst.parse::<u64>(), src.parse::<u64>()) {
            *dst = (a + b).to_string();
        } else if *dst == "missing" && src != "missing" {
            *dst = src.clone();
        }
    }
}
