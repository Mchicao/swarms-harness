//! Small blocking ACP v1 client used by the Rust scheduler.
//!
//! ACP agents speak JSON-RPC messages over newline-delimited stdio. The
//! scheduler owns this process and polls the incoming stream so steering can
//! cancel a live turn without handing control to a terminal UI.

use serde_json::{json, Value};
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::thread;
use std::time::Duration;

type Result<T> = std::result::Result<T, String>;

#[derive(Debug)]
pub enum Event {
    Update(Value),
    Notification {
        method: String,
        params: Value,
    },
    Response {
        id: u64,
        result: Option<Value>,
        error: Option<String>,
    },
}

pub struct Client {
    child: Child,
    stdin: ChildStdin,
    incoming: Receiver<String>,
    next_id: u64,
    session_id: Option<String>,
    cancel_grace: Duration,
}

impl Client {
    /// Start an ACP child with stdin/stdout pipes and a separate stderr log.
    pub fn launch(
        program: &str,
        args: &[String],
        cwd: &Path,
        log_path: &Path,
        startup_timeout: Duration,
        cancel_grace: Duration,
    ) -> Result<Self> {
        if program.trim().is_empty() {
            return Err("ACP command is empty".to_string());
        }
        if let Some(parent) = log_path.parent() {
            fs::create_dir_all(parent).map_err(|error| format!("ACP log directory: {error}"))?;
        }
        let mut command = Command::new(program);
        command
            .args(args)
            .current_dir(cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x0800_0000);
        }
        let mut child = command
            .spawn()
            .map_err(|error| format!("spawn ACP '{}': {error}", program))?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| "ACP child did not expose stdin".to_string())?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| "ACP child did not expose stdout".to_string())?;
        let stderr = child.stderr.take();
        let (sender, incoming) = mpsc::channel();
        thread::spawn(move || {
            for line in BufReader::new(stdout)
                .lines()
                .map_while(std::result::Result::ok)
            {
                if sender.send(line).is_err() {
                    break;
                }
            }
        });
        if let Some(stderr) = stderr {
            let stderr_path = log_path.to_path_buf();
            thread::spawn(move || {
                let Ok(mut file) = OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(stderr_path)
                else {
                    return;
                };
                for line in BufReader::new(stderr)
                    .lines()
                    .map_while(std::result::Result::ok)
                {
                    let encoded = serde_json::to_string(&line)
                        .unwrap_or_else(|_| "\"unreadable\"".to_string());
                    let _ = writeln!(file, "{{\"type\":\"stderr\",\"text\":{encoded}}}");
                }
            });
        }
        let mut client = Self {
            child,
            stdin,
            incoming,
            next_id: 1,
            session_id: None,
            cancel_grace,
        };
        client.initialize(startup_timeout)?;
        Ok(client)
    }

    fn initialize(&mut self, timeout: Duration) -> Result<()> {
        let id = self.request(
            "initialize",
            json!({
                "protocolVersion": 1,
                "clientInfo": {"name": "swarms-runtime", "version": env!("CARGO_PKG_VERSION")},
                "clientCapabilities": {}
            }),
        )?;
        let response = self.wait_for_response(id, timeout)?;
        let negotiated = response
            .get("protocolVersion")
            .and_then(Value::as_u64)
            .unwrap_or(1);
        if negotiated != 1 {
            return Err(format!(
                "ACP agent negotiated unsupported protocol version {negotiated}"
            ));
        }
        Ok(())
    }

    pub fn open_session(
        &mut self,
        cwd: &Path,
        existing_session: Option<&str>,
        timeout: Duration,
    ) -> Result<String> {
        let (method, params) = if let Some(session_id) = existing_session {
            (
                "session/load",
                json!({"sessionId": session_id, "cwd": cwd, "mcpServers": []}),
            )
        } else {
            ("session/new", json!({"cwd": cwd, "mcpServers": []}))
        };
        let id = self.request(method, params)?;
        let response = self.wait_for_response(id, timeout)?;
        let session_id = response
            .get("sessionId")
            .and_then(Value::as_str)
            .ok_or_else(|| format!("ACP {method} response did not include sessionId"))?
            .to_string();
        self.session_id = Some(session_id.clone());
        Ok(session_id)
    }

    pub fn start_prompt(&mut self, prompt: &str) -> Result<u64> {
        let session_id = self
            .session_id
            .as_deref()
            .ok_or_else(|| "ACP session is not open".to_string())?;
        self.request(
            "session/prompt",
            json!({
                "sessionId": session_id,
                "prompt": [{"type": "text", "text": prompt.replace('\0', "")}]
            }),
        )
    }

    pub fn cancel(&mut self) -> Result<()> {
        let session_id = self
            .session_id
            .as_deref()
            .ok_or_else(|| "ACP session is not open".to_string())?;
        self.notification("session/cancel", json!({"sessionId": session_id}))
    }

    pub fn next_event(&self, timeout: Duration) -> Result<Option<Event>> {
        let line = match self.incoming.recv_timeout(timeout) {
            Ok(line) => line,
            Err(RecvTimeoutError::Timeout) => return Ok(None),
            Err(RecvTimeoutError::Disconnected) => return Err("ACP stdout closed".to_string()),
        };
        parse_event(&line)
            .map(Some)
            .map_err(|error| format!("invalid ACP message: {error}; line={line:?}"))
    }

    pub fn cancel_grace(&self) -> Duration {
        self.cancel_grace
    }

    fn request(&mut self, method: &str, params: Value) -> Result<u64> {
        let id = self.next_id;
        self.next_id = self.next_id.saturating_add(1);
        self.write_message(
            json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}),
        )?;
        Ok(id)
    }

    fn notification(&mut self, method: &str, params: Value) -> Result<()> {
        self.write_message(json!({"jsonrpc": "2.0", "method": method, "params": params}))
    }

    fn write_message(&mut self, value: Value) -> Result<()> {
        let line = serde_json::to_string(&value).map_err(|error| error.to_string())?;
        self.stdin
            .write_all(line.as_bytes())
            .and_then(|_| self.stdin.write_all(b"\n"))
            .and_then(|_| self.stdin.flush())
            .map_err(|error| format!("write ACP message: {error}"))
    }

    fn wait_for_response(&self, id: u64, timeout: Duration) -> Result<Value> {
        let deadline = std::time::Instant::now() + timeout;
        loop {
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            if remaining.is_zero() {
                return Err(format!("ACP request {id} timed out"));
            }
            let Some(event) = self.next_event(remaining)? else {
                return Err(format!("ACP request {id} timed out"));
            };
            if let Event::Response {
                id: response_id,
                result,
                error,
            } = event
            {
                if response_id != id {
                    continue;
                }
                if let Some(error) = error {
                    return Err(format!("ACP request {id} failed: {error}"));
                }
                return result.ok_or_else(|| format!("ACP request {id} had no result"));
            }
        }
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        if let Some(session_id) = self.session_id.clone() {
            let _ = self.notification("session/close", json!({"sessionId": session_id}));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn parse_event(line: &str) -> Result<Event> {
    let value: Value = serde_json::from_str(line).map_err(|error| error.to_string())?;
    if let Some(method) = value.get("method").and_then(Value::as_str) {
        let params = value.get("params").cloned().unwrap_or(Value::Null);
        if method == "session/update" {
            return Ok(Event::Update(params));
        }
        return Ok(Event::Notification {
            method: method.to_string(),
            params,
        });
    }
    let id = value
        .get("id")
        .and_then(Value::as_u64)
        .ok_or_else(|| "ACP response has no numeric id".to_string())?;
    let error = value.get("error").map(|error| {
        error
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("unknown ACP error")
            .to_string()
    });
    Ok(Event::Response {
        id,
        result: value.get("result").cloned(),
        error,
    })
}

/// Extract human-readable text from a v1 session/update payload.
pub fn update_text(params: &Value) -> Option<&str> {
    let update = params.get("update").unwrap_or(params);
    if update
        .get("sessionUpdate")
        .and_then(Value::as_str)
        .is_some_and(|kind| kind == "agent_message_chunk" || kind == "agent_thought_chunk")
    {
        return update
            .pointer("/content/text")
            .and_then(Value::as_str)
            .or_else(|| update.get("text").and_then(Value::as_str));
    }
    None
}

pub fn stop_reason(response: &Value) -> Option<&str> {
    response.get("stopReason").and_then(Value::as_str)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_update_and_extracts_text() {
        let event = parse_event(
            r#"{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"s1","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"hello"}}}}"#,
        )
        .unwrap();
        let Event::Update(params) = event else {
            panic!("expected update")
        };
        assert_eq!(update_text(&params), Some("hello"));
    }

    #[test]
    fn parses_response_error_without_panicking() {
        let event = parse_event(
            r#"{"jsonrpc":"2.0","id":4,"error":{"code":-32600,"message":"bad request"}}"#,
        )
        .unwrap();
        let Event::Response { id, error, result } = event else {
            panic!("expected response")
        };
        assert_eq!(id, 4);
        assert_eq!(error.as_deref(), Some("bad request"));
        assert!(result.is_none());
    }

    #[cfg(windows)]
    #[test]
    fn powershell_fake_peer_completes_initialize_session_and_prompt() {
        // A cold PowerShell process on GitHub's Windows runner can take more
        // than five seconds to reach the ACP read loop under parallel load.
        const CI_PROCESS_TIMEOUT: Duration = Duration::from_secs(30);
        let root = std::env::temp_dir().join(format!(
            "swarms-acp-peer-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let log = root.join("worker.log");
        let script = r#"
function Send-Message($value) {
    $value | ConvertTo-Json -Compress -Depth 20
}
while (($line = [Console]::ReadLine()) -ne $null) {
    $message = $line | ConvertFrom-Json
    switch ($message.method) {
        "initialize" {
            Send-Message @{jsonrpc="2.0"; id=$message.id; result=@{protocolVersion=1}}
        }
        "session/new" {
            Send-Message @{jsonrpc="2.0"; id=$message.id; result=@{sessionId="fake-session"}}
        }
        "session/prompt" {
            Send-Message @{jsonrpc="2.0"; method="session/update"; params=@{sessionId="fake-session"; update=@{sessionUpdate="agent_message_chunk"; content=@{type="text"; text="hello"}}}}
            Send-Message @{jsonrpc="2.0"; id=$message.id; result=@{stopReason="end_turn"}}
        }
        "session/close" {
            break
        }
    }
}
"#;
        let args = vec![
            "-NoLogo".to_string(),
            "-NoProfile".to_string(),
            "-Command".to_string(),
            script.to_string(),
        ];
        let mut client = Client::launch(
            "powershell",
            &args,
            &root,
            &log,
            CI_PROCESS_TIMEOUT,
            Duration::from_secs(1),
        )
        .unwrap();
        assert_eq!(
            client
                .open_session(&root, None, CI_PROCESS_TIMEOUT)
                .unwrap(),
            "fake-session"
        );
        let prompt_id = client.start_prompt("test").unwrap();
        let update = client.next_event(CI_PROCESS_TIMEOUT).unwrap().unwrap();
        let Event::Update(params) = update else {
            panic!("expected fake ACP update");
        };
        assert_eq!(update_text(&params), Some("hello"));
        let response = client.next_event(CI_PROCESS_TIMEOUT).unwrap().unwrap();
        let Event::Response { id, result, error } = response else {
            panic!("expected fake ACP response");
        };
        assert_eq!(id, prompt_id);
        assert!(error.is_none());
        assert_eq!(stop_reason(&result.unwrap()), Some("end_turn"));
        drop(client);
        std::fs::remove_dir_all(root).ok();
    }
}
