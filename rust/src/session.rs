//! Session affinity store: persistent registry, locking, and reuse validation.

use crate::model::{OnMissing, SessionConfig, SessionMode};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

type Result<T> = std::result::Result<T, String>;

// ---------------------------------------------------------------------------
// Session entry
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SessionEntry {
    pub key: String,
    pub provider_session_id: String,
    pub route: String,
    pub model: String,
    pub adapter: String,
    pub workspace: String,
    pub created_at: String,
    #[serde(default)]
    pub reused_count: u32,
}

// ---------------------------------------------------------------------------
// Session store
// ---------------------------------------------------------------------------

pub struct SessionStore {
    path: PathBuf,
    data: Mutex<SessionData>,
}

#[derive(Default, Serialize, Deserialize)]
struct SessionData {
    #[serde(default)]
    sessions: HashMap<String, SessionEntry>,
}

impl SessionStore {
    pub fn open(run_dir: &Path) -> Result<Self> {
        let path = run_dir.join("sessions.json");
        let data = if path.exists() {
            let text =
                fs::read_to_string(&path).map_err(|e| format!("read {}: {e}", path.display()))?;
            serde_json::from_str(&text).unwrap_or_default()
        } else {
            SessionData::default()
        };
        Ok(Self {
            path,
            data: Mutex::new(data),
        })
    }

    pub fn get(&self, key: &str) -> Option<SessionEntry> {
        self.data.lock().ok()?.sessions.get(key).cloned()
    }

    pub fn put(&self, entry: SessionEntry) -> Result<()> {
        let mut data = self
            .data
            .lock()
            .map_err(|e| format!("session store lock: {e}"))?;
        data.sessions.insert(entry.key.clone(), entry);
        self.persist(&data)
    }

    /// Check whether an existing session can be safely reused.
    pub fn validate_reuse(
        &self,
        key: &str,
        route: &str,
        model: &str,
        adapter: &str,
        workspace: &str,
    ) -> Result<Option<SessionEntry>> {
        match self.get(key) {
            None => Ok(None),
            Some(entry)
                if entry.route == route
                    && entry.model == model
                    && entry.adapter == adapter
                    && entry.workspace == workspace =>
            {
                Ok(Some(entry))
            }
            Some(entry) => Err(format!(
                "session key '{}' was created with route='{}' model='{}' adapter='{}' workspace='{}' \
                 but current task requests route='{}' model='{}' adapter='{}' workspace='{}'; \
                 refusing to reuse mismatched session",
                key, entry.route, entry.model, entry.adapter, entry.workspace,
                route, model, adapter, workspace
            )),
        }
    }

    fn persist(&self, data: &SessionData) -> Result<()> {
        let json = serde_json::to_string_pretty(data).map_err(|e| e.to_string())?;
        let tmp = self.path.with_extension("json.tmp");
        fs::write(&tmp, &json).map_err(|e| format!("write {}: {e}", tmp.display()))?;
        fs::rename(&tmp, &self.path).map_err(|e| format!("rename {:?}: {e}", self.path))
    }
}

// ---------------------------------------------------------------------------
// Session decision logic
// ---------------------------------------------------------------------------

/// Decide what to do for a task's session configuration.
pub enum SessionDecision {
    /// No session interaction needed.
    Skip,
    /// Start a new session; capture ID from output.
    New,
    /// Reuse an existing session with this provider ID.
    Reuse(String),
    /// Fail: a required session could not be established.
    Fail(String),
}

pub fn decide(
    config: &SessionConfig,
    store: &SessionStore,
    route: &str,
    model: &str,
    adapter: &str,
    workspace: &str,
) -> Result<SessionDecision> {
    let key = match config.key.as_deref() {
        Some(k) if !k.is_empty() => k,
        _ => return Ok(SessionDecision::Skip),
    };

    match config.mode {
        SessionMode::Disabled => Ok(SessionDecision::Skip),
        SessionMode::New => Ok(SessionDecision::New),
        SessionMode::Reuse => match store.validate_reuse(key, route, model, adapter, workspace)? {
            Some(entry) => Ok(SessionDecision::Reuse(entry.provider_session_id)),
            None => match config.on_missing {
                OnMissing::New => Ok(SessionDecision::New),
                OnMissing::Fail => Ok(SessionDecision::Fail(format!(
                    "session key '{key}' has no prior session and on_missing=fail"
                ))),
            },
        },
    }
}

pub fn now_iso() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    iso_from_epoch(secs)
}

pub(crate) fn iso_from_epoch(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let seconds = secs % 86_400;
    let z = days + 719_468;
    let era = z / 146_097;
    let day_of_era = z - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let mut year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = month_prime + if month_prime < 10 { 3 } else { -9 };
    year += i64::from(month <= 2);
    let hour = seconds / 3_600;
    let minute = (seconds % 3_600) / 60;
    let second = seconds % 60;
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}
