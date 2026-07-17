//! Native Rust adapters: command construction, mock execution, and HTTP.
//!
//! No adapter invokes Python.  The mock adapter is in-process; CLI adapters
//! produce [`CliSpec`] structs that the runtime executes via [`std::process`];
//! `openai_compat` uses [`ureq`] for native HTTPS.

use crate::model::{Provider, Task, ThinkingLevel};
use crate::telemetry::Usage;
use serde_json::{json, Value};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

type Result<T> = std::result::Result<T, String>;

// ---------------------------------------------------------------------------
// Adapter kind
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AdapterKind {
    Mock,
    Codex,
    OpenCode,
    Kilo,
    Hermes,
    Agy,
    OpenAiCompat,
}

impl AdapterKind {
    pub fn from_wrapper(wrapper: &str) -> Option<Self> {
        match wrapper {
            "mock" => Some(Self::Mock),
            "codex" => Some(Self::Codex),
            "opencode" => Some(Self::OpenCode),
            "kilo" => Some(Self::Kilo),
            "hermes" => Some(Self::Hermes),
            "gemini" => Some(Self::Agy),
            "openai_compat" => Some(Self::OpenAiCompat),
            _ => None,
        }
    }

    /// Whether this adapter has a native thinking/reasoning flag.
    pub fn supports_thinking(&self) -> bool {
        matches!(self, Self::Codex | Self::OpenCode | Self::Kilo)
    }

    /// Whether this adapter can capture and resume a session ID.
    pub fn supports_session_reuse(&self) -> bool {
        matches!(self, Self::Codex | Self::OpenCode | Self::Kilo)
    }
}

// ---------------------------------------------------------------------------
// CLI specification (for testable command construction)
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct CliSpec {
    pub program: String,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
}

// ---------------------------------------------------------------------------
// Prompt generation (cache-friendly: stable prefix, dynamic suffix)
// ---------------------------------------------------------------------------

/// Stable prefix placed before dynamic content in every prompt.
pub const PROMPT_PREFIX: &str = "\
You are a SWARMS worker with a narrow task.
Return only the required result and keep output concise.
For coding, planning, and review, apply Ponytail/full: choose the smallest correct solution; reuse existing code, then the standard library, native platform, or installed dependencies; avoid speculative abstractions and new dependencies; fix root causes; never omit required validation, security, or error handling.
When .codegraph exists and tools are available, prefer CodeGraph for codebase exploration and impact analysis.\n\n---\n\n";

pub fn build_prompt(
    role: &str,
    task_text: &str,
    artifacts: &[String],
    dependency_context: &str,
) -> String {
    let mut lines = Vec::new();
    lines.push(PROMPT_PREFIX.to_string());
    lines.push(format!("Role: {role}"));
    lines.push(format!("Task: {task_text}"));
    if artifacts.is_empty() {
        lines.push("Allowed artifacts: (task-defined)".to_string());
    } else {
        lines.push(format!("Allowed artifacts: {}", artifacts.join(", ")));
    }
    if !dependency_context.is_empty() {
        lines.push(String::new());
        lines.push("Use these completed dependency outputs as input:".to_string());
        lines.push(dependency_context.to_string());
    }
    lines.join("\n")
}

// ---------------------------------------------------------------------------
// CLI command builders
// ---------------------------------------------------------------------------

pub fn build_cli_command(
    kind: AdapterKind,
    task: &Task,
    prompt_text: &str,
    thinking: ThinkingLevel,
    session_id: Option<&str>,
    provider_name: &str,
) -> Result<CliSpec> {
    match kind {
        AdapterKind::Codex => build_codex(task, prompt_text, thinking, session_id),
        AdapterKind::OpenCode => build_opencode(task, prompt_text, thinking, session_id),
        AdapterKind::Kilo => build_kilo(task, prompt_text, thinking, session_id),
        AdapterKind::Hermes => build_hermes(task, prompt_text, provider_name),
        AdapterKind::Agy => build_agy(task, prompt_text),
        _ => Err(format!("not a CLI adapter: {kind:?}")),
    }
}

fn build_codex(
    task: &Task,
    prompt_text: &str,
    thinking: ThinkingLevel,
    session_id: Option<&str>,
) -> Result<CliSpec> {
    let program = which("codex").unwrap_or_else(|| "codex".to_string());
    let sandbox = if task.spec.tools_policy == "full" {
        "workspace-write"
    } else {
        "read-only"
    };
    let mut args = Vec::new();

    if let Some(sid) = session_id {
        args.push("exec".to_string());
        args.push("resume".to_string());
        args.push(sid.to_string());
    } else {
        args.push("exec".to_string());
    }
    args.push("-m".to_string());
    args.push(task.provider.model.clone());
    args.push("-s".to_string());
    args.push(sandbox.to_string());
    if let Some(effort) = thinking.as_codex_str() {
        args.push("-c".to_string());
        args.push(format!("model_reasoning_effort={effort}"));
    }
    args.push("--json".to_string());
    args.push(prompt_text.to_string());

    Ok(CliSpec {
        program,
        args,
        env: Vec::new(),
    })
}

fn build_opencode(
    task: &Task,
    prompt_text: &str,
    thinking: ThinkingLevel,
    session_id: Option<&str>,
) -> Result<CliSpec> {
    build_opencode_family("opencode", task, prompt_text, thinking, session_id)
}

fn build_kilo(
    task: &Task,
    prompt_text: &str,
    thinking: ThinkingLevel,
    session_id: Option<&str>,
) -> Result<CliSpec> {
    build_opencode_family("kilo", task, prompt_text, thinking, session_id)
}

fn build_opencode_family(
    bin_name: &str,
    task: &Task,
    prompt_text: &str,
    thinking: ThinkingLevel,
    session_id: Option<&str>,
) -> Result<CliSpec> {
    let program = which(bin_name).unwrap_or_else(|| bin_name.to_string());
    let mut args = vec!["run".to_string()];

    if !task.provider.model.is_empty() {
        args.push("-m".to_string());
        args.push(task.provider.model.clone());
    }
    args.push("--format".to_string());
    args.push("json".to_string());
    if !thinking.is_default() {
        args.push("--variant".to_string());
        args.push(thinking.as_str().to_string());
    }
    if task.spec.tools_policy == "full" {
        args.push("--auto".to_string());
    } else {
        args.push("--pure".to_string());
    }
    if let Some(sid) = session_id {
        args.push("--session".to_string());
        args.push(sid.to_string());
    }
    args.push(prompt_text.to_string());

    Ok(CliSpec {
        program,
        args,
        env: Vec::new(),
    })
}

fn build_hermes(task: &Task, prompt_text: &str, provider_name: &str) -> Result<CliSpec> {
    let program = which("hermes").unwrap_or_else(|| "hermes".to_string());
    let mut args = vec!["chat".to_string(), "-Q".to_string()];

    let max_turns = env::var("HERMES_MAX_TURNS").unwrap_or_else(|_| "8".to_string());
    args.push("--max-turns".to_string());
    args.push(max_turns);

    if !task.provider.model.is_empty() {
        args.push("-m".to_string());
        args.push(task.provider.model.clone());
    }
    // Force Nous Portal for HY3 models (free tier).
    if task.provider.model.starts_with("tencent/hy3") {
        args.push("--provider".to_string());
        args.push("nous".to_string());
    } else if provider_name != "hermes" && !provider_name.is_empty() {
        args.push("--provider".to_string());
        args.push(provider_name.to_string());
    }
    if task.spec.tools_policy == "full" {
        args.push("--yolo".to_string());
    }
    args.push("-q".to_string());
    args.push(prompt_text.to_string());

    Ok(CliSpec {
        program,
        args,
        env: Vec::new(),
    })
}

fn build_agy(task: &Task, prompt_text: &str) -> Result<CliSpec> {
    let program = which("agy").unwrap_or_else(|| "agy".to_string());
    let mut args = vec!["--print".to_string()];

    if !task.provider.model.is_empty() {
        args.push("--model".to_string());
        args.push(task.provider.model.clone());
    }
    if task.spec.tools_policy == "full" {
        args.push("--mode".to_string());
        args.push("accept-edits".to_string());
        args.push("--dangerously-skip-permissions".to_string());
    } else {
        args.push("--sandbox".to_string());
    }
    args.push(prompt_text.to_string());

    Ok(CliSpec {
        program,
        args,
        env: Vec::new(),
    })
}

// ---------------------------------------------------------------------------
// Session ID parsing
// ---------------------------------------------------------------------------

/// Attempt to extract a provider session ID from structured adapter output.
/// Returns `None` if no reliable ID is found — never guesses.
///
/// - Codex JSONL: searches each line recursively for `thread_id`.
/// - OpenCode/Kilo: tries single-JSON then JSONL, searching recursively.
pub fn parse_session_id(kind: AdapterKind, output: &str) -> Option<String> {
    let trimmed = output.trim();
    match kind {
        AdapterKind::Codex => {
            for line in trimmed.lines() {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }
                if let Ok(v) = serde_json::from_str::<Value>(line) {
                    if let Some(id) =
                        find_id_recursive(&v, &["thread_id", "session_id", "sessionID"])
                    {
                        return Some(id);
                    }
                }
            }
            None
        }
        AdapterKind::OpenCode | AdapterKind::Kilo => {
            // Try as a single JSON object first, then as JSONL.
            if let Ok(v) = serde_json::from_str::<Value>(trimmed) {
                if let Some(id) = find_id_recursive(&v, &["sessionID", "session_id", "session"]) {
                    return Some(id);
                }
            }
            for line in trimmed.lines() {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }
                if let Ok(v) = serde_json::from_str::<Value>(line) {
                    if let Some(id) = find_id_recursive(&v, &["sessionID", "session_id", "session"])
                    {
                        return Some(id);
                    }
                }
            }
            None
        }
        _ => None,
    }
}

/// Recursively search a JSON value for the first non-empty string matching
/// any of the candidate keys.
fn find_id_recursive(v: &Value, keys: &[&str]) -> Option<String> {
    for key in keys {
        if let Some(s) = v.get(*key).and_then(|f| f.as_str()) {
            if !s.is_empty() {
                return Some(s.to_string());
            }
        }
    }
    if let Some(obj) = v.as_object() {
        for (_, val) in obj {
            if val.is_object() || val.is_array() {
                if let Some(id) = find_id_recursive(val, keys) {
                    return Some(id);
                }
            }
        }
    }
    if let Some(arr) = v.as_array() {
        for val in arr {
            if let Some(id) = find_id_recursive(val, keys) {
                return Some(id);
            }
        }
    }
    None
}

/// Parse token telemetry exposed by structured CLI output.
///
/// OpenCode/Kilo emit one JSON object per step with `part.tokens`; Codex emits
/// JSONL events with a `usage` object. Unknown shapes remain `missing` rather
/// than being reported as zero usage.
pub fn parse_cli_usage(kind: AdapterKind, output: &str) -> Usage {
    if !matches!(
        kind,
        AdapterKind::Codex | AdapterKind::OpenCode | AdapterKind::Kilo
    ) {
        return Usage::missing();
    }

    let mut totals = [0_u64; 5];
    let mut found = false;
    for line in output
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
    {
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let usage = value
            .pointer("/part/tokens")
            .or_else(|| value.get("usage"))
            .or_else(|| value.pointer("/event/usage"));
        let Some(usage) = usage else { continue };
        found = true;
        totals[0] += usage_u64(usage, &["input", "input_tokens", "prompt_tokens"]);
        totals[1] += usage
            .pointer("/cache/read")
            .and_then(Value::as_u64)
            .unwrap_or_else(|| usage_u64(usage, &["cached_input_tokens", "cache_read_tokens"]));
        totals[2] += usage
            .pointer("/cache/write")
            .and_then(Value::as_u64)
            .unwrap_or_else(|| usage_u64(usage, &["cache_write_tokens"]));
        totals[3] += usage_u64(usage, &["output", "output_tokens", "completion_tokens"]);
        totals[4] += usage_u64(usage, &["reasoning", "reasoning_tokens"]);
    }
    if !found {
        return Usage::missing();
    }
    Usage {
        input: totals[0].to_string(),
        cache_read: totals[1].to_string(),
        cache_write: totals[2].to_string(),
        output: totals[3].to_string(),
        reasoning: totals[4].to_string(),
    }
}

fn usage_u64(value: &Value, keys: &[&str]) -> u64 {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(Value::as_u64))
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------
// Mock adapter (in-process deterministic execution)
// ---------------------------------------------------------------------------

pub struct MockOutput {
    pub stdout: String,
}

pub fn execute_mock(root: &Path, prompt: &str) -> Result<MockOutput> {
    let mut did_work = false;

    if prompt.contains("reshard_plan.md") {
        write_mock_file(root, "docs/bench_notes/reshard_plan.md", RESHARD_PLAN_MD)?;
        did_work = true;
    }
    if prompt.contains("compress.py") && prompt.contains("Implement") {
        write_mock_file(root, "bench_apps/reshard/compress.py", COMPRESS_PY)?;
        did_work = true;
    }
    if prompt.contains("decompress.py") && prompt.contains("Implement") {
        write_mock_file(root, "bench_apps/reshard/decompress.py", DECOMPRESS_PY)?;
        did_work = true;
    }
    if prompt.contains("bench_tests/test_bench_reshard.py") && prompt.contains("Create") {
        write_mock_file(root, "bench_tests/test_bench_reshard.py", TEST_RESHARD_PY)?;
        did_work = true;
    }
    if prompt.contains("Run pytest") {
        return Ok(MockOutput {
            stdout: "mock verification task completed".to_string(),
        });
    }
    if !did_work {
        return Err("mock worker found no matching deterministic task".to_string());
    }
    Ok(MockOutput {
        stdout: "mock worker completed deterministic edits".to_string(),
    })
}

fn write_mock_file(root: &Path, rel: &str, content: &str) -> Result<()> {
    let path = root.join(rel);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("mkdir {}: {e}", parent.display()))?;
    }
    fs::write(&path, content).map_err(|e| format!("write {}: {e}", path.display()))
}

const RESHARD_PLAN_MD: &str = "\
# Reshard Roundtrip Plan

Files are copied into deterministic `shard_N` folders, with at most
three files per shard. Decompression reconstructs a flat target
directory and refuses path traversal outside the requested output.
Verification covers empty inputs, deterministic shard naming,
roundtrip content, and output-boundary checks.
";

const COMPRESS_PY: &str = "\
from __future__ import annotations

import shutil
from pathlib import Path


def _safe_child(root: Path, name: str) -> Path:
    target = (root / name).resolve()
    target.relative_to(root.resolve())
    return target


def compress(input_dir: str | Path, output_dir: str | Path, files_per_shard: int = 3) -> list[Path]:
    source = Path(input_dir)
    target = Path(output_dir)
    if files_per_shard <= 0:
        raise ValueError(\"files_per_shard must be positive\")
    if not source.is_dir():
        raise FileNotFoundError(source)
    target.mkdir(parents=True, exist_ok=True)
    files = sorted(path for path in source.iterdir() if path.is_file())
    shards: list[Path] = []
    for index in range(0, len(files), files_per_shard):
        shard = _safe_child(target, f\"shard_{len(shards)}\")
        shard.mkdir(exist_ok=True)
        shards.append(shard)
        for file_path in files[index : index + files_per_shard]:
            shutil.copy2(file_path, _safe_child(shard, file_path.name))
    return shards
";

const DECOMPRESS_PY: &str = "\
from __future__ import annotations

import shutil
from pathlib import Path


def _safe_child(root: Path, name: str) -> Path:
    target = (root / name).resolve()
    target.relative_to(root.resolve())
    return target


def decompress(sharded_dir: str | Path, output_dir: str | Path) -> list[Path]:
    source = Path(sharded_dir)
    target = Path(output_dir)
    if not source.is_dir():
        raise FileNotFoundError(source)
    target.mkdir(parents=True, exist_ok=True)
    written: list[Path] = []
    for shard in sorted(path for path in source.iterdir() if path.is_dir()):
        if not shard.name.startswith(\"shard_\"):
            continue
        for file_path in sorted(path for path in shard.iterdir() if path.is_file()):
            destination = _safe_child(target, file_path.name)
            shutil.copy2(file_path, destination)
            written.append(destination)
    return written
";

const TEST_RESHARD_PY: &str = "\
from pathlib import Path

from bench_apps.reshard.compress import compress
from bench_apps.reshard.decompress import decompress


def test_roundtrip_and_deterministic_shards(tmp_path: Path):
    source = tmp_path / \"source\"
    packed = tmp_path / \"packed\"
    restored = tmp_path / \"restored\"
    source.mkdir()
    for name, body in {\"b.txt\": \"B\", \"a.txt\": \"A\", \"c.txt\": \"C\", \"d.txt\": \"D\"}.items():
        (source / name).write_text(body, encoding=\"utf-8\")

    shards = compress(source, packed, files_per_shard=3)
    assert [path.name for path in shards] == [\"shard_0\", \"shard_1\"]
    assert sorted(path.name for path in (packed / \"shard_0\").iterdir()) == [\"a.txt\", \"b.txt\", \"c.txt\"]

    decompress(packed, restored)
    assert {path.name: path.read_text(encoding=\"utf-8\") for path in restored.iterdir()} == {
        \"a.txt\": \"A\",
        \"b.txt\": \"B\",
        \"c.txt\": \"C\",
        \"d.txt\": \"D\",
    }


def test_empty_input_has_no_shards(tmp_path: Path):
    source = tmp_path / \"source\"
    packed = tmp_path / \"packed\"
    source.mkdir()
    assert compress(source, packed) == []
    assert packed.exists()
";

// ---------------------------------------------------------------------------
// OpenAI-compatible HTTP adapter
// ---------------------------------------------------------------------------

pub struct OpenAiCompatOutput {
    pub content: String,
    pub usage: Usage,
}

pub fn execute_openai_compat(
    task: &Task,
    prompt: &str,
    thinking: ThinkingLevel,
    timeout: Duration,
) -> Result<OpenAiCompatOutput> {
    let provider_name = &task.provider.provider;
    let key_env = task
        .provider
        .key_env
        .as_deref()
        .or_else(|| default_key_env(provider_name))
        .ok_or_else(|| format!("no key_env for provider '{provider_name}'"))?;
    let key = env::var(key_env).map_err(|_| format!("API key not set in env var {key_env}"))?;

    let base_url = resolve_base_url(&task.provider)?;
    validate_url(&base_url)?;
    let url = format!("{}/chat/completions", base_url.trim_end_matches('/'));

    let mut body = json!({
        "model": task.provider.model,
        "messages": [
            {"role": "system", "content": "You are a SWARMS worker. Return only the required result."},
            {"role": "user", "content": prompt}
        ],
        "temperature": 0.2,
    });

    if !thinking.is_default() {
        if let Some(field) = &task.provider.thinking_field {
            if let Some(obj) = body.as_object_mut() {
                obj.insert(field.clone(), json!(thinking.as_str()));
            }
        } else {
            return Err(format!(
                "thinking {:?} requested but route '{}' has no thinking_field",
                thinking, task.spec.route
            ));
        }
    }

    let response = ureq::post(&url)
        .set("Authorization", &format!("Bearer {key}"))
        .timeout(timeout)
        .send_json(body)
        .map_err(|e| format!("HTTP request to {url} failed: {e}"))?;

    let resp_json: Value = response
        .into_json()
        .map_err(|e| format!("failed to parse HTTP response: {e}"))?;

    let content = resp_json
        .get("choices")
        .and_then(|c| c.get(0))
        .and_then(|c| c.get("message"))
        .and_then(|m| m.get("content"))
        .and_then(|c| c.as_str())
        .ok_or_else(|| "no content in response".to_string())?;

    let usage = parse_openai_usage(&resp_json);

    Ok(OpenAiCompatOutput {
        content: content.to_string(),
        usage,
    })
}

fn parse_openai_usage(resp: &Value) -> Usage {
    let u = resp.get("usage");
    Usage {
        input: u
            .and_then(|u| u.get("prompt_tokens"))
            .and_then(|v| v.as_u64())
            .map_or_else(|| "missing".to_string(), |n| n.to_string()),
        cache_read: u
            .and_then(|u| u.get("prompt_tokens_details"))
            .and_then(|d| d.get("cached_tokens"))
            .and_then(|v| v.as_u64())
            .map_or_else(|| "missing".to_string(), |n| n.to_string()),
        cache_write: u
            .and_then(|u| u.get("cache_creation_input_tokens"))
            .and_then(|v| v.as_u64())
            .map_or_else(|| "missing".to_string(), |n| n.to_string()),
        output: u
            .and_then(|u| u.get("completion_tokens"))
            .and_then(|v| v.as_u64())
            .map_or_else(|| "missing".to_string(), |n| n.to_string()),
        reasoning: u
            .and_then(|u| u.get("completion_tokens_details"))
            .and_then(|d| d.get("reasoning_tokens"))
            .and_then(|v| v.as_u64())
            .map_or_else(|| "missing".to_string(), |n| n.to_string()),
    }
}

fn default_key_env(provider: &str) -> Option<&'static str> {
    match provider {
        "openrouter" => Some("OPENROUTER_API_KEY"),
        "novita" => Some("NOVITA_API_KEY"),
        "gitlawb" => Some("GITLAWB_API_KEY"),
        "siliconflow" => Some("SILICONFLOW_API_KEY"),
        "kilo" | "kilo_cli" => Some("KILO_API_KEY"),
        _ => Some("OPENAI_COMPAT_API_KEY"),
    }
}

fn resolve_base_url(provider: &Provider) -> Result<String> {
    if let Some(url) = &provider.base_url {
        return Ok(url.clone());
    }
    let env_name = provider
        .base_url_env
        .as_deref()
        .unwrap_or_else(|| default_base_url_env(&provider.provider));
    if let Ok(val) = env::var(env_name) {
        if !val.is_empty() {
            return Ok(val);
        }
    }
    default_base_url(&provider.provider).ok_or_else(|| {
        format!(
            "no base_url for provider '{}'; set {} or configure base_url",
            provider.provider, env_name
        )
    })
}

fn default_base_url_env(provider: &str) -> &str {
    match provider {
        "openrouter" => "OPENROUTER_BASE_URL",
        "gitlawb" => "GITLAWB_BASE_URL",
        _ => "OPENAI_COMPAT_BASE_URL",
    }
}

fn default_base_url(provider: &str) -> Option<String> {
    match provider {
        "openrouter" => Some("https://openrouter.ai/api/v1".to_string()),
        "gitlawb" => Some("https://opengateway.gitlawb.com/v1".to_string()),
        _ => None,
    }
}

/// Reject non-HTTPS URLs and URLs with embedded credentials.
pub fn validate_url(url: &str) -> Result<()> {
    if let Some(rest) = url.strip_prefix("https://") {
        if rest.contains('@') {
            return Err("URL must not contain embedded credentials".to_string());
        }
        return Ok(());
    }
    if let Some(rest) = url.strip_prefix("http://") {
        let host = rest.split(['/', ':']).next().unwrap_or("");
        if matches!(host, "localhost" | "127.0.0.1" | "::1") {
            if rest.contains('@') {
                return Err("URL must not contain embedded credentials".to_string());
            }
            return Ok(());
        }
        return Err(format!(
            "non-HTTPS URL to non-loopback host is unsafe: {url}"
        ));
    }
    Err(format!("URL must use http(s) scheme: {url}"))
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn which(bin_name: &str) -> Option<String> {
    let var = if cfg!(windows) { "Path" } else { "PATH" };
    let path = env::var(var).ok()?;
    let ext = if cfg!(windows) { ".exe" } else { "" };
    for dir in path.split(if cfg!(windows) { ';' } else { ':' }) {
        let candidate = PathBuf::from(dir).join(format!("{bin_name}{ext}"));
        if candidate.is_file() {
            return Some(candidate.to_string_lossy().to_string());
        }
    }
    None
}

/// Resolve prompt file path for adapters that read from a file.
pub fn prompt_file_path(work_dir: &Path, task_id: &str) -> PathBuf {
    work_dir.join(format!("{task_id}.prompt.txt"))
}
