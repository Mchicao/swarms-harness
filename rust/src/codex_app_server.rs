//! Minimal Codex app-server session used for live steering.
//!
//! ponytail: keep this transport small and line-oriented. The provider owns
//! the turn; SWARMS only sends `turn/steer` between tool rounds.

use crate::adapter::{which, ChildGuard};
use crate::model::{Task, ThinkingLevel};
use crate::steering;
use serde_json::{json, Value};
use std::collections::VecDeque;
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread;
use std::time::Duration;

type Result<T> = std::result::Result<T, String>;

pub struct SessionResult {
    pub output: String,
    pub session_id: Option<String>,
}

pub fn enabled() -> bool {
    std::env::var("SWARMS_CODEX_APP_SERVER")
        .map(|value| matches!(value.trim(), "1" | "true" | "yes"))
        .unwrap_or(false)
}

pub fn run(
    task: &Task,
    prompt: &str,
    thinking: ThinkingLevel,
    session_id: Option<&str>,
    cwd: &Path,
    log_path: &Path,
    run_dir: &Path,
) -> Result<SessionResult> {
    let program = which("codex").unwrap_or_else(|| "codex".to_string());
    let mut child = ChildGuard::new(
        Command::new(program)
            .args(["app-server", "--stdio"])
            .current_dir(cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| format!("spawn codex app-server: {error}"))?,
    );
    let mut stdin = child
        .stdin
        .take()
        .ok_or("codex app-server stdin unavailable")?;
    let stdout = child
        .stdout
        .take()
        .ok_or("codex app-server stdout unavailable")?;
    let stderr = child
        .stderr
        .take()
        .ok_or("codex app-server stderr unavailable")?;

    let (tx, rx) = mpsc::channel::<String>();
    spawn_reader(stdout, tx);
    spawn_text_reader(stderr, log_path.to_path_buf());
    let mut log = OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path)
        .map_err(|error| format!("open {}: {error}", log_path.display()))?;
    let mut pending_events = VecDeque::new();

    let mut next_id = 1_u64;
    send(
        &mut stdin,
        next_id,
        "initialize",
        json!({
            "clientInfo": {"name": "swarms", "title": "SWARMS", "version": env!("CARGO_PKG_VERSION")}
        }),
    )?;
    wait_response(
        &rx,
        &mut log,
        &mut pending_events,
        next_id,
        Duration::from_secs(20),
    )?;
    send_notification(&mut stdin, "initialized", json!({}))?;
    next_id += 1;

    let mut thread_params = json!({
        "cwd": cwd.to_string_lossy(),
        "model": task.provider.model,
        "approvalPolicy": "never",
        "sandbox": if task.spec.allows_workspace_write() { "workspaceWrite" } else { "readOnly" },
        "serviceName": "swarms-runtime"
    });
    let thread_method = if let Some(id) = session_id {
        thread_params["threadId"] = json!(id);
        "thread/resume"
    } else {
        "thread/start"
    };
    send(&mut stdin, next_id, thread_method, thread_params)?;
    let thread_response = wait_response(
        &rx,
        &mut log,
        &mut pending_events,
        next_id,
        Duration::from_secs(20),
    )?;
    let thread_id = find_string(&thread_response, &["threadId", "id"]);

    next_id += 1;
    let mut turn_params = json!({
        "threadId": thread_id.clone().or_else(|| session_id.map(str::to_string)).ok_or("Codex app-server did not return a thread id")?,
        "input": [{"type": "text", "text": prompt}],
        "model": task.provider.model,
        "cwd": cwd.to_string_lossy(),
        "approvalPolicy": "never"
    });
    if let Some(effort) = thinking.as_codex_str() {
        turn_params["effort"] = json!(effort);
    }
    send(&mut stdin, next_id, "turn/start", turn_params)?;
    let turn_response = wait_response(
        &rx,
        &mut log,
        &mut pending_events,
        next_id,
        Duration::from_secs(20),
    )?;
    let mut turn_id = find_string(&turn_response, &["turnId", "id"])
        .ok_or("Codex app-server did not return a turn id")?;
    next_id += 1;

    let mut output = String::new();
    let mut steer_id = next_id;
    let mut queued_steers = Vec::new();
    let mut active_queued_steers = Vec::new();
    loop {
        for steer in steering::drain(run_dir, &task.id)? {
            if steer.mode == steering::SteeringMode::Enqueue {
                queued_steers.push(steer);
                continue;
            }
            let mode = steer.mode.as_str();
            let response = if mode == "cancel_and_restart" {
                send(
                    &mut stdin,
                    steer_id,
                    "turn/interrupt",
                    json!({
                        "threadId": thread_id.clone().unwrap_or_default(),
                        "turnId": turn_id
                    }),
                )?;
                wait_response(
                    &rx,
                    &mut log,
                    &mut pending_events,
                    steer_id,
                    Duration::from_secs(10),
                )?;
                steer_id += 1;
                send(
                    &mut stdin,
                    steer_id,
                    "turn/start",
                    json!({
                        "threadId": thread_id.clone().unwrap_or_default(),
                        "input": [{"type": "text", "text": format!("{prompt}\n\nUSER STEER PROMPT\n{}", steer.prompt)}],
                        "model": task.provider.model,
                        "cwd": cwd.to_string_lossy(),
                        "approvalPolicy": "never"
                    }),
                )?;
                let restarted = wait_response(
                    &rx,
                    &mut log,
                    &mut pending_events,
                    steer_id,
                    Duration::from_secs(20),
                )?;
                turn_id = find_string(&restarted, &["turnId", "id"])
                    .ok_or("Codex app-server did not return a restarted turn id")?;
                Ok(restarted)
            } else {
                send(
                    &mut stdin,
                    steer_id,
                    "turn/steer",
                    json!({
                        "threadId": thread_id.clone().unwrap_or_default(),
                        "expectedTurnId": turn_id,
                        "input": [{"type": "text", "text": steer.prompt}]
                    }),
                )?;
                wait_response(
                    &rx,
                    &mut log,
                    &mut pending_events,
                    steer_id,
                    Duration::from_secs(10),
                )
            };
            let applied = response.is_ok();
            steering::mark_applied(
                run_dir,
                &task.id,
                &steering::AppliedSteer {
                    message: steer,
                    status: if applied {
                        "accepted".to_string()
                    } else {
                        "failed".to_string()
                    },
                    error: response.err(),
                },
            )?;
            steer_id += 1;
        }

        let event = if let Some(value) = pending_events.pop_front() {
            Some(value)
        } else {
            match rx.recv_timeout(Duration::from_millis(100)) {
                Ok(message) => {
                    append_line(&mut log, &message)?;
                    serde_json::from_str::<Value>(&message).ok()
                }
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => break,
            }
        };
        if let Some(value) = event {
            collect_text(&value, &mut output);
            if completed_turn_id(&value).as_deref() == Some(turn_id.as_str()) {
                if let Some(error) = completion_error(&value) {
                    return Err(error);
                }
                for steer in active_queued_steers.drain(..) {
                    steering::mark_applied(
                        run_dir,
                        &task.id,
                        &steering::AppliedSteer {
                            message: steer,
                            status: "applied".to_string(),
                            error: None,
                        },
                    )?;
                }
                if queued_steers.is_empty() {
                    let _ = child.kill();
                    return Ok(SessionResult {
                        output,
                        session_id: thread_id,
                    });
                }
                let queued_prompt = queued_steers
                    .iter()
                    .map(|steer| steer.prompt.as_str())
                    .collect::<Vec<_>>()
                    .join("\n\n");
                send(
                    &mut stdin,
                    steer_id,
                    "turn/start",
                    json!({
                        "threadId": thread_id.clone().unwrap_or_default(),
                        "input": [{"type": "text", "text": format!("USER STEER PROMPT (enqueue)\n{queued_prompt}\n\nThe previous turn completed. Apply this queued direction before finalizing the task.")}],
                    }),
                )?;
                let response = wait_response(
                    &rx,
                    &mut log,
                    &mut pending_events,
                    steer_id,
                    Duration::from_secs(20),
                )?;
                turn_id = find_string(&response, &["turnId", "id"])
                    .ok_or("Codex app-server did not return a queued turn id")?;
                steer_id += 1;
                active_queued_steers.append(&mut queued_steers);
            }
        }
    }
    let status = child.wait().map_err(|error| error.to_string())?;
    if status.success() {
        Ok(SessionResult {
            output,
            session_id: thread_id,
        })
    } else {
        Err(format!("codex app-server exited {:?}", status.code()))
    }
}

fn spawn_reader<R: std::io::Read + Send + 'static>(reader: R, tx: Sender<String>) {
    thread::spawn(move || {
        for line in BufReader::new(reader).lines().map_while(|line| line.ok()) {
            let _ = tx.send(line);
        }
    });
}

fn spawn_text_reader<R: std::io::Read + Send + 'static>(reader: R, path: std::path::PathBuf) {
    thread::spawn(move || {
        let Ok(mut log) = OpenOptions::new().create(true).append(true).open(path) else {
            return;
        };
        for line in BufReader::new(reader).lines().map_while(|line| line.ok()) {
            let _ = writeln!(log, "{}", json!({"type": "stderr", "text": line}));
        }
    });
}

fn send(stdin: &mut impl Write, id: u64, method: &str, params: Value) -> Result<()> {
    let message = json!({"id": id, "method": method, "params": params});
    serde_json::to_writer(&mut *stdin, &message).map_err(|error| error.to_string())?;
    stdin.write_all(b"\n").map_err(|error| error.to_string())?;
    stdin.flush().map_err(|error| error.to_string())
}

fn send_notification(stdin: &mut impl Write, method: &str, params: Value) -> Result<()> {
    let message = json!({"method": method, "params": params});
    serde_json::to_writer(&mut *stdin, &message).map_err(|error| error.to_string())?;
    stdin.write_all(b"\n").map_err(|error| error.to_string())?;
    stdin.flush().map_err(|error| error.to_string())
}

fn wait_response(
    rx: &Receiver<String>,
    log: &mut fs::File,
    pending_events: &mut VecDeque<Value>,
    id: u64,
    timeout: Duration,
) -> Result<Value> {
    let deadline = std::time::Instant::now() + timeout;
    loop {
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        if remaining.is_zero() {
            return Err(format!("timeout waiting for app-server response {id}"));
        }
        let message = rx
            .recv_timeout(remaining)
            .map_err(|error| error.to_string())?;
        append_line(log, &message)?;
        let value: Value = serde_json::from_str(&message).map_err(|error| error.to_string())?;
        if value.get("id").and_then(Value::as_u64) == Some(id) {
            if let Some(error) = value.get("error") {
                return Err(format!("app-server request {id} failed: {error}"));
            }
            return Ok(value.get("result").cloned().unwrap_or(Value::Null));
        }
        pending_events.push_back(value);
    }
}

fn append_line(log: &mut fs::File, line: &str) -> Result<()> {
    writeln!(log, "{line}").map_err(|error| error.to_string())
}

fn find_string(value: &Value, keys: &[&str]) -> Option<String> {
    for key in keys {
        if let Some(found) = value.get(*key).and_then(Value::as_str) {
            return Some(found.to_string());
        }
    }
    if let Some(object) = value.as_object() {
        for child in object.values() {
            if let Some(found) = find_string(child, keys) {
                return Some(found);
            }
        }
    }
    None
}

fn collect_text(value: &Value, output: &mut String) {
    if value.get("method").and_then(Value::as_str) == Some("item/agentMessage/delta") {
        if let Some(text) = value.pointer("/params/delta").and_then(Value::as_str) {
            output.push_str(text);
        }
    } else if let Some(text) = value.get("delta").and_then(Value::as_str) {
        output.push_str(text);
    } else if let Some(text) = value.get("text").and_then(Value::as_str) {
        output.push_str(text);
    }
}

fn completed_turn_id(value: &Value) -> Option<String> {
    if value.get("method").and_then(Value::as_str) != Some("turn/completed") {
        return None;
    }
    value
        .pointer("/params/turn/id")
        .and_then(Value::as_str)
        .map(str::to_string)
}

fn completion_error(value: &Value) -> Option<String> {
    match value
        .pointer("/params/turn/status")
        .and_then(Value::as_str)?
    {
        "failed" => Some("Codex app-server turn failed".to_string()),
        "interrupted" => Some("Codex app-server turn was interrupted".to_string()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_current_app_server_ids_and_delta_shape() {
        let response = json!({"thread": {"id": "thr_123"}});
        assert_eq!(
            find_string(&response, &["threadId", "id"]).as_deref(),
            Some("thr_123")
        );

        let mut output = String::new();
        collect_text(
            &json!({
                "method": "item/agentMessage/delta",
                "params": {"threadId": "thr_123", "turnId": "turn_1", "delta": "hello"}
            }),
            &mut output,
        );
        assert_eq!(output, "hello");

        assert_eq!(
            completed_turn_id(&json!({
                "method": "turn/completed",
                "params": {"turn": {"id": "turn_1", "status": "completed"}}
            }))
            .as_deref(),
            Some("turn_1")
        );
        assert_eq!(
            completion_error(&json!({
                "method": "turn/completed",
                "params": {"turn": {"id": "turn_1", "status": "failed"}}
            }))
            .as_deref(),
            Some("Codex app-server turn failed")
        );
    }
}
