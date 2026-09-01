use crate::adapter::{self, AdapterKind, CliSpec};
use crate::model::{Router, Task, TaskSpec, ThinkingLevel};
use serde::Serialize;
use serde_json::{json, Value};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub type Result<T> = std::result::Result<T, String>;

#[derive(Debug, Serialize)]
pub struct ObserverOutput {
    pub route: String,
    pub provider: String,
    pub model: String,
    pub content: String,
}

pub fn run(
    router: &Router,
    route_name: &str,
    prompt: &str,
    thinking: ThinkingLevel,
) -> Result<ObserverOutput> {
    if prompt.trim().is_empty() {
        return Err("observer prompt is empty".to_string());
    }
    let route = router.resolve_route(route_name).to_string();
    let provider = router
        .providers
        .get(&route)
        .cloned()
        .ok_or_else(|| format!("unknown observer route '{route_name}'"))?;
    if !provider.enabled {
        return Err(format!("observer route '{route}' is disabled"));
    }
    let kind = AdapterKind::from_wrapper(&provider.wrapper)
        .ok_or_else(|| format!("unsupported observer wrapper '{}'", provider.wrapper))?;
    if kind == AdapterKind::ChatGptChat {
        return Err("chatgpt_chat cannot observe its own Goal loop".to_string());
    }

    let spec: TaskSpec = serde_json::from_value(json!({
        "id": "goal-observer",
        "route": route,
        "task": "Evaluate whether a ChatGPT Goal should continue",
        "role": "observer",
        "tools_policy": "none"
    }))
    .map_err(|e| e.to_string())?;
    let task = Task {
        id: "goal-observer".to_string(),
        source_id: "goal-observer".to_string(),
        stage: "observer".to_string(),
        stage_parallel: false,
        spec,
        provider: provider.clone(),
        effective_route: route.clone(),
    };

    let temp = observer_temp_dir()?;
    fs::create_dir_all(&temp).map_err(|e| format!("create observer temp dir: {e}"))?;
    let result = match kind {
        // Observer mock is an offline transport smoke, not the task-fixture mock adapter.
        AdapterKind::Mock => Ok(prompt.to_string()),
        AdapterKind::OpenAiCompat => {
            adapter::execute_openai_compat(&task, prompt, thinking).map(|out| out.content)
        }
        AdapterKind::Agy => execute_agy(&task, prompt, thinking, &temp),
        AdapterKind::Codex
        | AdapterKind::OpenCode
        | AdapterKind::OpenCode2
        | AdapterKind::Kilo
        | AdapterKind::Claude
        | AdapterKind::Hermes
        | AdapterKind::Perch
        | AdapterKind::Pi => execute_cli_observer(kind, &task, prompt, thinking, &temp),
        AdapterKind::ChatGptChat => unreachable!(),
    };
    let _ = fs::remove_dir_all(&temp);
    let content = result?.trim().to_string();
    if content.is_empty() {
        return Err(format!("observer route '{route}' produced no content"));
    }
    Ok(ObserverOutput {
        route,
        provider: provider.provider,
        model: provider.model,
        content,
    })
}

fn observer_temp_dir() -> Result<PathBuf> {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_nanos();
    Ok(env::temp_dir().join(format!(
        "swarms-goal-observer-{}-{stamp}",
        std::process::id()
    )))
}

fn execute_cli_observer(
    kind: AdapterKind,
    task: &Task,
    prompt: &str,
    thinking: ThinkingLevel,
    cwd: &Path,
) -> Result<String> {
    let spec =
        adapter::build_cli_command(kind, task, prompt, thinking, None, &task.provider.provider)?;
    let raw = run_cli(spec, cwd)?;
    extract_final_text(kind, &raw)
}

fn run_cli(spec: CliSpec, cwd: &Path) -> Result<String> {
    let output = Command::new(&spec.program)
        .args(&spec.args)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .envs(spec.env.iter().cloned())
        .output()
        .map_err(|e| format!("spawn '{}': {e}", spec.program))?;
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    if !output.status.success() {
        return Err(format!(
            "observer process '{}' exited {:?}: {}",
            spec.program,
            output.status.code(),
            tail(&format!("{stdout}\n{stderr}"), 2000)
        ));
    }
    Ok(stdout)
}

fn execute_agy(task: &Task, prompt: &str, thinking: ThinkingLevel, cwd: &Path) -> Result<String> {
    let marker = format!(
        "SWARMS_GOAL_OBSERVER_{}_{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    );
    let marked = format!("{prompt}\n\nObserver correlation marker: {marker}. Do not mention this marker in your answer.");
    let started = SystemTime::now();
    let spec = adapter::build_cli_command(
        AdapterKind::Agy,
        task,
        &marked,
        thinking,
        None,
        &task.provider.provider,
    )?;
    let direct = run_cli(spec, cwd)?;
    if let Some(answer) = find_agy_answer(&marker, started) {
        return Ok(answer);
    }
    if !direct.trim().is_empty() {
        return Ok(direct);
    }
    Err("agy completed but no persisted assistant answer was found".to_string())
}

fn find_agy_answer(marker: &str, started: SystemTime) -> Option<String> {
    let home = env::var_os("USERPROFILE").or_else(|| env::var_os("HOME"))?;
    let brain = PathBuf::from(home)
        .join(".gemini")
        .join("antigravity-cli")
        .join("brain");
    let deadline = std::time::Instant::now() + Duration::from_secs(8);
    loop {
        let mut candidates: Vec<(SystemTime, PathBuf)> = Vec::new();
        if let Ok(entries) = fs::read_dir(&brain) {
            for entry in entries.flatten() {
                let base = entry.path().join(".system_generated").join("logs");
                for name in ["transcript.jsonl", "transcript_full.jsonl"] {
                    let path = base.join(name);
                    let Ok(meta) = fs::metadata(&path) else {
                        continue;
                    };
                    let modified = meta.modified().unwrap_or(UNIX_EPOCH);
                    if modified
                        >= started
                            .checked_sub(Duration::from_secs(2))
                            .unwrap_or(UNIX_EPOCH)
                    {
                        candidates.push((modified, path));
                    }
                }
            }
        }
        candidates.sort_by_key(|item| std::cmp::Reverse(item.0));
        for (_, path) in candidates {
            let Ok(text) = fs::read_to_string(&path) else {
                continue;
            };
            if !text.contains(marker) {
                continue;
            }
            if let Some(answer) = last_planner_response(&text) {
                return Some(answer);
            }
        }
        if std::time::Instant::now() >= deadline {
            return None;
        }
        thread::sleep(Duration::from_millis(200));
    }
}

fn last_planner_response(text: &str) -> Option<String> {
    let mut last = None;
    for line in text.lines() {
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if value.get("type").and_then(Value::as_str) == Some("PLANNER_RESPONSE") {
            if let Some(content) = value.get("content").and_then(Value::as_str) {
                if !content.trim().is_empty() {
                    last = Some(content.trim().to_string());
                }
            }
        }
    }
    last
}

fn extract_final_text(kind: AdapterKind, raw: &str) -> Result<String> {
    match kind {
        AdapterKind::OpenCode | AdapterKind::OpenCode2 | AdapterKind::Kilo => {
            let mut chunks = Vec::new();
            for line in raw.lines() {
                let Ok(value) = serde_json::from_str::<Value>(line) else {
                    continue;
                };
                if value.get("type").and_then(Value::as_str) == Some("error") {
                    return Err(format!("OpenCode observer error: {}", tail(line, 1000)));
                }
                if value.get("type").and_then(Value::as_str) == Some("text") {
                    if let Some(text) = value.pointer("/part/text").and_then(Value::as_str) {
                        chunks.push(text.to_string());
                    }
                }
            }
            if chunks.is_empty() {
                Ok(raw.to_string())
            } else {
                Ok(chunks.join(""))
            }
        }
        AdapterKind::Codex => {
            let mut last = None;
            for line in raw.lines() {
                let Ok(value) = serde_json::from_str::<Value>(line) else {
                    continue;
                };
                if value.get("type").and_then(Value::as_str) == Some("item.completed")
                    && value.pointer("/item/type").and_then(Value::as_str) == Some("agent_message")
                {
                    if let Some(text) = value.pointer("/item/text").and_then(Value::as_str) {
                        last = Some(text.to_string());
                    }
                }
            }
            last.ok_or_else(|| "Codex observer produced no final agent_message".to_string())
        }
        _ => Ok(raw.to_string()),
    }
}

fn tail(text: &str, limit: usize) -> String {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= limit {
        text.to_string()
    } else {
        chars[chars.len() - limit..].iter().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_opencode_text_events() {
        let raw = "{\"type\":\"text\",\"part\":{\"text\":\"{\\\"action\\\":\"}}\n{\"type\":\"text\",\"part\":{\"text\":\"\\\"stop\\\"}\"}}";
        assert_eq!(
            extract_final_text(AdapterKind::OpenCode, raw).unwrap(),
            "{\"action\":\"stop\"}"
        );
    }

    #[test]
    fn extracts_codex_agent_message() {
        let raw = "{\"type\":\"thread.started\",\"thread_id\":\"x\"}\n{\"type\":\"item.completed\",\"item\":{\"type\":\"agent_message\",\"text\":\"done\"}}";
        assert_eq!(extract_final_text(AdapterKind::Codex, raw).unwrap(), "done");
    }

    #[test]
    fn parses_agy_planner_response() {
        let raw = "{\"type\":\"USER\",\"content\":\"x\"}\n{\"type\":\"PLANNER_RESPONSE\",\"content\":\"answer\"}";
        assert_eq!(last_planner_response(raw).as_deref(), Some("answer"));
    }
}
