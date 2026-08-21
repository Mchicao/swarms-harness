//! Adaptive parallel test-time scaling.
//!
//! A scaled task runs several candidate rollouts in isolated git worktrees,
//! ranks them with deterministic `verify` commands first, then an optional LLM
//! verifier, and finally escalates to a stronger model (select / review /
//! synthesize) only when the cheap rollouts remain ambiguous. The winner's
//! diff is applied back to the workspace and the standard artifact + verify
//! gates run exactly once against the real root.
//!
//! Deliberate ceilings (ponytail):
//! - Wave size is clamped by this task's route cap and the global cap, but
//!   rollouts of concurrent tasks cannot see each other; slight
//!   oversubscription across tasks is possible. Upgrade path: a shared permit
//!   registry in `execute()`.
//! - Root dirty state (tracked diff + non-ignored untracked files) is copied
//!   into every candidate worktree; very large untracked trees pay a copy per
//!   candidate. Upgrade path: worktree-local overlay FS.
//! - The verifier compares all candidates in one prompt (O(1) calls, N<=8);
//!   for larger N switch to a pivot tournament (O(Nk)) as in llm-as-a-verifier.

use crate::model::{EscalateAction, Plan, Router, ScalingMode, ScalingPolicy, Task, ThinkingLevel};
use crate::runtime::{
    append_event, check_artifacts_with_snapshot, execute_adapter, failed_state, merge_usage,
    run_verify_commands, success_state,
};
use crate::telemetry::{
    EscalationInfo, RolloutInfo, ScalingOutcome, TaskState, Usage, VerifierInfo,
};
use serde_json::json;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread;
use std::time::Instant;

type Result<T> = std::result::Result<T, String>;

const CANDIDATE_EXCERPT_CHARS: usize = 3_000;

// ---------------------------------------------------------------------------
// Candidate bookkeeping
// ---------------------------------------------------------------------------

struct Candidate {
    index: usize,
    route: String,
    model: String,
    worktree: PathBuf,
    dir: PathBuf,
    ok: bool,
    error: Option<String>,
    verified: Option<bool>,
    verify_error: Option<String>,
    score: Option<f64>,
    duration_ms: u128,
    usage: Usage,
}

impl Candidate {
    fn passed(&self) -> bool {
        self.verified == Some(true)
    }
}

struct DirtyState {
    patch: String,
    untracked: Vec<String>,
}

// ---------------------------------------------------------------------------
// Git helpers
// ---------------------------------------------------------------------------

fn git(cwd: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .map_err(|e| format!("spawn git: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "git {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

fn git_apply(cwd: &Path, patch_path: &Path) -> Result<()> {
    git(
        cwd,
        &[
            "apply",
            "--whitespace=nowarn",
            &patch_path.to_string_lossy(),
        ],
    )
    .map(|_| ())
}

fn capture_dirty(root: &Path) -> Result<DirtyState> {
    let patch = git(root, &["diff", "--binary", "HEAD"])?;
    let untracked_raw = git(root, &["ls-files", "--others", "--exclude-standard", "-z"])?;
    let untracked = untracked_raw
        .split('\0')
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect();
    Ok(DirtyState { patch, untracked })
}

fn apply_dirty(
    source_root: &Path,
    worktree: &Path,
    patch_path: &Path,
    dirty: &DirtyState,
) -> Result<()> {
    if !dirty.patch.trim().is_empty() {
        fs::write(patch_path, &dirty.patch).map_err(|e| e.to_string())?;
        git_apply(worktree, patch_path)?;
    }
    for rel in &dirty.untracked {
        let src = source_root.join(rel);
        let dst = worktree.join(rel);
        if let Some(parent) = dst.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("mkdir {}: {e}", parent.display()))?;
        }
        fs::copy(&src, &dst).map_err(|e| format!("copy {rel}: {e}"))?;
    }
    Ok(())
}

fn add_worktree(root: &Path, path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("mkdir {}: {e}", parent.display()))?;
    }
    git(
        root,
        &[
            "worktree",
            "add",
            "--detach",
            &path.to_string_lossy(),
            "HEAD",
        ],
    )
    .map(|_| ())
}

fn remove_worktree(root: &Path, path: &Path) {
    let _ = git(
        root,
        &["worktree", "remove", "--force", &path.to_string_lossy()],
    );
}

fn untracked_files(worktree: &Path) -> Vec<String> {
    git(
        worktree,
        &["ls-files", "--others", "--exclude-standard", "-z"],
    )
    .map(|raw| {
        raw.split('\0')
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect()
    })
    .unwrap_or_default()
}

fn worktree_diff(worktree: &Path) -> String {
    git(worktree, &["diff", "HEAD"]).unwrap_or_default()
}

/// Apply the winner candidate's changes (tracked diff + new files) to root.
fn apply_winner(root: &Path, winner: &Candidate) -> Result<()> {
    let diff = git(&winner.worktree, &["diff", "--binary", "HEAD"])?;
    if !diff.trim().is_empty() {
        let patch_path = winner.dir.join("winner.patch");
        fs::write(&patch_path, &diff).map_err(|e| e.to_string())?;
        git_apply(root, &patch_path)?;
    }
    for rel in untracked_files(&winner.worktree) {
        let src = winner.worktree.join(&rel);
        let dst = root.join(&rel);
        let src_bytes = fs::read(&src).map_err(|e| format!("read {rel}: {e}"))?;
        let dst_bytes = fs::read(&dst).ok();
        if dst_bytes.as_deref() != Some(src_bytes.as_slice()) {
            if let Some(parent) = dst.parent() {
                fs::create_dir_all(parent)
                    .map_err(|e| format!("mkdir {}: {e}", parent.display()))?;
            }
            fs::write(&dst, &src_bytes).map_err(|e| format!("write {rel}: {e}"))?;
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Rollouts
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn run_rollout(
    task: &Task,
    plan: &Plan,
    prompt: &str,
    thinking: ThinkingLevel,
    worktree: &Path,
    cand_dir: &Path,
    run_dir: &Path,
) -> Candidate {
    let started = Instant::now();
    let worktree = worktree.to_path_buf();
    let cand_dir = cand_dir.to_path_buf();
    let run_dir = run_dir.to_path_buf();
    let task = task.clone();
    let plan = plan.clone();
    let prompt = prompt.to_string();

    let outcome: Result<(Usage, Option<bool>, Option<String>)> =
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let exec = execute_adapter(
                &task,
                &prompt,
                thinking,
                None,
                &worktree,
                &run_dir,
                &cand_dir,
                &plan.execution,
            )?;
            let (verified, verify_error) = run_verify_commands(&task, &worktree, &cand_dir);
            Ok((exec.usage, verified, verify_error))
        }))
        .unwrap_or_else(|_| Err("rollout thread panicked".to_string()));

    let duration_ms = started.elapsed().as_millis();
    let (usage, verified, verify_error, ok, error) = match outcome {
        Ok((usage, verified, verify_error)) => (usage, verified, verify_error, true, None),
        Err(error) => (Usage::missing(), None, None, false, Some(error)),
    };

    Candidate {
        index: 0,
        route: task.effective_route.clone(),
        model: task.provider.model.clone(),
        worktree,
        dir: cand_dir,
        ok,
        error,
        verified,
        verify_error,
        score: None,
        duration_ms,
        usage,
    }
}

/// Run `count` parallel rollouts in fresh worktrees, indices `start..start+count`.
#[allow(clippy::too_many_arguments)]
fn run_wave(
    task: &Task,
    plan: &Plan,
    prompt: &str,
    thinking: ThinkingLevel,
    root: &Path,
    run_dir: &Path,
    base_dir: &Path,
    dirty: &DirtyState,
    start_index: usize,
    count: usize,
) -> Result<Vec<Candidate>> {
    let mut handles = Vec::new();
    for offset in 0..count {
        let index = start_index + offset;
        let cand_dir = base_dir.join(format!("cand-{index:02}"));
        let wt = cand_dir.join("wt");
        add_worktree(root, &wt)?;
        let patch_path = base_dir.join(format!("cand-{index:02}.patch"));
        apply_dirty(root, &wt, &patch_path, dirty)?;
        let task = task.clone();
        let plan = plan.clone();
        let prompt = prompt.to_string();
        let run_dir = run_dir.to_path_buf();
        handles.push(thread::spawn(move || {
            run_rollout(&task, &plan, &prompt, thinking, &wt, &cand_dir, &run_dir)
        }));
    }
    let mut candidates = Vec::new();
    for (offset, handle) in handles.into_iter().enumerate() {
        let fallback_dir = base_dir.join(format!("cand-{:02}", start_index + offset));
        let mut candidate = handle.join().unwrap_or_else(|_| Candidate {
            index: 0,
            route: task.effective_route.clone(),
            model: task.provider.model.clone(),
            worktree: fallback_dir.join("wt"),
            dir: fallback_dir,
            ok: false,
            error: Some("rollout thread panicked".to_string()),
            verified: None,
            verify_error: None,
            score: None,
            duration_ms: 0,
            usage: Usage::missing(),
        });
        candidate.index = start_index + offset;
        append_event(
            run_dir,
            "scaling_rollout_finished",
            json!({
                "task_id": task.id,
                "index": candidate.index,
                "route": candidate.route,
                "model": candidate.model,
                "ok": candidate.ok,
                "verified": candidate.verified,
                "duration_ms": candidate.duration_ms,
            }),
        );
        candidates.push(candidate);
    }
    Ok(candidates)
}

// ---------------------------------------------------------------------------
// Ranking
// ---------------------------------------------------------------------------

/// Deterministic ranking: verified passers first, then healthy rollouts, then
/// failures; within a class, higher verifier score wins, then lower index.
fn rank_indices(candidates: &[Candidate]) -> Vec<usize> {
    let mut order: Vec<usize> = (0..candidates.len()).collect();
    order.sort_by(|&a, &b| {
        let ca = &candidates[a];
        let cb = &candidates[b];
        cb.passed()
            .cmp(&ca.passed())
            .then(cb.ok.cmp(&ca.ok))
            .then(
                cb.score
                    .unwrap_or(0.0)
                    .partial_cmp(&ca.score.unwrap_or(0.0))
                    .unwrap_or(std::cmp::Ordering::Equal),
            )
            .then(a.cmp(&b))
    });
    order
}

fn passer_count(candidates: &[Candidate]) -> usize {
    candidates.iter().filter(|c| c.passed()).count()
}

// ---------------------------------------------------------------------------
// LLM verifier / selector
// ---------------------------------------------------------------------------

#[derive(Debug, Default, serde::Deserialize)]
struct VerifierVerdict {
    #[serde(default)]
    scores: Vec<f64>,
    #[serde(default)]
    confidence: f64,
    #[serde(default)]
    winner: Option<usize>,
}

/// Extract the first JSON object from possibly noisy CLI output.
fn parse_verifier_verdict(output: &str) -> Option<VerifierVerdict> {
    let start = output.find('{')?;
    let end = output.rfind('}')?;
    if end <= start {
        return None;
    }
    serde_json::from_str(&output[start..=end]).ok()
}

/// Clone the task onto another route (verifier or escalation model).
fn route_task(base: &Task, route: &str, router: &Router, write_access: bool) -> Result<Task> {
    let provider = router
        .get_provider(route)
        .filter(|p| p.enabled)
        .ok_or_else(|| format!("scaling route '{route}' is unknown or disabled"))?;
    let mut task = base.clone();
    task.spec.route = route.to_string();
    task.spec.tools_policy = if write_access {
        "workspace-write".to_string()
    } else {
        "read-only".to_string()
    };
    task.effective_route = router.resolve_route(route).to_string();
    task.provider = provider.clone();
    Ok(task)
}

fn tail(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    let start = text.char_indices().nth_back(max_chars - 1).map(|(i, _)| i);
    match start {
        Some(i) => text[i..].to_string(),
        None => text.to_string(),
    }
}

fn candidate_excerpt(c: &Candidate) -> String {
    let mut text = format!(
        "--- Candidate {} (route {}, model {}) ---\n",
        c.index, c.route, c.model
    );
    let diff = worktree_diff(&c.worktree);
    let new_files = untracked_files(&c.worktree);
    if diff.trim().is_empty() && new_files.is_empty() {
        text.push_str("(no workspace changes; worker output tail)\n");
        let log = fs::read_to_string(c.dir.join("worker.log")).unwrap_or_default();
        text.push_str(&tail(&log, CANDIDATE_EXCERPT_CHARS));
        return text;
    }
    text.push_str(&tail(&diff, CANDIDATE_EXCERPT_CHARS / 2));
    for rel in new_files.iter().take(4) {
        let content = fs::read_to_string(c.worktree.join(rel)).unwrap_or_default();
        text.push_str(&format!("\n[new file {rel}]\n"));
        text.push_str(&tail(&content, CANDIDATE_EXCERPT_CHARS / 4));
    }
    text
}

/// One-shot ask of an aux model (verifier or selector). Returns its raw usage.
#[allow(clippy::too_many_arguments)]
fn run_aux_model(
    aux_task: &Task,
    plan: &Plan,
    root: &Path,
    run_dir: &Path,
    work_dir: &Path,
    prompt: &str,
) -> Option<(String, Usage)> {
    let _ = fs::create_dir_all(work_dir);
    let exec = execute_adapter(
        aux_task,
        prompt,
        ThinkingLevel::default(),
        None,
        root,
        run_dir,
        work_dir,
        &plan.execution,
    )
    .ok()?;
    Some((exec.output, exec.usage))
}

fn verifier_prompt(task_text: &str, candidates: &[Candidate]) -> String {
    let mut prompt = String::from(
        "You are an LLM verifier. Compare the candidate solutions below for ONE task.\n\
         Respond with ONLY a JSON object: {\"scores\": [0.0-10.0 per candidate in order], \
         \"confidence\": 0.0-1.0, \"winner\": <best candidate index>}.\n\
         Rank by correctness, completeness, and safety of the changes.\n\n",
    );
    prompt.push_str(&format!("Task: {task_text}\n\n"));
    for c in candidates {
        prompt.push_str(&candidate_excerpt(c));
        prompt.push_str("\n\n");
    }
    prompt
}

fn selector_prompt(task_text: &str, candidates: &[Candidate]) -> String {
    let mut prompt = String::from(
        "You are a strong reviewer model. Inspect the candidates for ONE task and pick \
         the single best one.\n\
         Respond with ONLY a JSON object: {\"winner\": <candidate index>}.\n\n",
    );
    prompt.push_str(&format!("Task: {task_text}\n\n"));
    for c in candidates {
        prompt.push_str(&candidate_excerpt(c));
        prompt.push_str("\n\n");
    }
    prompt
}

// ---------------------------------------------------------------------------
// Fusion rollouts (review / synthesize escalation)
// ---------------------------------------------------------------------------

enum FusionKind {
    Review,
    Synthesize,
}

/// Run an escalation rollout that starts from the leading candidate's changes
/// (review) or from a synthesis prompt over the top candidates (synthesize).
#[allow(clippy::too_many_arguments)]
fn run_fusion_rollout(
    task: &Task,
    plan: &Plan,
    prompt: &str,
    thinking: ThinkingLevel,
    root: &Path,
    run_dir: &Path,
    base_dir: &Path,
    dirty: &DirtyState,
    router: &Router,
    route: &str,
    candidates: &[Candidate],
    kind: FusionKind,
) -> Result<(Candidate, EscalationInfo)> {
    let escalate_task = route_task(task, route, router, true)?;
    let index = candidates.len();
    let cand_dir = base_dir.join(format!("cand-{index:02}"));
    let wt = cand_dir.join("wt");
    add_worktree(root, &wt)?;
    let patch_path = base_dir.join(format!("cand-{index:02}.patch"));
    apply_dirty(root, &wt, &patch_path, dirty)?;

    let order = rank_indices(candidates);
    let leader = order
        .first()
        .and_then(|&i| candidates.get(i))
        .ok_or_else(|| "no candidates to escalate from".to_string())?;

    let fusion_prompt = match kind {
        FusionKind::Review => {
            // Seed the worktree with the leader's changes so the strong model
            // repairs them in place instead of starting over.
            let leader_diff =
                git(&leader.worktree, &["diff", "--binary", "HEAD"]).unwrap_or_default();
            if !leader_diff.trim().is_empty() {
                let leader_patch = cand_dir.join("leader.patch");
                fs::write(&leader_patch, &leader_diff).map_err(|e| e.to_string())?;
                git_apply(&wt, &leader_patch)?;
            }
            format!(
                "{prompt}\n\nESCALATION (REVIEW): The leading candidate below may contain \
                 defects; its changes are already applied in this working tree. Find and repair \
                 the issues, then finish the task.\n\n{}",
                candidate_excerpt(leader)
            )
        }
        FusionKind::Synthesize => {
            let mut text = format!(
                "{prompt}\n\nESCALATION (SYNTHESIZE): Several candidate solutions follow. \
                 Produce ONE new solution in this working tree that combines their best parts.\n\n"
            );
            for &i in order.iter().take(3) {
                text.push_str(&candidate_excerpt(&candidates[i]));
                text.push_str("\n\n");
            }
            text
        }
    };

    let mut fused = run_rollout(
        &escalate_task,
        plan,
        &fusion_prompt,
        thinking,
        &wt,
        &cand_dir,
        run_dir,
    );
    fused.route = escalate_task.effective_route.clone();
    fused.model = escalate_task.provider.model.clone();
    let info = EscalationInfo {
        route: escalate_task.effective_route.clone(),
        action: match kind {
            FusionKind::Review => "review",
            FusionKind::Synthesize => "synthesize",
        }
        .to_string(),
        reason: "ambiguous rollouts".to_string(),
    };
    Ok((fused, info))
}

// ---------------------------------------------------------------------------
// Driver
// ---------------------------------------------------------------------------

/// Run one scaled task end-to-end and return its final state.
#[allow(clippy::too_many_arguments)]
pub fn run_scaled_task(
    root: &Path,
    run_dir: &Path,
    task: &Task,
    plan: &Plan,
    prompt: &str,
    router: &Router,
    global_cap: usize,
    caps: &HashMap<String, usize>,
) -> TaskState {
    let policy = task.spec.effective_scaling(plan);
    let thinking = task.spec.effective_thinking(plan);
    let started = Instant::now();

    let fail =
        |reason: String| failed_state(task, thinking, started, 1, &reason, &Usage::missing());

    // Scaling needs git worktrees for candidate isolation; fail closed.
    if git(root, &["rev-parse", "--show-toplevel"]).is_err() {
        return fail("scaling requires a git workspace for candidate isolation".to_string());
    }

    let dirty = match capture_dirty(root) {
        Ok(dirty) => dirty,
        Err(e) => return fail(format!("capture workspace dirty state: {e}")),
    };

    let work_dir = run_dir.join("results").join(&task.id);
    let base_dir = work_dir.join("candidates");
    if let Err(e) = fs::create_dir_all(&base_dir) {
        return fail(format!("mkdir {}: {e}", base_dir.display()));
    }

    let artifact_snapshot = crate::runtime::capture_artifact_snapshot(root, task);

    let route_cap = caps
        .get(task.effective_route.as_str())
        .copied()
        .unwrap_or(1)
        .max(1);
    let wave_cap = route_cap.min(global_cap.max(1));

    let budget = policy.rollout_budget();
    let waves: Vec<usize> = match policy.mode {
        ScalingMode::AdaptiveParallel => vec![1, policy.candidates],
        _ => vec![policy.candidates],
    };

    let mut candidates: Vec<Candidate> = Vec::new();
    let mut verifier_info: Option<VerifierInfo> = None;
    let mut escalation_info: Option<EscalationInfo> = None;
    // (decision label, reason, winner index)
    let mut decision: Option<(String, String, usize)> = None;

    for (wave_index, &want) in waves.iter().enumerate() {
        let remaining = budget.saturating_sub(candidates.len());
        let count = want
            .min(remaining)
            .min(wave_cap)
            .max(if candidates.is_empty() { 1 } else { 0 });
        if count == 0 {
            break;
        }
        append_event(
            run_dir,
            "scaling_wave_started",
            json!({"task_id": task.id, "wave": wave_index + 1, "count": count}),
        );
        let start_index = candidates.len();
        match run_wave(
            task,
            plan,
            prompt,
            thinking,
            root,
            run_dir,
            &base_dir,
            &dirty,
            start_index,
            count,
        ) {
            Ok(mut wave) => candidates.append(&mut wave),
            Err(e) => return fail(format!("worktree wave failed: {e}")),
        }

        let passers = passer_count(&candidates);
        if passers == 1 {
            let winner = candidates.iter().position(|c| c.passed()).unwrap_or(0);
            decision = Some(("select".into(), "unique_verified".into(), winner));
            break;
        }
        if passers >= 2 || candidates.iter().any(|c| c.ok) {
            // Ambiguous (tie, or nothing deterministic separates them):
            // try the LLM verifier before spending more compute.
            if let Some(verifier_route) = policy.verifier_route.clone() {
                let verifier_task = match route_task(task, &verifier_route, router, false) {
                    Ok(t) => t,
                    Err(_) => continue,
                };
                let vdir = work_dir.join("verifier");
                let prompt = verifier_prompt(&task.spec.task, &candidates);
                if let Some((output, usage)) =
                    run_aux_model(&verifier_task, plan, root, run_dir, &vdir, &prompt)
                {
                    if let Some(verdict) = parse_verifier_verdict(&output) {
                        if verdict.scores.len() == candidates.len() {
                            for (c, score) in candidates.iter_mut().zip(verdict.scores.iter()) {
                                c.score = Some(*score);
                            }
                            verifier_info = Some(VerifierInfo {
                                route: verifier_task.effective_route.clone(),
                                model: verifier_task.provider.model.clone(),
                                confidence: Some(verdict.confidence),
                            });
                            if let Some(first) = candidates.first_mut() {
                                merge_usage(&mut first.usage, &usage);
                            }
                            if verdict.confidence >= policy.min_confidence {
                                let winner =
                                    rank_indices(&candidates).first().copied().unwrap_or(0);
                                decision =
                                    Some(("select".into(), "verifier_confident".into(), winner));
                                break;
                            }
                        }
                    }
                }
            }
        }
    }

    // Synthesis mode always produces a new solution from the strongest inputs.
    if decision.is_none() && policy.mode == ScalingMode::SynthesizeN {
        let synth_route = policy
            .escalate_route
            .clone()
            .or_else(|| policy.verifier_route.clone());
        if let Some(route) = synth_route {
            match run_fusion_rollout(
                task,
                plan,
                prompt,
                thinking,
                root,
                run_dir,
                &base_dir,
                &dirty,
                router,
                &route,
                &candidates,
                FusionKind::Synthesize,
            ) {
                Ok((mut fused, info)) => {
                    escalation_info = Some(info);
                    if fused.passed() {
                        let winner = candidates.len();
                        fused.index = winner;
                        candidates.push(fused);
                        decision =
                            Some(("synthesized".into(), "synthesis_verified".into(), winner));
                    } else {
                        decision = Some((
                            "select".into(),
                            "synthesis_failed_fallback".into(),
                            best_ranked(&candidates),
                        ));
                    }
                }
                Err(e) => return fail(format!("synthesis rollout failed: {e}")),
            }
        }
    }

    // Escalation for still-ambiguous adaptive/best-of-n tasks.
    if decision.is_none() {
        if let Some(escalate_route) = policy.escalate_route.clone() {
            let provider = router.get_provider(&escalate_route).cloned();
            let quota_ok = match &provider {
                Some(p) => crate::quota::QuotaGuard::load(root, &router.quota_policy)
                    .check(p)
                    .is_ok(),
                None => false,
            };
            if !quota_ok {
                decision = Some((
                    "select".into(),
                    "escalation_quota_blocked".into(),
                    best_ranked(&candidates),
                ));
            } else if policy.escalate_action == EscalateAction::Select {
                if let Ok(escalate_task) = route_task(task, &escalate_route, router, false) {
                    let edir = work_dir.join("escalate");
                    let prompt = selector_prompt(&task.spec.task, &candidates);
                    if let Some((output, usage)) =
                        run_aux_model(&escalate_task, plan, root, run_dir, &edir, &prompt)
                    {
                        if let Some(winner) = parse_verifier_verdict(&output)
                            .and_then(|v| v.winner)
                            .filter(|&w| w < candidates.len())
                        {
                            escalation_info = Some(EscalationInfo {
                                route: escalate_task.effective_route.clone(),
                                action: "select".into(),
                                reason: "ambiguous rollouts".into(),
                            });
                            if let Some(c) = candidates.get_mut(winner) {
                                merge_usage(&mut c.usage, &usage);
                            }
                            decision =
                                Some(("escalated_select".into(), "ambiguous".into(), winner));
                        }
                    }
                }
            } else {
                let kind = if policy.escalate_action == EscalateAction::Review {
                    FusionKind::Review
                } else {
                    FusionKind::Synthesize
                };
                if let Ok((mut fused, info)) = run_fusion_rollout(
                    task,
                    plan,
                    prompt,
                    thinking,
                    root,
                    run_dir,
                    &base_dir,
                    &dirty,
                    router,
                    &escalate_route,
                    &candidates,
                    kind,
                ) {
                    escalation_info = Some(info);
                    if fused.passed() {
                        let winner = candidates.len();
                        fused.index = winner;
                        candidates.push(fused);
                        let label = if policy.escalate_action == EscalateAction::Review {
                            "escalated_review"
                        } else {
                            "escalated_synthesize"
                        };
                        decision = Some((label.into(), "ambiguous".into(), winner));
                    }
                }
            }
        }
    }

    // Final fallbacks.
    let (label, reason, winner_index) = match decision {
        Some(d) => d,
        None => {
            if candidates.iter().any(|c| c.ok) {
                let tie = passer_count(&candidates) >= 2;
                (
                    "select".to_string(),
                    if tie {
                        "tie_first_verified".to_string()
                    } else {
                        "budget_exhausted".to_string()
                    },
                    best_ranked(&candidates),
                )
            } else {
                cleanup_worktrees(root, &candidates);
                let errors = candidates
                    .iter()
                    .filter_map(|c| c.error.clone().or_else(|| c.verify_error.clone()))
                    .collect::<Vec<_>>()
                    .join("; ");
                let mut state = fail(if errors.is_empty() {
                    "all rollouts failed".to_string()
                } else {
                    errors
                });
                state.scaling = Some(ScalingOutcome {
                    mode: mode_str(policy.mode).to_string(),
                    rollouts: rollout_infos(&candidates),
                    decision: "failed".to_string(),
                    decision_reason: "all_rollouts_failed".to_string(),
                    winner_index: None,
                    verifier: verifier_info,
                    escalation: escalation_info,
                });
                return state;
            }
        }
    };

    let Some(winner) = candidates.get(winner_index) else {
        return fail(format!("winner index {winner_index} out of range"));
    };

    // Bring the winner's changes into the real workspace, then run the
    // standard root-level artifact + verify gates exactly once.
    if let Err(e) = apply_winner(root, winner) {
        cleanup_worktrees(root, &candidates);
        let mut state = fail(format!("apply winner candidate: {e}"));
        state.scaling = Some(outcome_of(
            &policy,
            &candidates,
            &label,
            &reason,
            Some(winner_index),
            verifier_info,
            escalation_info,
        ));
        return state;
    }
    cleanup_worktrees(root, &candidates);

    if let Err(e) = check_artifacts_with_snapshot(root, task, Some(&artifact_snapshot)) {
        let mut state = fail(e);
        state.scaling = Some(outcome_of(
            &policy,
            &candidates,
            &label,
            &reason,
            Some(winner_index),
            verifier_info,
            escalation_info,
        ));
        return state;
    }

    let (verified, verify_error) = run_verify_commands(task, root, &work_dir);
    if verified == Some(false) || (task.spec.requires_verification() && verified != Some(true)) {
        let mut state = failed_state(
            task,
            thinking,
            started,
            1,
            verify_error
                .as_deref()
                .unwrap_or("role requires verification but none passed"),
            &Usage::missing(),
        );
        state.verified = verified;
        state.verify_error = verify_error;
        state.scaling = Some(outcome_of(
            &policy,
            &candidates,
            &label,
            &reason,
            Some(winner_index),
            verifier_info,
            escalation_info,
        ));
        return state;
    }

    let total_usage = candidates.iter().fold(Usage::missing(), |mut acc, c| {
        merge_usage(&mut acc, &c.usage);
        acc
    });

    let summary = format!(
        "scaling mode={} rollouts={} winner=cand-{winner_index} route={} model={} decision={} reason={}\n",
        mode_str(policy.mode),
        candidates.len(),
        winner.route,
        winner.model,
        label,
        reason,
    );
    let _ = fs::write(work_dir.join("worker.log"), summary);

    append_event(
        run_dir,
        "scaling_decision",
        json!({
            "task_id": task.id,
            "decision": label,
            "reason": reason,
            "winner": winner_index,
            "rollouts": candidates.len(),
        }),
    );

    let mut state = success_state(
        task,
        thinking,
        started,
        1,
        false,
        None,
        0,
        verified,
        verify_error,
        &total_usage,
        None,
    );
    state.scaling = Some(outcome_of(
        &policy,
        &candidates,
        &label,
        &reason,
        Some(winner_index),
        verifier_info,
        escalation_info,
    ));
    state
}

// ---------------------------------------------------------------------------
// Small helpers
// ---------------------------------------------------------------------------

fn mode_str(mode: ScalingMode) -> &'static str {
    match mode {
        ScalingMode::Single => "single",
        ScalingMode::BestOfN => "best_of_n",
        ScalingMode::AdaptiveParallel => "adaptive_parallel",
        ScalingMode::SynthesizeN => "synthesize_n",
    }
}

fn best_ranked(candidates: &[Candidate]) -> usize {
    rank_indices(candidates).first().copied().unwrap_or(0)
}

fn rollout_infos(candidates: &[Candidate]) -> Vec<RolloutInfo> {
    candidates
        .iter()
        .map(|c| RolloutInfo {
            index: c.index,
            route: c.route.clone(),
            model: c.model.clone(),
            ok: c.ok,
            verified: c.verified,
            score: c.score,
            duration_ms: c.duration_ms,
            usage: c.usage.clone(),
        })
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn outcome_of(
    policy: &ScalingPolicy,
    candidates: &[Candidate],
    label: &str,
    reason: &str,
    winner: Option<usize>,
    verifier: Option<VerifierInfo>,
    escalation: Option<EscalationInfo>,
) -> ScalingOutcome {
    ScalingOutcome {
        mode: mode_str(policy.mode).to_string(),
        rollouts: rollout_infos(candidates),
        decision: label.to_string(),
        decision_reason: reason.to_string(),
        winner_index: winner,
        verifier,
        escalation,
    }
}

fn cleanup_worktrees(root: &Path, candidates: &[Candidate]) {
    for c in candidates {
        remove_worktree(root, &c.worktree);
    }
    let _ = git(root, &["worktree", "prune"]);
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn candidate(index: usize, ok: bool, verified: Option<bool>, score: Option<f64>) -> Candidate {
        Candidate {
            index,
            route: "mock".into(),
            model: "m".into(),
            worktree: PathBuf::new(),
            dir: PathBuf::new(),
            ok,
            error: None,
            verified,
            verify_error: None,
            score,
            duration_ms: 0,
            usage: Usage::offline_mock(),
        }
    }

    #[test]
    fn rank_prefers_verified_then_healthy_then_score_then_index() {
        let candidates = vec![
            candidate(0, true, None, None),
            candidate(1, true, Some(true), None),
            candidate(2, false, None, None),
            candidate(3, true, Some(true), Some(5.0)),
        ];
        let order = rank_indices(&candidates);
        assert_eq!(order[0], 3, "highest-scoring passer first");
        assert_eq!(order[1], 1, "second passer by score/index");
        assert_eq!(order[2], 0, "healthy unverified next");
        assert_eq!(order[3], 2, "failed rollout last");
    }

    #[test]
    fn verifier_verdict_parses_json_from_noisy_output() {
        let raw =
            "some prefix text\n{\"scores\":[1.0,9.0],\"confidence\":0.9,\"winner\":1}\ntrailing";
        let verdict = parse_verifier_verdict(raw).expect("verdict parses");
        assert_eq!(verdict.scores, vec![1.0, 9.0]);
        assert_eq!(verdict.winner, Some(1));
        assert!((verdict.confidence - 0.9).abs() < f64::EPSILON);
        assert!(parse_verifier_verdict("no json here").is_none());
    }

    #[test]
    fn rollout_budget_matches_mode() {
        let base = ScalingPolicy {
            candidates: 4,
            ..Default::default()
        };
        let best_of_n = ScalingPolicy {
            mode: ScalingMode::BestOfN,
            ..base.clone()
        };
        assert_eq!(best_of_n.rollout_budget(), 4);
        let adaptive = ScalingPolicy {
            mode: ScalingMode::AdaptiveParallel,
            ..base.clone()
        };
        assert_eq!(adaptive.rollout_budget(), 5);
        let capped = ScalingPolicy {
            max_rollouts: 2,
            ..base
        };
        assert_eq!(capped.rollout_budget(), 2);
    }

    #[test]
    fn scaling_policy_deserializes_plan_json() {
        let value = json!({"mode": "best_of_n", "candidates": 3, "verifier_route": "mock"});
        let policy: ScalingPolicy = serde_json::from_value(value).unwrap();
        assert_eq!(policy.mode, ScalingMode::BestOfN);
        assert_eq!(policy.candidates, 3);
        assert_eq!(policy.verifier_route.as_deref(), Some("mock"));
        assert!((policy.min_confidence - 0.7).abs() < f64::EPSILON);
    }
}
