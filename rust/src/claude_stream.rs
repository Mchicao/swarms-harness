//! Claude Code bidirectional stream-json transport.
//!
//! This is the Rust equivalent of T3 Code's Claude prompt queue: one Claude
//! process remains alive, user messages are written as NDJSON, and stdout is
//! consumed as a stream of runtime events.

use crate::adapter::{which, ChildGuard};
use crate::model::{Task, ThinkingLevel};
use crate::steering;
use serde_json::{json, Value};
use std::fs::OpenOptions;
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread;
use std::time::Duration;

type Result<T> = std::result::Result<T, String>;

pub struct SessionResult {
    pub output: String,
    pub session_id: Option<String>,
}

pub fn enabled() -> bool {
    std::env::var("SWARMS_CLAUDE_STREAMING")
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
    let mut resolved_session = session_id.map(str::to_string);
    let (mut child, mut stdin, mut rx) = spawn_claude(task, cwd, resolved_session.as_deref())?;
    let mut log = OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path)
        .map_err(|error| format!("open {}: {error}", log_path.display()))?;

    send_prompt(&mut stdin, prompt)?;
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
                let _ = child.kill();
                let _ = child.wait();
                let restarted_prompt = format!("{prompt}\n\nUSER STEER PROMPT\n{}", steer.prompt);
                match spawn_claude(task, cwd, resolved_session.as_deref()) {
                    Ok((next_child, next_stdin, next_rx)) => {
                        child = next_child;
                        stdin = next_stdin;
                        rx = next_rx;
                        send_prompt(&mut stdin, &restarted_prompt)
                    }
                    Err(error) => Err(error),
                }
            } else {
                send_prompt(&mut stdin, &steer.prompt)
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
        match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(line) => {
                writeln!(log, "{line}").map_err(|error| error.to_string())?;
                if let Ok(value) = serde_json::from_str::<Value>(&line) {
                    if let Some(id) = value.get("session_id").and_then(Value::as_str) {
                        resolved_session = Some(id.to_string());
                    }
                    collect_text(&value, &mut output);
                    if value.get("type").and_then(Value::as_str) == Some("result") {
                        failed |= value
                            .get("is_error")
                            .and_then(Value::as_bool)
                            .unwrap_or(false);
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
                            send_prompt(
                                &mut stdin,
                                &format!(
                                    "USER STEER PROMPT (enqueue)\n{queued_prompt}\n\nThe previous turn completed. Apply this queued direction before finalizing the task."
                                ),
                            )?;
                            active_queued_steers.append(&mut queued_steers);
                        }
                    }
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }
    let _ = child.kill();
    let status = child.wait().map_err(|error| error.to_string())?;
    if !status.success() && output.is_empty() {
        return Err(format!("claude stream exited {:?}", status.code()));
    }
    if failed {
        return Err("Claude stream reported an error result".to_string());
    }
    Ok(SessionResult {
        output,
        session_id: resolved_session,
    })
}

fn spawn_claude(
    task: &Task,
    cwd: &Path,
    session_id: Option<&str>,
) -> Result<(ChildGuard, ChildStdin, mpsc::Receiver<String>)> {
    let program = which("claude").unwrap_or_else(|| "claude".to_string());
    let mut command = Command::new(program);
    command
        .args([
            "-p",
            "--input-format",
            "stream-json",
            "--output-format",
            "stream-json",
            "--verbose",
            "--replay-user-messages",
        ])
        .current_dir(cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(id) = session_id {
        command.args(["--resume", id]);
    }
    if !task.provider.model.is_empty() {
        command.args(["--model", &task.provider.model]);
    }
    if task.spec.tools_policy == "full" {
        command.arg("--dangerously-skip-permissions");
    }
    let mut child = ChildGuard::new(
        command
            .spawn()
            .map_err(|error| format!("spawn claude stream-json: {error}"))?,
    );
    let stdin = child.stdin.take().ok_or("claude stdin unavailable")?;
    let stdout = child.stdout.take().ok_or("claude stdout unavailable")?;
    let stderr = child.stderr.take().ok_or("claude stderr unavailable")?;
    let (tx, rx) = mpsc::channel::<String>();
    spawn_reader(stdout, tx.clone());
    spawn_reader(stderr, tx);
    Ok((child, stdin, rx))
}

fn send_prompt(stdin: &mut impl Write, prompt: &str) -> Result<()> {
    serde_json::to_writer(
        &mut *stdin,
        &json!({"type":"user","message":{"role":"user","content":[{"type":"text","text":prompt}]},"parent_tool_use_id":null}),
    )
    .map_err(|error| error.to_string())?;
    stdin.write_all(b"\n").map_err(|error| error.to_string())?;
    stdin.flush().map_err(|error| error.to_string())
}

fn spawn_reader<R: std::io::Read + Send + 'static>(reader: R, tx: mpsc::Sender<String>) {
    thread::spawn(move || {
        for line in BufReader::new(reader).lines().map_while(|line| line.ok()) {
            let _ = tx.send(line);
        }
    });
}

fn collect_text(value: &Value, output: &mut String) {
    if let Some(result) = value.get("result").and_then(Value::as_str) {
        if output.is_empty() {
            output.push_str(result);
        }
        return;
    }
    if let Some(text) = value.pointer("/event/delta/text").and_then(Value::as_str) {
        output.push_str(text);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collects_stream_delta_and_final_result() {
        let mut output = String::new();
        collect_text(&json!({"event": {"delta": {"text": "hello"}}}), &mut output);
        collect_text(&json!({"type": "result", "result": " done"}), &mut output);
        assert_eq!(output, "hello");

        let mut result_only = String::new();
        collect_text(
            &json!({"type": "result", "result": "complete"}),
            &mut result_only,
        );
        assert_eq!(result_only, "complete");
    }
}
