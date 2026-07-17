//! Persisted user steering mailbox for active SWARMS tasks.

use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

type Result<T> = std::result::Result<T, String>;

pub const MAX_STEER_PROMPT_CHARS: usize = 4_000;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct SteerMessage {
    pub id: String,
    pub created_at_epoch_ms: u128,
    pub prompt: String,
    pub source: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct AppliedSteer {
    pub message: SteerMessage,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

pub fn enqueue(run_dir: &Path, task_id: &str, prompt: &str, source: &str) -> Result<SteerMessage> {
    validate_component(task_id, "task_id")?;
    let prompt = prompt.trim();
    let count = prompt.chars().count();
    if count == 0 || count > MAX_STEER_PROMPT_CHARS {
        return Err(format!(
            "steer prompt must contain 1..={MAX_STEER_PROMPT_CHARS} characters"
        ));
    }
    let created_at_epoch_ms = now_epoch_ms();
    let message = SteerMessage {
        id: format!("{created_at_epoch_ms}-{}", std::process::id()),
        created_at_epoch_ms,
        prompt: prompt.to_string(),
        source: source.to_string(),
    };
    append_json_line(&inbox_path(run_dir, task_id), &message)?;
    Ok(message)
}

/// Atomically claim every currently queued message for one task.
pub fn drain(run_dir: &Path, task_id: &str) -> Result<Vec<SteerMessage>> {
    validate_component(task_id, "task_id")?;
    let inbox = inbox_path(run_dir, task_id);
    if !inbox.exists() {
        return Ok(Vec::new());
    }
    let claimed = inbox.with_extension(format!("claimed-{}", now_epoch_ms()));
    match fs::rename(&inbox, &claimed) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => {
            return Err(format!(
                "claim steering inbox '{}': {error}",
                inbox.display()
            ))
        }
    }
    let text = fs::read_to_string(&claimed)
        .map_err(|error| format!("read steering inbox '{}': {error}", claimed.display()))?;
    let messages = text
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str(line).map_err(|error| error.to_string()))
        .collect::<Result<Vec<_>>>()?;
    fs::remove_file(&claimed).map_err(|error| {
        format!(
            "remove claimed steering inbox '{}': {error}",
            claimed.display()
        )
    })?;
    Ok(messages)
}

pub fn mark_applied(run_dir: &Path, task_id: &str, applied: &AppliedSteer) -> Result<()> {
    validate_component(task_id, "task_id")?;
    append_json_line(&history_path(run_dir, task_id), applied)
}

pub fn history(run_dir: &Path, task_id: &str) -> Vec<AppliedSteer> {
    let path = history_path(run_dir, task_id);
    fs::read_to_string(path)
        .ok()
        .map(|text| {
            text.lines()
                .filter_map(|line| serde_json::from_str(line).ok())
                .collect()
        })
        .unwrap_or_default()
}

fn task_dir(run_dir: &Path, task_id: &str) -> PathBuf {
    run_dir.join("steering").join(task_id)
}

fn inbox_path(run_dir: &Path, task_id: &str) -> PathBuf {
    task_dir(run_dir, task_id).join("inbox.jsonl")
}

fn history_path(run_dir: &Path, task_id: &str) -> PathBuf {
    task_dir(run_dir, task_id).join("history.jsonl")
}

fn append_json_line<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| format!("{}: {error}", parent.display()))?;
    }
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|error| format!("{}: {error}", path.display()))?;
    let mut line = serde_json::to_vec(value).map_err(|error| error.to_string())?;
    line.push(b'\n');
    file.write_all(&line)
        .map_err(|error| format!("{}: {error}", path.display()))
}

fn validate_component(value: &str, label: &str) -> Result<()> {
    let valid = !value.is_empty()
        && value.len() <= 160
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'));
    if valid {
        Ok(())
    } else {
        Err(format!("unsafe {label}: {value:?}"))
    }
}

fn now_epoch_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEMP_DIR_SEQUENCE: AtomicU64 = AtomicU64::new(0);

    fn temp_dir() -> PathBuf {
        std::env::temp_dir().join(format!(
            "swarms-steering-{}-{}-{}",
            std::process::id(),
            now_epoch_ms(),
            TEMP_DIR_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ))
    }

    #[test]
    fn steering_mailbox_is_persisted_drained_and_audited() {
        let dir = temp_dir();
        let message = enqueue(&dir, "task-1", "Prefer the smaller API.", "test").unwrap();
        let drained = drain(&dir, "task-1").unwrap();
        assert_eq!(drained, vec![message.clone()]);
        assert!(drain(&dir, "task-1").unwrap().is_empty());
        mark_applied(
            &dir,
            "task-1",
            &AppliedSteer {
                message: message.clone(),
                status: "applied".to_string(),
                error: None,
            },
        )
        .unwrap();
        assert_eq!(history(&dir, "task-1")[0].message, message);
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn steering_rejects_unsafe_ids_and_oversized_prompts() {
        let dir = temp_dir();
        assert!(enqueue(&dir, "../escape", "hello", "test").is_err());
        assert!(enqueue(&dir, "task", "", "test").is_err());
        assert!(enqueue(
            &dir,
            "task",
            &"x".repeat(MAX_STEER_PROMPT_CHARS + 1),
            "test"
        )
        .is_err());
        fs::remove_dir_all(dir).ok();
    }
}
