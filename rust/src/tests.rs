use crate::adapter::{self, AdapterKind, CliSpec, PROMPT_PREFIX};
use crate::cli::safe_run_id;
use crate::model::{
    self, find_dependency_task, OnUnknownQuota, Provider, QuotaPolicy, Router, SessionConfig,
    SessionMode, Task, TaskSpec, ThinkingLevel,
};
use crate::review::{detect_cycles, review_plan, validate_artifact_path, Severity};
use crate::runtime::{self, check_artifacts};
use crate::session::{self, SessionDecision, SessionStore};
use crate::telemetry::{TaskState, TaskStatus};
use serde_json::json;
use std::collections::HashMap;
use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn mock_provider() -> Provider {
    Provider {
        enabled: true,
        provider: "mock".to_string(),
        model: "mock-worker".to_string(),
        canonical_model: None,
        wrapper: "mock".to_string(),
        key_env: None,
        base_url: None,
        base_url_env: None,
        thinking_field: None,
        quota_key: None,
        fallback_routes: Vec::new(),
    }
}

#[allow(dead_code)]
fn mock_router() -> Router {
    let mut providers = HashMap::new();
    providers.insert("mock".to_string(), mock_provider());
    Router {
        fallback_route: Some("mock".to_string()),
        aliases: HashMap::new(),
        role_routes: HashMap::new(),
        quota_policy: QuotaPolicy::default(),
        providers,
    }
}

fn make_task(id: &str, needs: &[&str], route: &str) -> Task {
    let mut providers = HashMap::new();
    providers.insert("mock".to_string(), mock_provider());
    providers.insert(
        "codex".to_string(),
        Provider {
            enabled: true,
            provider: "codex_cli".to_string(),
            model: "gpt-5.5-codex".to_string(),
            canonical_model: None,
            wrapper: "codex".to_string(),
            key_env: None,
            base_url: None,
            base_url_env: None,
            thinking_field: None,
            quota_key: None,
            fallback_routes: Vec::new(),
        },
    );
    providers.insert(
        "oc".to_string(),
        Provider {
            enabled: true,
            provider: "opencode".to_string(),
            model: "zai-coding-plan/glm-5.2".to_string(),
            canonical_model: None,
            wrapper: "opencode".to_string(),
            key_env: None,
            base_url: None,
            base_url_env: None,
            thinking_field: None,
            quota_key: None,
            fallback_routes: Vec::new(),
        },
    );
    let spec = TaskSpec {
        id: id.to_string(),
        route: route.to_string(),
        task: format!("task {id}"),
        role: "general".to_string(),
        needs: needs.iter().map(|s| s.to_string()).collect(),
        tools_policy: "none".to_string(),
        artifacts: Vec::new(),
        verify: Vec::new(),
        thinking: None,
        session: None,
        timeout_seconds: None,
        max_attempts: None,
    };
    let provider = providers.get(route).cloned().unwrap_or_else(mock_provider);
    Task {
        id: model::task_index_to_id(0, id),
        source_id: id.to_string(),
        stage: "test".to_string(),
        stage_parallel: true,
        spec,
        provider,
        effective_route: route.to_string(),
    }
}

// ---------------------------------------------------------------------------
// 1. DAG cycle detection
// ---------------------------------------------------------------------------

#[test]
fn dag_cycle_simple_pair() {
    let tasks = vec![
        make_task("a", &["b"], "mock"),
        make_task("b", &["a"], "mock"),
    ];
    let cycles = detect_cycles(&tasks);
    assert!(!cycles.is_empty(), "expected at least one cycle");
}

#[test]
fn dag_no_cycle_linear_chain() {
    let tasks = vec![
        make_task("a", &[], "mock"),
        make_task("b", &["a"], "mock"),
        make_task("c", &["b"], "mock"),
    ];
    let cycles = detect_cycles(&tasks);
    assert!(cycles.is_empty(), "no cycles expected, got {cycles:?}");
}

#[test]
fn dag_self_dependency() {
    let tasks = vec![make_task("a", &["a"], "mock")];
    let cycles = detect_cycles(&tasks);
    assert!(!cycles.is_empty(), "self-dependency should be detected");
}

#[test]
fn legacy_timeout_fields_never_create_a_worker_deadline() {
    let mut task = make_task("long", &[], "mock");
    task.spec.timeout_seconds = Some(1);
    let plan = model::Plan {
        schema_version: None,
        goal: None,
        project: None,
        planner: None,
        review_policy: None,
        budget_policy: model::BudgetPolicy::default(),
        stages: Vec::new(),
        thinking: None,
        session: None,
        default_timeout_seconds: Some(1),
        default_max_attempts: None,
    };
    assert_eq!(task.spec.effective_timeout(&plan), None);
}

// ---------------------------------------------------------------------------
// 2. Adapter command mappings
// ---------------------------------------------------------------------------

#[test]
fn codex_command_maps_thinking_and_resume() {
    let task = make_task("t1", &[], "codex");
    let spec: CliSpec = adapter::build_cli_command(
        AdapterKind::Codex,
        &task,
        "do work",
        ThinkingLevel::High,
        None,
        "codex_cli",
    )
    .unwrap();

    assert!(spec.program.contains("codex"));
    assert!(spec.args.contains(&"exec".to_string()));
    assert!(spec.args.contains(&"-m".to_string()));
    assert!(spec.args.contains(&"gpt-5.5-codex".to_string()));
    assert!(spec.args.iter().any(|a| a == "model_reasoning_effort=high"));
    assert!(spec.args.contains(&"--json".to_string()));
    // resume subcommand when session_id present
    let spec2 = adapter::build_cli_command(
        AdapterKind::Codex,
        &task,
        "do work",
        ThinkingLevel::High,
        Some("sess-abc"),
        "codex_cli",
    )
    .unwrap();
    assert!(spec2.args.contains(&"resume".to_string()));
    assert!(spec2.args.contains(&"sess-abc".to_string()));
}

#[test]
fn codex_max_maps_to_verified_ultra_effort() {
    let task = make_task("t1", &[], "codex");
    let spec = adapter::build_cli_command(
        AdapterKind::Codex,
        &task,
        "do work",
        ThinkingLevel::Max,
        None,
        "codex_cli",
    )
    .unwrap();
    assert!(spec
        .args
        .iter()
        .any(|arg| arg == "model_reasoning_effort=ultra"));
}

#[test]
fn opencode_command_maps_variant_and_session() {
    let task = make_task("t1", &[], "oc");
    let spec = adapter::build_cli_command(
        AdapterKind::OpenCode,
        &task,
        "code",
        ThinkingLevel::Max,
        Some("sid-1"),
        "opencode",
    )
    .unwrap();

    assert!(spec.args.contains(&"--variant".to_string()));
    assert!(spec.args.contains(&"max".to_string()));
    assert!(spec.args.contains(&"--session".to_string()));
    assert!(spec.args.contains(&"sid-1".to_string()));
    assert!(spec.args.contains(&"--pure".to_string()));
}

#[test]
fn hermes_no_thinking_flag() {
    let task = make_task("t1", &[], "mock");
    let hermes_provider = Provider {
        enabled: true,
        provider: "hermes".to_string(),
        model: "tencent/hy3:free".to_string(),
        canonical_model: None,
        wrapper: "hermes".to_string(),
        key_env: None,
        base_url: None,
        base_url_env: None,
        thinking_field: None,
        quota_key: None,
        fallback_routes: Vec::new(),
    };
    let task = Task {
        provider: hermes_provider,
        ..task
    };
    let spec = adapter::build_cli_command(
        AdapterKind::Hermes,
        &task,
        "chat",
        ThinkingLevel::High,
        None,
        "hermes",
    )
    .unwrap();
    assert!(spec.args.contains(&"chat".to_string()));
    assert!(spec.args.contains(&"-q".to_string()));
    let query = spec.args.iter().position(|arg| arg == "-q").unwrap();
    assert_eq!(spec.args.get(query + 1).map(String::as_str), Some("chat"));
    assert!(!spec.args.contains(&"--variant".to_string()));
    assert!(spec.args.contains(&"--provider".to_string()));
    assert!(spec.args.contains(&"nous".to_string()));
}

#[test]
fn agy_no_session_support() {
    assert!(!AdapterKind::Agy.supports_session_reuse());
    assert!(!AdapterKind::Agy.supports_thinking());
    assert!(!AdapterKind::Hermes.supports_session_reuse());
    assert!(!AdapterKind::Hermes.supports_thinking());
    assert!(AdapterKind::Codex.supports_session_reuse());
    assert!(AdapterKind::Codex.supports_thinking());
    assert!(AdapterKind::OpenCode.supports_thinking());
    assert!(AdapterKind::Kilo.supports_thinking());
}

#[test]
fn agy_full_policy_accepts_edits() {
    let mut task = make_task("t1", &[], "mock");
    task.spec.tools_policy = "full".to_string();
    let spec = adapter::build_cli_command(
        AdapterKind::Agy,
        &task,
        "edit",
        ThinkingLevel::Auto,
        None,
        "antigravity_cli",
    )
    .unwrap();
    assert!(spec
        .args
        .windows(2)
        .any(|args| args == ["--mode", "accept-edits"]));
}

#[test]
fn agy_print_prompt_follows_session_options() {
    let mut task = make_task("t1", &[], "mock");
    task.provider.model = "Gemini 3.5 Flash (Medium)".to_string();
    let spec = adapter::build_cli_command(
        AdapterKind::Agy,
        &task,
        "complete the assigned task",
        ThinkingLevel::Auto,
        None,
        "antigravity_cli",
    )
    .unwrap();

    let print_index = spec.args.iter().position(|arg| arg == "--print").unwrap();
    let project_index = spec
        .args
        .iter()
        .position(|arg| arg == "--new-project")
        .unwrap();
    assert!(project_index < print_index);
    assert_eq!(
        spec.args.get(print_index + 1).map(String::as_str),
        Some("complete the assigned task")
    );
    let model_index = spec.args.iter().position(|arg| arg == "--model").unwrap();
    assert!(model_index < print_index);
}

#[test]
fn task_prompt_binds_the_declared_workspace() {
    let task = make_task("workspace", &[], "mock");
    let prompt = runtime::build_task_prompt(
        std::path::Path::new("."),
        std::path::Path::new("C:\\workspaces\\target"),
        &task,
        std::slice::from_ref(&task),
        &HashMap::new(),
    );
    assert!(prompt.contains("WORKSPACE BOUNDARY: C:\\workspaces\\target"));
    assert!(prompt.contains("Do not write or create artifacts outside it"));
}

// ---------------------------------------------------------------------------
// 3. Session ID parsing
// ---------------------------------------------------------------------------

#[test]
fn codex_thread_id_from_jsonl() {
    let output = r#"{"type":"start"}
{"type":"thread","thread_id":"abc-123-def"}
{"type":"message","content":"hello"}"#;
    let id = adapter::parse_session_id(AdapterKind::Codex, output);
    assert_eq!(id.as_deref(), Some("abc-123-def"));
}

#[test]
fn codex_nested_thread_id() {
    let output = r#"{"meta":{"session":{"thread_id":"deep-id"}}}"#;
    let id = adapter::parse_session_id(AdapterKind::Codex, output);
    assert_eq!(id.as_deref(), Some("deep-id"));
}

#[test]
fn opencode_session_from_single_json() {
    let output = r#"{"session":"oc-sess-1","output":"done"}"#;
    let id = adapter::parse_session_id(AdapterKind::OpenCode, output);
    assert_eq!(id.as_deref(), Some("oc-sess-1"));
}

#[test]
fn opencode_session_from_jsonl() {
    let output = "{\"type\":\"step_start\",\"id\":\"part-not-session\"}\n{\"type\":\"step_finish\",\"sessionID\":\"jl-42\"}";
    let id = adapter::parse_session_id(AdapterKind::OpenCode, output);
    assert_eq!(id.as_deref(), Some("jl-42"));
}

#[test]
fn opencode_usage_includes_cache_tokens() {
    let output = r#"{"type":"step_finish","part":{"tokens":{"input":100,"output":7,"reasoning":3,"cache":{"read":80,"write":2}}}}"#;
    let usage = adapter::parse_cli_usage(AdapterKind::OpenCode, output);
    assert_eq!(usage.input, "100");
    assert_eq!(usage.cache_read, "80");
    assert_eq!(usage.cache_write, "2");
    assert_eq!(usage.output, "7");
    assert_eq!(usage.reasoning, "3");
}

#[test]
fn no_session_id_from_garbage() {
    assert_eq!(
        adapter::parse_session_id(AdapterKind::Codex, "not json"),
        None
    );
    assert_eq!(adapter::parse_session_id(AdapterKind::Mock, "{}"), None);
}

// ---------------------------------------------------------------------------
// 4. Session decisions and persistence
// ---------------------------------------------------------------------------

fn temp_dir() -> std::path::PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_TEMP_DIR: AtomicU64 = AtomicU64::new(0);
    let dir = std::env::temp_dir().join(format!(
        "swarms-test-{}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos(),
        NEXT_TEMP_DIR.fetch_add(1, Ordering::Relaxed),
    ));
    fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn session_decision_disabled_skips() {
    let dir = temp_dir();
    let store = SessionStore::open(&dir).unwrap();
    let cfg = SessionConfig::default();
    let d = session::decide(&cfg, &store, "mock", "m", "mock", "/").unwrap();
    assert!(matches!(d, SessionDecision::Skip));
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn session_decision_new_without_existing() {
    let dir = temp_dir();
    let store = SessionStore::open(&dir).unwrap();
    let cfg = SessionConfig {
        mode: SessionMode::Reuse,
        key: Some("k1".to_string()),
        on_missing: Default::default(),
    };
    let d = session::decide(&cfg, &store, "mock", "m", "mock", "/").unwrap();
    assert!(matches!(d, SessionDecision::New));
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn session_decision_reuse_after_put() {
    let dir = temp_dir();
    let store = SessionStore::open(&dir).unwrap();
    store
        .put(session::SessionEntry {
            key: "k1".to_string(),
            provider_session_id: "pid-99".to_string(),
            route: "mock".to_string(),
            model: "m".to_string(),
            adapter: "mock".to_string(),
            workspace: "/".to_string(),
            created_at: "t".to_string(),
            reused_count: 0,
        })
        .unwrap();
    let cfg = SessionConfig {
        mode: SessionMode::Reuse,
        key: Some("k1".to_string()),
        on_missing: Default::default(),
    };
    let d = session::decide(&cfg, &store, "mock", "m", "mock", "/").unwrap();
    assert!(matches!(d, SessionDecision::Reuse(ref id) if id == "pid-99"));
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn session_decision_mismatch_fails() {
    let dir = temp_dir();
    let store = SessionStore::open(&dir).unwrap();
    store
        .put(session::SessionEntry {
            key: "k1".to_string(),
            provider_session_id: "pid".to_string(),
            route: "codex".to_string(),
            model: "x".to_string(),
            adapter: "codex".to_string(),
            workspace: "/".to_string(),
            created_at: "t".to_string(),
            reused_count: 0,
        })
        .unwrap();
    let cfg = SessionConfig {
        mode: SessionMode::Reuse,
        key: Some("k1".to_string()),
        on_missing: Default::default(),
    };
    let result = session::decide(&cfg, &store, "mock", "m", "mock", "/");
    assert!(result.is_err());
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn session_store_survives_reopen() {
    let dir = temp_dir();
    {
        let store = SessionStore::open(&dir).unwrap();
        store
            .put(session::SessionEntry {
                key: "persist-test".to_string(),
                provider_session_id: "survived".to_string(),
                route: "mock".to_string(),
                model: "m".to_string(),
                adapter: "mock".to_string(),
                workspace: "/".to_string(),
                created_at: "t".to_string(),
                reused_count: 0,
            })
            .unwrap();
    }
    let store2 = SessionStore::open(&dir).unwrap();
    assert_eq!(
        store2.get("persist-test").unwrap().provider_session_id,
        "survived"
    );
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn timestamps_are_real_utc_iso8601() {
    assert_eq!(crate::session::iso_from_epoch(0), "1970-01-01T00:00:00Z");
    assert_eq!(
        crate::session::iso_from_epoch(1_784_237_748),
        "2026-07-16T21:35:48Z"
    );
}

// ---------------------------------------------------------------------------
// 5. Mock adapter E2E
// ---------------------------------------------------------------------------

#[test]
fn mock_writes_reshard_files() {
    let dir = temp_dir();
    let prompt = "Create docs/bench_notes/reshard_plan.md with edge cases.";
    let out = adapter::execute_mock(&dir, prompt).unwrap();
    assert!(!out.stdout.is_empty());
    assert!(dir.join("docs/bench_notes/reshard_plan.md").exists());
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn mock_writes_compress_py() {
    let dir = temp_dir();
    let prompt = "Implement bench_apps/reshard/compress.py with sharding logic.";
    adapter::execute_mock(&dir, prompt).unwrap();
    assert!(dir.join("bench_apps/reshard/compress.py").exists());
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn mock_returns_error_on_no_match() {
    let dir = temp_dir();
    let result = adapter::execute_mock(&dir, "nothing matches here");
    assert!(result.is_err());
    fs::remove_dir_all(&dir).ok();
}

// ---------------------------------------------------------------------------
// 6. Verify failure
// ---------------------------------------------------------------------------

#[test]
fn verify_failure_blocks_completion() {
    let dir = temp_dir();
    let failing_verify = if cfg!(windows) { "exit /b 1" } else { "false" };

    // Minimal plan JSON with a mock task that declares a failing verify.
    let plan_json = json!({
        "schema_version": 1,
        "goal": "test verify failure",
        "stages": [{
            "name": "S",
            "tasks": [{
                "id": "t",
                "route": "mock",
                "role": "verifier",
                "task": "Implement bench_apps/reshard/compress.py so verify runs.",
                "artifacts": ["bench_apps/reshard/compress.py"],
                "verify": [failing_verify]
            }]
        }]
    });

    let plan_path = dir.join("plan.json");
    fs::write(&plan_path, plan_json.to_string()).unwrap();

    // Write the mock router config
    let cfg_path = dir.join("config/swarm_router.json");
    fs::create_dir_all(cfg_path.parent().unwrap()).unwrap();
    fs::write(&cfg_path, json!({"providers":{"mock":{"enabled":true,"provider":"mock","model":"mock-worker","wrapper":"mock"}}}).to_string()).unwrap();

    let plan = crate::config::load_plan(&plan_path).unwrap();
    let router = crate::config::load_router(&dir).unwrap();
    let tasks = crate::config::build_tasks(&plan, &router).unwrap();

    let caps = HashMap::from([("mock".to_string(), 1)]);
    let report = runtime::execute(
        &dir,
        &dir,
        &tasks,
        &plan,
        &router,
        1,
        &caps,
        "verify-test",
        true,
        false,
    )
    .unwrap();

    assert_eq!(report.status, "failed");
    let task_state = &report.results[0];
    assert_eq!(task_state.status, TaskStatus::Failed);
    assert_eq!(task_state.verified, Some(false));
    assert!(task_state.heartbeat_unix_ms.is_some());

    fs::remove_dir_all(&dir).ok();
}

#[test]
fn quoted_verify_command_survives_platform_shell_parsing() {
    let dir = temp_dir();
    let command = if cfg!(windows) {
        r#"if "a"=="a" (exit /b 0) else (exit /b 1)"#
    } else {
        r#"test "a" = "a""#
    };
    runtime::execute_shell(command, &dir, &dir.join("verify.log")).unwrap();
    fs::remove_dir_all(dir).ok();
}

#[test]
fn dependency_output_truncation_preserves_utf8_boundaries() {
    let dir = temp_dir();
    let dependency = make_task("dep", &[], "mock");
    let consumer = make_task("consumer", &["dep"], "mock");
    let log_dir = dir.join("results").join(&dependency.id);
    fs::create_dir_all(&log_dir).unwrap();
    fs::write(log_dir.join("worker.log"), "🙂".repeat(5_000)).unwrap();
    let mut state = TaskState::new(
        &dependency.id,
        &dependency.source_id,
        &dependency.stage,
        "mock",
    );
    state.status = TaskStatus::Completed;
    let states = HashMap::from([(dependency.id.clone(), state)]);
    let context =
        runtime::dependency_outputs(&dir, &consumer, &[dependency, consumer.clone()], &states);
    assert!(context.contains('🙂'));
    assert!(context.is_char_boundary(context.len()));
    fs::remove_dir_all(dir).ok();
}

// ---------------------------------------------------------------------------
// 7. Resume preserves completed tasks
// ---------------------------------------------------------------------------

#[test]
fn resume_keeps_completed_tasks() {
    let dir = temp_dir();

    let plan_json = json!({
        "schema_version": 1,
        "goal": "test resume",
        "stages": [{
            "name": "S",
            "tasks": [{
                "id": "t",
                "route": "mock",
                "role": "general",
                "task": "Implement bench_apps/reshard/compress.py now.",
                "artifacts": ["bench_apps/reshard/compress.py"]
            }]
        }]
    });
    let plan_path = dir.join("plan.json");
    fs::write(&plan_path, plan_json.to_string()).unwrap();
    let cfg_path = dir.join("config/swarm_router.json");
    fs::create_dir_all(cfg_path.parent().unwrap()).unwrap();
    fs::write(
        &cfg_path,
        json!({"providers":{"mock":{"enabled":true,"provider":"mock","model":"mock-worker","wrapper":"mock"}}}).to_string(),
    )
    .unwrap();

    let plan = crate::config::load_plan(&plan_path).unwrap();
    let router = crate::config::load_router(&dir).unwrap();
    let tasks = crate::config::build_tasks(&plan, &router).unwrap();
    let caps = HashMap::from([("mock".to_string(), 1)]);

    // First run
    let _r1 = runtime::execute(
        &dir,
        &dir,
        &tasks,
        &plan,
        &router,
        1,
        &caps,
        "resume-test",
        true,
        false,
    )
    .unwrap();

    // Explicit resume keeps a matching completed checkpoint.
    let r2 = runtime::execute(
        &dir,
        &dir,
        &tasks,
        &plan,
        &router,
        1,
        &caps,
        "resume-test",
        false,
        true,
    )
    .unwrap();

    assert_eq!(r2.status, "completed");
    assert_eq!(r2.results[0].status, TaskStatus::Completed);
    let original_checkpoint = r2.results[0].checkpoint_key.clone();

    let accidental = runtime::execute(
        &dir,
        &dir,
        &tasks,
        &plan,
        &router,
        1,
        &caps,
        "resume-test",
        false,
        false,
    )
    .unwrap_err();
    assert!(accidental.contains("use --resume or --force"));

    let mut changed_json = plan_json;
    changed_json["stages"][0]["tasks"][0]["task"] =
        json!("Implement bench_apps/reshard/compress.py now. Changed definition.");
    fs::write(&plan_path, changed_json.to_string()).unwrap();
    let changed_plan = crate::config::load_plan(&plan_path).unwrap();
    let changed_tasks = crate::config::build_tasks(&changed_plan, &router).unwrap();
    let changed = runtime::execute(
        &dir,
        &dir,
        &changed_tasks,
        &changed_plan,
        &router,
        1,
        &caps,
        "resume-test",
        false,
        true,
    )
    .unwrap();
    assert_ne!(changed.results[0].checkpoint_key, original_checkpoint);

    fs::remove_dir_all(&dir).ok();
}

#[test]
fn resume_requires_existing_run() {
    let dir = temp_dir();
    let plan = serde_json::from_value::<model::Plan>(json!({
        "stages":[{"name":"S","tasks":[{"id":"t","route":"mock","task":"x"}]}]
    }))
    .unwrap();
    let router = mock_router();
    let tasks = crate::config::build_tasks(&plan, &router).unwrap();
    let error = runtime::execute(
        &dir,
        &dir,
        &tasks,
        &plan,
        &router,
        1,
        &HashMap::from([("mock".to_string(), 1)]),
        "missing",
        false,
        true,
    )
    .unwrap_err();
    assert!(error.contains("cannot resume missing run"));
    fs::remove_dir_all(dir).ok();
}

// ---------------------------------------------------------------------------
// 8. URL validation
// ---------------------------------------------------------------------------

#[test]
fn url_validation_rejects_non_https() {
    assert!(adapter::validate_url("https://api.example.com/v1").is_ok());
    assert!(adapter::validate_url("http://localhost:8080/v1").is_ok());
    assert!(adapter::validate_url("http://127.0.0.1/v1").is_ok());
    assert!(adapter::validate_url("http://evil.com/v1").is_err());
    assert!(adapter::validate_url("https://user:pass@api.com/v1").is_err());
    assert!(adapter::validate_url("ftp://api.com").is_err());
}

// ---------------------------------------------------------------------------
// 9. Run ID safety
// ---------------------------------------------------------------------------

#[test]
fn run_id_safety() {
    assert!(safe_run_id("abc-123_def.4"));
    assert!(!safe_run_id("../escape"));
    assert!(!safe_run_id(""));
    assert!(!safe_run_id("has spaces"));
}

// ---------------------------------------------------------------------------
// 10. Prompt generation
// ---------------------------------------------------------------------------

#[test]
fn prompt_has_stable_prefix() {
    let p = adapter::build_prompt("coder", "do something", &[], "");
    assert!(p.starts_with(PROMPT_PREFIX));
    assert!(p.contains("Ponytail/full"));
    assert!(p.contains("prefer CodeGraph"));
    assert!(p.contains("Role: coder"));
    assert!(p.contains("Task: do something"));
}

#[test]
fn prompt_prefix_is_shared_across_roles() {
    for role in ["planner", "critic", "programmer", "verifier"] {
        let prompt = adapter::build_prompt(role, "work", &[], "");
        assert!(prompt.starts_with(PROMPT_PREFIX));
        assert!(prompt.contains("Ponytail/full"));
        assert!(prompt.contains("prefer CodeGraph"));
    }
}

#[test]
fn prompt_dependency_context_after_prefix() {
    let dep = "Dependency 0001-x output:\nhello";
    let p = adapter::build_prompt("coder", "task text", &["file.py".to_string()], dep);
    assert!(p.contains(dep));
    assert!(p.contains("Allowed artifacts: file.py"));
}

#[test]
fn prompt_sanitizes_nul_from_dependency_transport() {
    let prompt = adapter::build_prompt(
        "programmer",
        "build the lane",
        &["core/pbir_logic.py".to_string()],
        "Dependency output\0must not reach a CLI argument",
    );

    assert!(!prompt.contains('\0'));
    assert!(prompt.contains("Dependency outputmust not reach a CLI argument"));
}

// ---------------------------------------------------------------------------
// 11. Artifact path validation
// ---------------------------------------------------------------------------

#[test]
fn artifact_path_validation() {
    assert!(validate_artifact_path("src/main.rs").is_ok());
    assert!(validate_artifact_path("docs/bench_notes/plan.md").is_ok());
    assert!(validate_artifact_path("/etc/passwd").is_err());
    assert!(validate_artifact_path("../escape").is_err());
    assert!(validate_artifact_path("C:\\Windows").is_err());
    assert!(validate_artifact_path("").is_err());
}

// ---------------------------------------------------------------------------
// 12. find_dependency_task
// ---------------------------------------------------------------------------

#[test]
fn find_dep_by_source_id() {
    let tasks = vec![make_task("alpha", &[], "mock")];
    assert!(find_dependency_task(&tasks, "alpha").is_some());
    assert!(find_dependency_task(&tasks, "nonexistent").is_none());
}

// ---------------------------------------------------------------------------
// 13. Review catches unsupported thinking
// ---------------------------------------------------------------------------

#[test]
fn review_rejects_thinking_on_hermes() {
    let hermes_provider = Provider {
        enabled: true,
        provider: "hermes".to_string(),
        model: "tencent/hy3:free".to_string(),
        canonical_model: None,
        wrapper: "hermes".to_string(),
        key_env: None,
        base_url: None,
        base_url_env: None,
        thinking_field: None,
        quota_key: None,
        fallback_routes: Vec::new(),
    };
    let task = Task {
        provider: hermes_provider.clone(),
        spec: TaskSpec {
            id: "t".to_string(),
            route: "hermes".to_string(),
            task: "x".to_string(),
            role: "general".to_string(),
            needs: Vec::new(),
            tools_policy: "none".to_string(),
            artifacts: Vec::new(),
            verify: Vec::new(),
            thinking: Some(ThinkingLevel::High),
            session: None,
            timeout_seconds: None,
            max_attempts: None,
        },
        id: model::task_index_to_id(0, "t"),
        source_id: "t".to_string(),
        stage: "S".to_string(),
        stage_parallel: true,
        effective_route: "hermes".to_string(),
    };

    let plan = serde_json::from_str::<model::Plan>(
        &json!({
            "schema_version": 1,
            "goal": "x",
            "stages": [{"name":"S","tasks":[{"id":"t","route":"hermes","task":"x","thinking":"high"}]}]
        })
        .to_string(),
    )
    .unwrap();
    let mut providers = HashMap::new();
    providers.insert("hermes".to_string(), hermes_provider);
    let router = Router {
        fallback_route: None,
        aliases: HashMap::new(),
        role_routes: HashMap::new(),
        quota_policy: QuotaPolicy::default(),
        providers,
    };
    let tasks = vec![task];
    let result = review_plan(&plan, &router, &tasks);
    assert!(
        result
            .findings
            .iter()
            .any(|f| { f.severity == Severity::Error && f.code == "thinking_not_supported" }),
        "expected thinking_not_supported error for hermes"
    );
}

// ---------------------------------------------------------------------------
// 14. Quota-aware route selection
// ---------------------------------------------------------------------------

#[test]
fn quota_guard_spills_to_fallback_and_records_effective_route() {
    let dir = temp_dir();
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    fs::write(
        dir.join("quota.json"),
        json!({
            "generated_at_epoch": now,
            "quotas": {"codex:Codex": {"windows": {"5h": 4.0, "7d": 80.0}}}
        })
        .to_string(),
    )
    .unwrap();
    let plan_path = dir.join("plan.json");
    fs::write(
        &plan_path,
        json!({"stages":[{"name":"S","tasks":[{
            "id":"t","route":"primary","task":"Implement bench_apps/reshard/compress.py now."
        }]}]})
        .to_string(),
    )
    .unwrap();
    let cfg_path = dir.join("config/swarm_router.json");
    fs::create_dir_all(cfg_path.parent().unwrap()).unwrap();
    fs::write(
        &cfg_path,
        json!({
            "quota_policy": {
                "enabled": true,
                "snapshot_path": "quota.json",
                "min_remaining_percent": 10.0,
                "max_age_seconds": 60,
                "on_unknown": "block"
            },
            "providers": {
                "primary": {"enabled":true,"provider":"mock","model":"mock-worker","wrapper":"mock","quota_key":"codex:Codex","fallback_routes":["backup"]},
                "backup": {"enabled":true,"provider":"mock","model":"mock-worker","wrapper":"mock"}
            }
        })
        .to_string(),
    )
    .unwrap();
    let plan = crate::config::load_plan(&plan_path).unwrap();
    let router = crate::config::load_router(&dir).unwrap();
    let tasks = crate::config::build_tasks(&plan, &router).unwrap();
    let caps = HashMap::from([("primary".to_string(), 1), ("backup".to_string(), 1)]);
    let report = runtime::execute(
        &dir,
        &dir,
        &tasks,
        &plan,
        &router,
        1,
        &caps,
        "quota-fallback",
        true,
        false,
    )
    .unwrap();
    assert_eq!(report.status, "completed");
    assert_eq!(report.results[0].route, "primary");
    assert_eq!(report.results[0].effective_route, "backup");
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn quota_guard_keeps_accounts_and_known_windows_separate() {
    let dir = temp_dir();
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    fs::write(
        dir.join("quota.json"),
        json!({
            "generated_at_epoch": now,
            "quotas": {
                "codex:Codex": {"windows": {"5h": 50.0}},
                "codex:Hermes": {"windows": {"5h": 2.0}}
            }
        })
        .to_string(),
    )
    .unwrap();
    let policy = QuotaPolicy {
        enabled: true,
        snapshot_path: dir.join("quota.json").to_string_lossy().to_string(),
        min_remaining_percent: 10.0,
        max_age_seconds: 60,
        on_unknown: OnUnknownQuota::Block,
    };
    let guard = crate::quota::QuotaGuard::load(&dir, &policy);
    let mut codex = mock_provider();
    codex.quota_key = Some("codex:Codex".to_string());
    let mut hermes = mock_provider();
    hermes.quota_key = Some("codex:Hermes".to_string());
    assert!(guard.check(&codex).is_ok(), "missing 7d is not zero");
    assert!(guard.check(&hermes).unwrap_err().contains("2.0%"));
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn quota_snapshot_view_is_sorted_and_contains_only_normalized_windows() {
    let dir = temp_dir();
    let path = dir.join("quota.json");
    fs::write(
        &path,
        json!({
            "generated_at_epoch": 123,
            "quotas": {
                "zai:coding": {"windows": {"5h": 90.0}, "secret": "ignored"},
                "codex:Codex": {"windows": {"7d": 80.0, "5h": 40.0}}
            }
        })
        .to_string(),
    )
    .unwrap();
    let view = crate::quota::load_snapshot_view(&path).unwrap();
    assert_eq!(view.generated_at_epoch, 123);
    assert_eq!(view.entries[0].key, "codex:Codex");
    assert_eq!(view.entries[0].windows["5h"], 40.0);
    assert_eq!(view.entries[1].key, "zai:coding");
    fs::remove_dir_all(dir).ok();
}

#[test]
fn queued_steer_is_applied_before_mock_task_finishes() {
    let dir = temp_dir();
    let run_dir = dir.join(".agent/swarm/runs/steer-test");
    let plan = serde_json::from_value::<model::Plan>(json!({
        "stages": [{"name": "Build", "tasks": [{
            "id": "worker",
            "route": "mock",
            "task": "Create docs/bench_notes/reshard_plan.md with edge cases."
        }]}]
    }))
    .unwrap();
    let router = mock_router();
    let task = crate::config::build_tasks(&plan, &router)
        .unwrap()
        .remove(0);
    fs::create_dir_all(run_dir.join("results").join(&task.id)).unwrap();
    crate::steering::enqueue(
        &run_dir,
        &task.id,
        "Keep the result concise and deterministic.",
        "test",
    )
    .unwrap();
    let store = crate::session::SessionStore::open(&run_dir).unwrap();
    let state = runtime::run_task(
        &dir,
        &run_dir,
        &task,
        &plan,
        "Create docs/bench_notes/reshard_plan.md with edge cases.",
        &store,
    );
    assert_eq!(state.status, TaskStatus::Completed);
    let history = crate::steering::history(&run_dir, &task.id);
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].status, "applied");
    let log =
        fs::read_to_string(run_dir.join("results").join(&task.id).join("worker.log")).unwrap();
    assert!(log.contains("user steer"));
    fs::remove_dir_all(dir).ok();
}

#[test]
fn quota_guard_rejects_stale_snapshot_when_unknown_blocks() {
    let dir = temp_dir();
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    fs::write(
        dir.join("quota.json"),
        json!({"generated_at_epoch": now - 100, "quotas": {}}).to_string(),
    )
    .unwrap();
    let policy = QuotaPolicy {
        enabled: true,
        snapshot_path: dir.join("quota.json").to_string_lossy().to_string(),
        min_remaining_percent: 10.0,
        max_age_seconds: 10,
        on_unknown: OnUnknownQuota::Block,
    };
    let guard = crate::quota::QuotaGuard::load(&dir, &policy);
    let mut provider = mock_provider();
    provider.quota_key = Some("codex:Codex".to_string());
    assert!(guard.check(&provider).unwrap_err().contains("stale"));
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn quota_guard_rejects_snapshot_too_far_in_future() {
    let dir = temp_dir();
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    fs::write(
        dir.join("quota.json"),
        json!({"generated_at_epoch": now + 100, "quotas": {}}).to_string(),
    )
    .unwrap();
    let policy = QuotaPolicy {
        enabled: true,
        snapshot_path: dir.join("quota.json").to_string_lossy().to_string(),
        min_remaining_percent: 10.0,
        max_age_seconds: 10,
        on_unknown: OnUnknownQuota::Block,
    };
    let guard = crate::quota::QuotaGuard::load(&dir, &policy);
    let mut provider = mock_provider();
    provider.quota_key = Some("codex:Codex".to_string());
    assert!(guard.check(&provider).unwrap_err().contains("future"));
    fs::remove_dir_all(dir).ok();
}

#[test]
fn router_override_and_alias_caps_are_applied() {
    let dir = temp_dir();
    let base = dir.join("alternate.json");
    fs::write(
        &base,
        json!({
            "aliases": {"cheap": "mock"},
            "providers": {"mock": {"enabled": true, "provider": "mock", "model": "alternate", "wrapper": "mock"}}
        })
        .to_string(),
    )
    .unwrap();
    let router = crate::config::load_router_from_path(&dir, &base).unwrap();
    assert_eq!(router.providers["mock"].model, "alternate");
    let plan = serde_json::from_value::<model::Plan>(json!({
        "budget_policy": {"provider_concurrency": {"cheap": 2}},
        "stages":[{"tasks":[{"id":"t","route":"cheap","task":"x"}]}]
    }))
    .unwrap();
    let caps =
        crate::config::effective_caps(&plan, &HashMap::from([("cheap".to_string(), 3)]), &router);
    assert_eq!(caps.get("mock"), Some(&3));
    assert!(!caps.contains_key("cheap"));
    fs::remove_dir_all(dir).ok();
}

#[test]
fn workflow_metadata_persists_explicit_and_default_projects() {
    let dir = temp_dir();
    let router = mock_router();
    let explicit = serde_json::from_value::<model::Plan>(json!({
        "goal": "project metadata",
        "project": {"id": "swarms-core", "name": "SWARMS Core"},
        "stages":[{"tasks":[{"id":"t","route":"mock","task":"x"}]}]
    }))
    .unwrap();
    let tasks = crate::config::build_tasks(&explicit, &router).unwrap();
    let run_dir = dir.join("explicit");
    runtime::dry_run(
        &run_dir,
        &dir,
        "explicit",
        &tasks,
        &explicit,
        1,
        &HashMap::new(),
    )
    .unwrap();
    let workflow: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(run_dir.join("workflow.json")).unwrap()).unwrap();
    assert_eq!(workflow["project_id"], "swarms-core");
    assert_eq!(workflow["project_name"], "SWARMS Core");

    let implicit = serde_json::from_value::<model::Plan>(json!({
        "goal": "default project",
        "stages":[{"tasks":[{"id":"t","route":"mock","task":"x"}]}]
    }))
    .unwrap();
    assert!(implicit.project.is_none());
    let implicit_tasks = crate::config::build_tasks(&implicit, &router).unwrap();
    let implicit_run = dir.join("implicit");
    runtime::dry_run(
        &implicit_run,
        &dir,
        "implicit",
        &implicit_tasks,
        &implicit,
        1,
        &HashMap::new(),
    )
    .unwrap();
    let workflow: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(implicit_run.join("workflow.json")).unwrap())
            .unwrap();
    assert!(workflow["project_id"]
        .as_str()
        .unwrap()
        .starts_with("workspace:"));
    assert_eq!(
        workflow["project_name"],
        dir.file_name().unwrap().to_string_lossy().as_ref()
    );
    fs::remove_dir_all(dir).ok();
}

#[test]
fn review_rejects_unsafe_project_identifier() {
    let plan = serde_json::from_value::<model::Plan>(json!({
        "goal": "bad project",
        "project": {"id": "../escape", "name": "Bad"},
        "stages":[{"tasks":[{"id":"t","route":"mock","task":"x"}]}]
    }))
    .unwrap();
    let router = mock_router();
    let tasks = crate::config::build_tasks(&plan, &router).unwrap();
    let result = review_plan(&plan, &router, &tasks);
    assert!(result
        .findings
        .iter()
        .any(|finding| finding.code == "invalid_project_id"));
}

#[test]
fn non_parallel_stage_selects_one_ready_task() {
    let plan = serde_json::from_value::<model::Plan>(json!({
        "stages":[{"name":"serial","parallel":false,"tasks":[
            {"id":"a","route":"mock","task":"a"},
            {"id":"b","route":"mock","task":"b"}
        ]}]
    }))
    .unwrap();
    let router = mock_router();
    let tasks = crate::config::build_tasks(&plan, &router).unwrap();
    let states = tasks
        .iter()
        .map(|task| {
            (
                task.id.clone(),
                crate::telemetry::TaskState::new(
                    &task.id,
                    &task.source_id,
                    &task.stage,
                    &task.spec.route,
                ),
            )
        })
        .collect();
    let quotas = crate::quota::QuotaGuard::load(std::path::Path::new("."), &router.quota_policy);
    let ready = runtime::find_ready(
        &tasks,
        &states,
        2,
        &HashMap::from([("mock".to_string(), 2)]),
        &plan,
        &router,
        &quotas,
    );
    assert_eq!(ready.selected.len(), 1);
}

#[test]
fn parallel_stage_spills_across_route_caps() {
    let plan = serde_json::from_value::<model::Plan>(json!({
        "stages":[{"name":"parallel","parallel":true,"tasks":[
            {"id":"a","route":"primary","task":"a"},
            {"id":"b","route":"primary","task":"b"}
        ]}]
    }))
    .unwrap();
    let mut primary = mock_provider();
    primary.fallback_routes = vec!["backup".to_string()];
    let router = Router {
        fallback_route: None,
        aliases: HashMap::new(),
        role_routes: HashMap::new(),
        quota_policy: QuotaPolicy::default(),
        providers: HashMap::from([
            ("primary".to_string(), primary),
            ("backup".to_string(), mock_provider()),
        ]),
    };
    let tasks = crate::config::build_tasks(&plan, &router).unwrap();
    let states = tasks
        .iter()
        .map(|task| {
            (
                task.id.clone(),
                crate::telemetry::TaskState::new(
                    &task.id,
                    &task.source_id,
                    &task.stage,
                    &task.spec.route,
                ),
            )
        })
        .collect();
    let quotas = crate::quota::QuotaGuard::load(std::path::Path::new("."), &router.quota_policy);
    let ready = runtime::find_ready(
        &tasks,
        &states,
        2,
        &HashMap::from([("primary".to_string(), 1), ("backup".to_string(), 1)]),
        &plan,
        &router,
        &quotas,
    );
    assert_eq!(ready.selected.len(), 2);
    assert_eq!(ready.selected[0].effective_route, "primary");
    assert_eq!(ready.selected[1].effective_route, "backup");
}

// ---------------------------------------------------------------------------
// 15. check_artifacts
// ---------------------------------------------------------------------------

#[test]
fn artifacts_must_exist() {
    let dir = temp_dir();
    let mut task = make_task("t", &[], "mock");
    task.spec.artifacts = vec!["nonexistent_file.xyz".to_string()];
    assert!(check_artifacts(&dir, &task).is_err());

    let file = dir.join("exists.txt");
    fs::write(&file, "ok").unwrap();
    task.spec.artifacts = vec!["exists.txt".to_string()];
    assert!(check_artifacts(&dir, &task).is_ok());

    fs::remove_dir_all(&dir).ok();
}
