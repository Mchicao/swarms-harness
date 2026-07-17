//! Domain model types for SWARMS plans, router config, and runtime tasks.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub type Result<T> = std::result::Result<T, String>;

// ---------------------------------------------------------------------------
// Thinking level
// ---------------------------------------------------------------------------

/// Per-task reasoning depth.  Maps only to verified adapter flags.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum ThinkingLevel {
    #[default]
    Auto,
    Minimal,
    Low,
    Medium,
    High,
    Max,
}

impl ThinkingLevel {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Minimal => "minimal",
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::Max => "max",
        }
    }

    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }

    /// Translate the common level to Codex's verified config vocabulary.
    pub fn as_codex_str(&self) -> Option<&'static str> {
        match self {
            Self::Auto => None,
            Self::Minimal => Some("minimal"),
            Self::Low => Some("low"),
            Self::Medium => Some("medium"),
            Self::High => Some("high"),
            Self::Max => Some("ultra"),
        }
    }
}

// ---------------------------------------------------------------------------
// Session configuration
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum SessionMode {
    #[default]
    Disabled,
    New,
    Reuse,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum OnMissing {
    #[default]
    New,
    Fail,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct SessionConfig {
    #[serde(default)]
    pub mode: SessionMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    #[serde(default, skip_serializing_if = "OnMissing::is_default")]
    pub on_missing: OnMissing,
}

impl OnMissing {
    pub fn is_default(&self) -> bool {
        *self == OnMissing::default()
    }
}

// ---------------------------------------------------------------------------
// Router / Provider
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Deserialize)]
pub struct Provider {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub provider: String,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub canonical_model: Option<String>,
    #[serde(default)]
    pub wrapper: String,
    /// OpenAI-compat: env var holding the API key.
    #[serde(default)]
    pub key_env: Option<String>,
    /// OpenAI-compat: explicit base URL.
    #[serde(default)]
    pub base_url: Option<String>,
    /// OpenAI-compat: env var holding the base URL.
    #[serde(default)]
    pub base_url_env: Option<String>,
    /// OpenAI-compat: JSON request field for reasoning level.
    #[serde(default)]
    pub thinking_field: Option<String>,
    /// Key in the external quota snapshot (for example `codex:Codex`).
    #[serde(default)]
    pub quota_key: Option<String>,
    /// Ordered routes to try when this provider is unavailable or quota-limited.
    #[serde(default)]
    pub fallback_routes: Vec<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum OnUnknownQuota {
    Allow,
    #[default]
    Block,
}

#[derive(Clone, Debug, Deserialize)]
pub struct QuotaPolicy {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_quota_snapshot_path")]
    pub snapshot_path: String,
    #[serde(default = "default_min_remaining_percent")]
    pub min_remaining_percent: f64,
    #[serde(default = "default_quota_max_age_seconds")]
    pub max_age_seconds: u64,
    #[serde(default)]
    pub on_unknown: OnUnknownQuota,
}

impl Default for QuotaPolicy {
    fn default() -> Self {
        Self {
            enabled: false,
            snapshot_path: default_quota_snapshot_path(),
            min_remaining_percent: default_min_remaining_percent(),
            max_age_seconds: default_quota_max_age_seconds(),
            on_unknown: OnUnknownQuota::Block,
        }
    }
}

fn default_quota_snapshot_path() -> String {
    "../ai-usage-monitor/quota_snapshot.json".to_string()
}

fn default_min_remaining_percent() -> f64 {
    10.0
}

fn default_quota_max_age_seconds() -> u64 {
    600
}

#[derive(Clone, Debug, Deserialize)]
pub struct Router {
    #[serde(default)]
    pub fallback_route: Option<String>,
    #[serde(default)]
    pub aliases: HashMap<String, String>,
    #[serde(default)]
    pub role_routes: HashMap<String, String>,
    #[serde(default)]
    pub quota_policy: QuotaPolicy,
    pub providers: HashMap<String, Provider>,
}

impl Router {
    /// Resolve a possibly-aliased route id to a concrete provider key.
    pub fn resolve_route<'a>(&'a self, route: &'a str) -> &'a str {
        if let Some(alias) = self.aliases.get(route) {
            alias.as_str()
        } else {
            route
        }
    }

    pub fn get_provider(&self, route: &str) -> Option<&Provider> {
        let resolved = self.resolve_route(route);
        self.providers.get(resolved)
    }
}

// ---------------------------------------------------------------------------
// Plan
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Deserialize)]
pub struct ProjectConfig {
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct PlannerConfig {
    #[serde(default)]
    pub route: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct ReviewPolicy {
    #[serde(default)]
    pub static_review_required: Option<bool>,
    #[serde(default)]
    pub critic_review_required: Option<bool>,
    #[serde(default)]
    pub premium_allowed: Option<bool>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Plan {
    #[serde(default)]
    pub schema_version: Option<u32>,
    #[serde(default)]
    pub goal: Option<String>,
    /// Stable UI grouping metadata. It never affects run-directory paths.
    #[serde(default)]
    pub project: Option<ProjectConfig>,
    #[serde(default)]
    pub planner: Option<PlannerConfig>,
    #[serde(default)]
    pub review_policy: Option<ReviewPolicy>,
    #[serde(default)]
    pub budget_policy: BudgetPolicy,
    pub stages: Vec<Stage>,
    /// Plan-level default thinking (overridable per task).
    #[serde(default)]
    pub thinking: Option<ThinkingLevel>,
    /// Plan-level default session config.
    #[serde(default)]
    pub session: Option<SessionConfig>,
    #[serde(default)]
    pub default_timeout_seconds: Option<u64>,
    #[serde(default)]
    pub default_max_attempts: Option<u32>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct BudgetPolicy {
    #[serde(default = "default_max_total_workers")]
    pub max_total_workers: usize,
    #[serde(default = "default_global_concurrency")]
    pub global_max_concurrency: usize,
    #[serde(default)]
    pub provider_concurrency: HashMap<String, usize>,
}

impl Default for BudgetPolicy {
    fn default() -> Self {
        Self {
            max_total_workers: default_max_total_workers(),
            global_max_concurrency: default_global_concurrency(),
            provider_concurrency: HashMap::new(),
        }
    }
}

fn default_max_total_workers() -> usize {
    1000
}

fn default_global_concurrency() -> usize {
    8
}

#[derive(Clone, Debug, Deserialize)]
pub struct Stage {
    #[serde(default = "default_stage_name")]
    pub name: String,
    #[serde(default = "default_true")]
    pub parallel: bool,
    pub tasks: Vec<TaskSpec>,
}

fn default_stage_name() -> String {
    "Unnamed".to_string()
}

fn default_true() -> bool {
    true
}

// ---------------------------------------------------------------------------
// Task specification (from plan JSON)
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Deserialize)]
pub struct TaskSpec {
    pub id: String,
    pub route: String,
    pub task: String,
    #[serde(default = "default_role")]
    pub role: String,
    #[serde(default)]
    pub needs: Vec<String>,
    #[serde(default = "default_tools_policy")]
    pub tools_policy: String,
    #[serde(default)]
    pub artifacts: Vec<String>,
    #[serde(default)]
    pub verify: Vec<String>,
    /// Per-task thinking level (overrides plan default).
    #[serde(default)]
    pub thinking: Option<ThinkingLevel>,
    /// Per-task session config (overrides plan default).
    #[serde(default)]
    pub session: Option<SessionConfig>,
    #[serde(default)]
    pub timeout_seconds: Option<u64>,
    #[serde(default)]
    pub max_attempts: Option<u32>,
}

fn default_role() -> String {
    "general".to_string()
}

fn default_tools_policy() -> String {
    "none".to_string()
}

impl TaskSpec {
    pub fn effective_thinking(&self, plan: &Plan) -> ThinkingLevel {
        self.thinking.or(plan.thinking).unwrap_or_default()
    }

    pub fn effective_session(&self, plan: &Plan) -> SessionConfig {
        self.session
            .clone()
            .or_else(|| plan.session.clone())
            .unwrap_or_default()
    }

    pub fn effective_timeout(&self, plan: &Plan) -> u64 {
        self.timeout_seconds
            .or(plan.default_timeout_seconds)
            .unwrap_or(600)
    }

    pub fn effective_max_attempts(&self, plan: &Plan) -> u32 {
        self.max_attempts.or(plan.default_max_attempts).unwrap_or(1)
    }
}

// ---------------------------------------------------------------------------
// Resolved task (internal runtime type)
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct Task {
    pub id: String,
    pub source_id: String,
    pub stage: String,
    pub stage_parallel: bool,
    pub spec: TaskSpec,
    pub provider: Provider,
    /// Concrete route selected by the scheduler; `spec.route` remains requested route.
    pub effective_route: String,
}

// ---------------------------------------------------------------------------
// Utility functions
// ---------------------------------------------------------------------------

/// Sanitise an arbitrary string into a filesystem-safe slug.
pub fn slug(value: &str) -> String {
    let clean: String = value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-') {
                ch
            } else {
                '-'
            }
        })
        .collect();
    let trimmed = clean.trim_matches('-');
    if trimmed.is_empty() {
        "task".to_string()
    } else {
        trimmed.chars().take(80).collect()
    }
}

/// Resolve a dependency reference against a set of tasks.
/// A dependency matches by source_id, task id, or slugified id suffix.
pub fn find_dependency_task<'a>(tasks: &'a [Task], dep: &str) -> Option<&'a Task> {
    tasks
        .iter()
        .find(|t| t.source_id == dep || t.id == dep)
        .or_else(|| {
            let key = slug(dep).to_lowercase();
            tasks.iter().find(|t| {
                if let Some(suffix) = t.id.split_once('-').map(|(_, s)| s) {
                    slug(suffix).to_lowercase() == key
                } else {
                    false
                }
            })
        })
}

pub fn task_index_to_id(index: usize, source_id: &str) -> String {
    format!("{index:04}-{}", slug(source_id))
}
