//! Read-only quota snapshot guard used by the deterministic scheduler.

use crate::model::{OnUnknownQuota, Provider, QuotaPolicy};
use serde::Deserialize;
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

type Result<T> = std::result::Result<T, String>;

#[derive(Debug, Deserialize)]
struct Snapshot {
    generated_at_epoch: u64,
    quotas: HashMap<String, QuotaEntry>,
}

#[derive(Debug, Deserialize)]
struct QuotaEntry {
    #[serde(default)]
    windows: HashMap<String, f64>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct QuotaSnapshotView {
    pub generated_at_epoch: u64,
    pub entries: Vec<QuotaViewEntry>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct QuotaViewEntry {
    pub key: String,
    pub windows: BTreeMap<String, f64>,
}

/// Load the sanitized quota values used by the scheduler. The snapshot contract
/// intentionally contains no credentials, account identifiers or auth data.
pub fn load_snapshot_view(path: &Path) -> Result<QuotaSnapshotView> {
    let text = fs::read_to_string(path)
        .map_err(|error| format!("quota snapshot '{}': {error}", path.display()))?;
    let snapshot: Snapshot = serde_json::from_str(&text)
        .map_err(|error| format!("quota snapshot '{}': {error}", path.display()))?;
    let mut entries: Vec<QuotaViewEntry> = snapshot
        .quotas
        .into_iter()
        .map(|(key, entry)| QuotaViewEntry {
            key,
            windows: entry.windows.into_iter().collect(),
        })
        .collect();
    entries.sort_by(|left, right| left.key.cmp(&right.key));
    Ok(QuotaSnapshotView {
        generated_at_epoch: snapshot.generated_at_epoch,
        entries,
    })
}

#[derive(Debug)]
pub struct QuotaGuard {
    policy: QuotaPolicy,
    snapshot: Option<Snapshot>,
    load_error: Option<String>,
}

impl QuotaGuard {
    pub fn load(root: &Path, policy: &QuotaPolicy) -> Self {
        if !policy.enabled {
            return Self {
                policy: policy.clone(),
                snapshot: None,
                load_error: None,
            };
        }
        let configured = Path::new(&policy.snapshot_path);
        let path = if configured.is_absolute() {
            configured.to_path_buf()
        } else {
            root.join(configured)
        };
        let loaded = fs::read_to_string(&path)
            .map_err(|e| format!("quota snapshot '{}': {e}", path.display()))
            .and_then(|text| {
                serde_json::from_str(&text)
                    .map_err(|e| format!("quota snapshot '{}': {e}", path.display()))
            });
        match loaded {
            Ok(snapshot) => Self {
                policy: policy.clone(),
                snapshot: Some(snapshot),
                load_error: None,
            },
            Err(error) => Self {
                policy: policy.clone(),
                snapshot: None,
                load_error: Some(error),
            },
        }
    }

    pub fn check(&self, provider: &Provider) -> Result<()> {
        if !self.policy.enabled || provider.quota_key.is_none() {
            return Ok(());
        }
        let key = provider.quota_key.as_deref().unwrap_or_default();
        let Some(snapshot) = self.snapshot.as_ref() else {
            return self.unknown(
                self.load_error
                    .as_deref()
                    .unwrap_or("quota snapshot unavailable"),
            );
        };
        let now = now_epoch();
        if snapshot.generated_at_epoch > now.saturating_add(self.policy.max_age_seconds) {
            return self.unknown("quota snapshot timestamp is too far in the future");
        }
        let age = now.saturating_sub(snapshot.generated_at_epoch);
        if age > self.policy.max_age_seconds {
            return self.unknown(&format!(
                "quota snapshot is stale ({age}s > {}s)",
                self.policy.max_age_seconds
            ));
        }
        let Some(entry) = snapshot.quotas.get(key) else {
            return self.unknown(&format!("quota key '{key}' is missing"));
        };
        if entry.windows.is_empty() {
            return self.unknown(&format!("quota key '{key}' has no known windows"));
        }
        if let Some((window, remaining)) = entry.windows.iter().find(|(_, remaining)| {
            !remaining.is_finite() || **remaining < self.policy.min_remaining_percent
        }) {
            return Err(format!(
                "quota '{key}' window {window} has {remaining:.1}% remaining (minimum {:.1}%)",
                self.policy.min_remaining_percent
            ));
        }
        Ok(())
    }

    fn unknown(&self, reason: &str) -> Result<()> {
        match self.policy.on_unknown {
            OnUnknownQuota::Allow => Ok(()),
            OnUnknownQuota::Block => Err(reason.to_string()),
        }
    }
}

fn now_epoch() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
