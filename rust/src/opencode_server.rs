//! OpenCode live session transport, modelled after T3 Code's provider adapter.
//!
//! The normal `opencode run` adapter remains the default. This opt-in transport
//! owns `opencode serve`, keeps one session alive for the active turn, streams
//! `/event`, and sends steering through `prompt_async`.

use crate::adapter::{which, ChildGuard};
use crate::model::{Task, ThinkingLevel};
use crate::steering;
use serde_json::{json, Value};
use std::fs::OpenOptions;
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread;
use std::time::{Duration, Instant};

type Result<T> = std::result::Result<T, String>;

pub struct SessionResult {
    pub output: String,
    pub session_id: Option<String>,
}

pub fn enabled() -> bool {
    std::env::var("SWARMS_OPENCODE_SERVER")
        .map(|value| matches!(value.trim(), "1" | "true" | "yes"))
        .unwrap_or(false)
}

pub fn run(
    task: &Task,
    prompt: &str,
    _thinking: ThinkingLevel,
    session_id: Option<&str>,
    cwd: &Path,
    log_path: &Path,
    run_dir: &Path,
) -> Result<SessionResult> {
    let program = which("opencode").unwrap_or_else(|| "opencode".to_string());
    let mut command = Command::new(program);
    command.args(["serve", "--hostname", "127.0.0.1", "--port", "0"]);
    if !task.spec.allows_workspace_write() {
        command.arg("--pure");
    }
    let mut child = ChildGuard::new(
        command
            .current_dir(cwd)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| format!("spawn opencode serve: {error}"))?,
    );
    let stdout = child.stdout.take().ok_or("opencode stdout unavailable")?;
    let stderr = child.stderr.take().ok_or("opencode stderr unavailable")?;
    let (startup_tx, startup_rx) = mpsc::channel::<String>();
    spawn_reader(stdout, startup_tx.clone());
    spawn_reader(stderr, startup_tx);
    let base_url = wait_for_server(&startup_rx, Duration::from_secs(20))?;

    let mut log = OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path)
        .map_err(|error| format!("open {}: {error}", log_path.display()))?;
    let session_id = match session_id {
        Some(id) => id.to_string(),
        None => create_session(&base_url)?,
    };
    let (event_tx, event_rx) = mpsc::channel::<Value>();
    let event_response = http_get(&format!("{base_url}/event"))?;
    thread::spawn(move || stream_events(event_response, event_tx));
    prompt_async(&base_url, &session_id, prompt, &task.provider.model)?;

    let mut output = String::new();
    let mut finished = false;
    let mut failed = false;
    let mut queued_steers = Vec::new();
    let mut active_queued_steers = Vec::new();
    while !finished {
        for steer in steering::drain(run_dir, &task.id)? {
            if steer.mode == steering::SteeringMode::Enqueue {
                queued_steers.push(steer);
                continue;
            }
            let result = if steer.mode.as_str() == "cancel_and_restart" {
                abort(&base_url, &session_id)?;
                prompt_async(
                    &base_url,
                    &session_id,
                    &format!("{prompt}\n\nUSER STEER PROMPT\n{}", steer.prompt),
                    &task.provider.model,
                )
            } else {
                prompt_async(&base_url, &session_id, &steer.prompt, &task.provider.model)
            };
            let error = result.err();
            steering::mark_applied(
                run_dir,
                &task.id,
                &steering::AppliedSteer {
                    message: steer,
                    status: if error.is_none() {
                        "accepted"
                    } else {
                        "failed"
                    }
                    .to_string(),
                    error,
                },
            )?;
        }
        match event_rx.recv_timeout(Duration::from_millis(100)) {
            Ok(event) => {
                append_json(&mut log, &event)?;
                collect_text(&event, &mut output);
                if is_error_event(&event) {
                    failed = true;
                    finished = true;
                } else if is_idle_event(&event, &session_id) {
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
                        finished = true;
                    } else {
                        let queued_prompt = queued_steers
                            .iter()
                            .map(|steer| steer.prompt.as_str())
                            .collect::<Vec<_>>()
                            .join("\n\n");
                        prompt_async(
                            &base_url,
                            &session_id,
                            &format!(
                                "USER STEER PROMPT (enqueue)\n{queued_prompt}\n\nThe previous turn completed. Apply this queued direction before finalizing the task."
                            ),
                            &task.provider.model,
                        )?;
                        active_queued_steers.append(&mut queued_steers);
                    }
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }
    let _ = abort(&base_url, &session_id);
    let _ = child.kill();
    let _ = child.wait();
    if failed {
        return Err("OpenCode session emitted an error".to_string());
    }
    if output.is_empty() {
        output = format!("session_id: {session_id}");
    }
    Ok(SessionResult {
        output: format!("{}\n{}", json!({"sessionID": session_id}), output),
        session_id: Some(session_id),
    })
}

fn wait_for_server(rx: &Receiver<String>, timeout: Duration) -> Result<String> {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        let left = deadline.saturating_duration_since(Instant::now());
        if let Ok(line) = rx.recv_timeout(left.min(Duration::from_millis(200))) {
            if let Some(start) = line.find("http://") {
                let url = line[start..].split_whitespace().next().unwrap_or_default();
                if !url.is_empty() {
                    return Ok(url.trim_end_matches('/').to_string());
                }
            }
        }
    }
    Err("timeout waiting for opencode serve".to_string())
}

fn create_session(base: &str) -> Result<String> {
    let value = http_json("POST", &format!("{base}/session"), json!({}))?;
    find_string(&value, &["id"]).ok_or_else(|| "OpenCode session.create returned no id".to_string())
}

fn prompt_async(base: &str, session: &str, prompt: &str, model: &str) -> Result<()> {
    let model = model_selection(model);
    http_json(
        "POST",
        &format!("{base}/session/{session}/prompt_async"),
        json!({
            "model": model,
            "parts": [{"type": "text", "text": prompt}]
        }),
    )?;
    Ok(())
}

fn model_selection(model: &str) -> Value {
    if let Some((provider_id, model_id)) = model.split_once('/') {
        json!({"providerID": provider_id, "modelID": model_id})
    } else {
        json!(model)
    }
}

fn abort(base: &str, session: &str) -> Result<()> {
    http_json(
        "POST",
        &format!("{base}/session/{session}/abort"),
        json!({}),
    )?;
    Ok(())
}

fn http_json(method: &str, url: &str, body: Value) -> Result<Value> {
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(10))
        .timeout_read(Duration::from_secs(20))
        .timeout_write(Duration::from_secs(20))
        .build();
    let request = match method {
        "POST" => agent.post(url),
        _ => return Err(format!("unsupported HTTP method {method}")),
    };
    let response = request
        .send_json(body)
        .map_err(|error| format!("OpenCode HTTP {url}: {error}"))?;
    if response.status() == 204 {
        return Ok(Value::Null);
    }
    response
        .into_json::<Value>()
        .map_err(|error| format!("decode OpenCode HTTP {url}: {error}"))
}

fn http_get(url: &str) -> Result<ureq::Response> {
    ureq::get(url)
        .call()
        .map_err(|error| format!("OpenCode event stream {url}: {error}"))
}

fn stream_events(response: ureq::Response, tx: Sender<Value>) {
    let reader = BufReader::new(response.into_reader());
    for line in reader.lines().map_while(|line| line.ok()) {
        if let Some(data) = line.strip_prefix("data:") {
            if let Ok(value) = serde_json::from_str::<Value>(data.trim()) {
                let _ = tx.send(value);
            }
        }
    }
}

fn spawn_reader<R: std::io::Read + Send + 'static>(reader: R, tx: Sender<String>) {
    thread::spawn(move || {
        for line in BufReader::new(reader).lines().map_while(|line| line.ok()) {
            let _ = tx.send(line);
        }
    });
}

fn append_json(log: &mut std::fs::File, value: &Value) -> Result<()> {
    serde_json::to_writer(&mut *log, value).map_err(|error| error.to_string())?;
    log.write_all(b"\n").map_err(|error| error.to_string())
}

fn find_string(value: &Value, keys: &[&str]) -> Option<String> {
    if let Some(object) = value.as_object() {
        for key in keys {
            if let Some(found) = object.get(*key).and_then(Value::as_str) {
                return Some(found.to_string());
            }
        }
        for child in object.values() {
            if let Some(found) = find_string(child, keys) {
                return Some(found);
            }
        }
    }
    None
}

fn event_type(value: &Value) -> Option<&str> {
    value
        .get("type")
        .and_then(Value::as_str)
        .or_else(|| value.pointer("/event/type").and_then(Value::as_str))
}

fn is_idle_event(value: &Value, session: &str) -> bool {
    matches!(event_type(value), Some("session.idle"))
        && find_string(value, &["sessionID", "sessionId"]).as_deref() == Some(session)
}

fn is_error_event(value: &Value) -> bool {
    matches!(event_type(value), Some("session.error" | "error"))
}

fn collect_text(value: &Value, output: &mut String) {
    if let Some(object) = value.as_object() {
        if object.get("type").and_then(Value::as_str) == Some("text") {
            if let Some(text) = object.get("text").and_then(Value::as_str) {
                output.push_str(text);
            }
        }
        for child in object.values() {
            collect_text(child, output);
        }
    } else if let Some(items) = value.as_array() {
        for child in items {
            collect_text(child, output);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_current_model_and_session_event_shapes() {
        assert_eq!(
            model_selection("opencode/big-pickle"),
            json!({"providerID": "opencode", "modelID": "big-pickle"})
        );
        assert!(is_idle_event(
            &json!({"type": "session.idle", "properties": {"sessionID": "s1"}}),
            "s1",
        ));
        assert!(is_error_event(&json!({"event": {"type": "session.error"}})));
    }
}
