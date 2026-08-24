# SWARMS

![SWARMS workflow cover](images/swarms-cover.png)

Local-first orchestration for coding agents.

SWARMS lets you decide which model plans, which model codes, which model reviews, and how many workers may run at the same time. The repo runs offline out of the box. Model calls happen only when you configure your own plans, APIs, CLIs, and routing policy.

Website: https://swarms-orchestrator.vercel.app/

New: [Swarm-Driven Development (SwDD)](https://github.com/Mchicao/swarm-driven-development) connects OpenSpec, SWARMS, Gentle-AI, and Engram behind one native `swdd` command — spec first, swarm execution, receipt-backed delivery.

I have used versions of this workflow personally since January-February 2026. The original idea came from Ralph-style coding loops: keep a strong model on planning and review, then let cheaper workers handle implementation, QA, issue triage, and repeated validation.

Español: [README.es.md](README.es.md)

## Free Provider Opportunities

With this harness, you can take advantage of free usage offered by AI providers
such as TokenRouter_US, Modal, or other providers that offer promotional access
from time to time. For example, a provider may advertise free tokens for models
such as Kimi K3 or GLM 5.2.

Availability, quotas, eligibility, pricing, and terms can change at any time.
Always verify the current offer directly with the provider before running a
workload. SWARMS does not guarantee free access; it routes only to the
providers and models you configure locally.

### Verified free routes (tested August 2026)

These four routes are configured and verified working end to end in a local
`config/swarm_router.local.json`:

| Route | Model | Source | Cost |
|---|---|---|---|
| `ox_alpha_free` | `opencode/x-preview-f-free` — Ox Alpha Free (Unlimited) | OpenCode Zen | $0 |
| `ox_alpha_hermes` | `stealth/ox-alpha` — Ox Alpha promo ("quadrillion tokens per day") | Nous Portal via Hermes Agent | $0 (limited time) |
| `muse_spark_free` | `opencode/muse-spark-1.2-contributor-free` — Muse Spark 1.2 Free, 1M ctx | OpenCode Zen | $0 |
| `gemini37_flash_medium` | Gemini 3.7 Flash (Medium) | Antigravity CLI | $0 |

Plus GLM 5.3 (`zai-coding-plan/glm-5.3`) through OpenCode on a Z.AI coding plan,
and Tencent HY3 across five additional free routes. Run four free workers in
parallel with:

```bash
cargo run --manifest-path rust/Cargo.toml -- run --force \
  --plan my_plan.json --global-max-concurrency 4 --provider-cap ox_alpha_free=4
```

## Claude Code and GPT-5.6 Ultra-Style Workflows

Claude Fable 5 can power long-running, multi-agent workflows in Claude Code by planning across stages, delegating to subagents, and checking its own work. OpenAI has also announced a new GPT-5.6 `ultra` mode built around subagents, but GPT-5.6 remains in limited preview rather than broad public availability. SWARMS targets this operating pattern from the local-first side: you choose the planner, critic, worker models, provider caps, verification metadata, and token budget.

Use SWARMS when you want an Ultra-style agent crew without tying the whole workflow to one vendor mode:

- run everything locally until you enable real providers;
- route planner, critic, programmer, verifier, and QA roles to different models;
- mix configured OpenAI-compatible APIs, GLM, Gemini, Codex CLI, Hermes, or offline mock workers;
- keep provider caps and reports visible;
- run Singularity when you want a long-running loop that keeps proposing, implementing, testing, and summarizing work.

## Integrations

SWARMS includes compatibility paths, wrappers, docs, routing names, or telemetry support for:

- OpenAI-compatible APIs.
- LiteLLM-style routing.
- Anthropic-style premium planner/critic routes.
- GLM 5.2 through OpenCode or Z.AI-style routes.
- Gemini 3.5 Flash through Antigravity CLI.
- Codex CLI for premium orchestration or escalation.
- OpenAI-compatible gateway routes configured by the user.
- Offline `mock` workers for CI, demos, and safe setup.
- Token/cost parsing for Codex logs, OpenCode logs, stdout-like CLI usage, cache reads, cache writes, and reasoning tokens.
- Two bundled skills in `.skillshare/skills/`: `swarms` for operating this runtime and `multi-provider-agent-orchestration` for safe delegation across agents.

The committed router enables only `mock`. That keeps a clone local and free. Your private setup lives in ignored files such as `config/swarm_router.local.json` and your own environment variables.

OpenCode 2.0 and pi-agent are explicit future provider targets. They are not
implemented routes yet; current OpenCode support must not be presented as
automatic OpenCode 2.0 compatibility.

## How Configuration Works

You choose the policy:

- Plans define roles, tasks, dependencies, expected artifacts, verification metadata, and premium permissions.
- `config/role_policy.json` defines planner, critic, programmer, and verifier intent.
- `config/swarm_router.json` is the safe local default.
- `config/swarm_router.local.example.json` shows how to enable your own providers.
- Provider caps limit concurrency per route.
- Token telemetry records what the CLI or API reports, and marks missing usage instead of pretending it was free.

The included skill teaches compatible agents how to use SWARMS:

```powershell
Copy-Item -Recurse -Force .\.skillshare\skills\swarms "$env:USERPROFILE\.codex\skills\swarms"
```

After that, an agent can inspect your local provider setup, draft a plan, review it, and run the offline validation path before you enable real routes.

## Rust Coordinator

The Rust binary is the sole public runtime. It is self-contained — no Python
dependency. All adapters (mock, Codex, OpenCode, Kilo, Hermes, agy,
OpenAI-compatible HTTP) are implemented natively in Rust.

```bash
cargo run --manifest-path rust/Cargo.toml -- doctor
cargo run --manifest-path rust/Cargo.toml -- review --plan docs/workflow_plan_example.json
cargo run --manifest-path rust/Cargo.toml -- dry-run --plan docs/workflow_plan_example.json --force
cargo run --manifest-path rust/Cargo.toml -- run --plan docs/workflow_plan_example.json --force --global-max-concurrency 3 --provider-cap mock=3
```


See `docs/RUST_RUNTIME.md` for the full architecture, thinking levels, session
affinity, and telemetry documentation.

## Parallel Agents (Test-Time Scaling)

Some tasks deserve more than one attempt. Instead of always paying for one
expensive execution, a task can run several cheap candidates in parallel, keep
the best one, and only spend more compute when the result is uncertain.

The evidence says this works: on DeepSWE, GLM trails Fable on a single shot
(69.7 vs 69.0) but passes it with 2 shots (81.1 vs 77.1) and 4 shots (87.6 vs
84.1) — at a fraction of the price. Similarly, sampling 5 DeepSeek V4 Flash
solutions and self-verifying with LLM-as-a-Verifier lifted Terminal-Bench 2.1
from 79% to 88%, beating Claude Fable 5 while being ~11× cheaper.

```text
Task
  -> N parallel candidates (each in its own isolated git worktree)
  -> tests/build run per candidate
  -> one clear winner?  -> done, no extra model calls
  -> tie or unclear?    -> verifier model scores the candidates
  -> still unclear?     -> stronger model selects, repairs, or synthesizes
```

Enable it per task with a `scaling` block in the plan:

```json
{
  "id": "compress",
  "route": "gemini37_flash_medium",
  "verify": ["python -m pytest bench_tests/ -q"],
  "scaling": {
    "mode": "adaptive_parallel",
    "candidates": 3,
    "verifier_route": "gemini37_flash_medium",
    "escalate_route": "gemini37_flash_medium",
    "escalate_action": "review"
  }
}
```

Four modes:

| Mode | What happens | Rollouts |
|---|---|---|
| `single` (default) | One execution, classic behavior | 1 |
| `best_of_n` | N candidates race in parallel, best wins | N |
| `adaptive_parallel` | 1 candidate first; expands to N more only if verification is ambiguous | 1 to 1+N |
| `synthesize_n` | N candidates, then one model writes a new solution combining the best parts | N+1 |

How the winner is picked — objective checks first, models only as tie-breaker:

1. Your `verify` commands (tests, build, lint) run inside each candidate's
   worktree. If exactly one candidate passes, it wins and no model judges
   anything.
2. On a tie, an optional `verifier_route` model (it can be cheaper than the
   worker) scores every candidate in one call. A low-confidence verdict never
   decides.
3. Still ambiguous? The `escalate_route` model `select`s the best candidate,
   `review`s and repairs the leader, or `synthesize`s a new solution.

Costs stay bounded: total rollouts are capped (`max_rollouts`), parallel waves
respect your provider and global concurrency caps, escalation is
quota-checked, and premium escalation routes still need explicit
`premium_allowed`. Every task records what it spent — rollouts, models, tokens,
scores, winner, and why — in its task state, so you can compare policies and
see whether scaling was worth it.

Try it offline-safe with the bundled example (needs the
`gemini37_flash_medium` route enabled in your local router, or swap routes to
`mock`):

```bash
cargo run --manifest-path rust/Cargo.toml -- review --plan docs/workflow_plan_scaling_example.json
cargo run --manifest-path rust/Cargo.toml -- run --plan docs/workflow_plan_scaling_example.json --force
```

## Quick Start

Requires Python 3.10+ and Git.

Before enabling legacy real-provider routes on a new machine, inspect the local
agent inventory:

```powershell
python scripts/swarm.py preflight --format json
```

See `docs/AGENT_PREFLIGHT.md`. The Python compatibility runtime refuses
unverified real agents before creating claims or workers.

```powershell
python scripts/swarm.py doctor
python scripts/swarm.py review --plan docs/workflow_plan_example.json
python scripts/swarm.py dry-run --plan docs/workflow_plan_example.json --force
python scripts/swarm.py run --plan docs/workflow_plan_example.json --force --global-max-concurrency 3 --provider-cap mock=3
```

Run-state files are the read-only integration boundary used by observability
tools; see `docs/STATE_CONTRACT.md`.

Optional editable install:

```powershell
python -m pip install -e ".[dev,yaml]"
swarms doctor
swarms run --plan docs/workflow_plan_example.json --force --global-max-concurrency 3 --provider-cap mock=3
```

## Runtime Model

![SWARMS runtime map](images/runtime-map.png)

```text
goal
  -> workflow plan
  -> static review
  -> deterministic runtime
  -> provider pools under caps
  -> worker output
  -> verification and report.json
```

The runtime stores state under `<workspace-root>/.agent/swarm/runs/<run_id>/`.
It keeps worker prompts, logs, task state, lifecycle events, result JSON, and
final reports beside the selected target workspace and out of the coordinator
context.

## Singularity Mode

Singularity is the autonomous loop for people who are willing to spend the tokens.

The intended use is a 24/7 local agent crew: propose improvements, read issues, create tasks, run workers, perform QA, validate features, summarize what changed, then start the next cycle. It is the closest SWARMS gets to a standing engineering loop.

```powershell
pwsh scripts/start_singularity.ps1 -MaxCycles 5
```

You control the risk. With only `mock`, Singularity is a local dry run. With real providers, high worker counts, and high cycle limits, it can consume a large amount of tokens. Use provider caps, `MaxCycles`, and a `STOP_SINGULARITY` file when you test it.

## Ideas To Implement

SWARMS should eventually connect the autonomous loop to the tools where engineering work already lives:

- Trello: read cards, create implementation plans, move cards after validation.
- Hermes Agent: use Hermes as another local agent route or coordination surface.
- OpenCode 2.0: add a versioned adapter after validating its actual CLI/API,
  sessions, steering, telemetry and workspace boundary.
- pi-agent: evaluate it as an opt-in provider/runtime after the same safety and
  offline-fixture gates; do not alias it to another adapter.
- Discord: post cycle summaries, request approvals, and accept lightweight commands.
- JIRA: read tickets, plan work, update status, and attach verification reports.
- Microsoft Teams: send QA summaries, escalation notices, and Singularity cycle reports.

## Provider Policy

Default role intent:

- Planner: Claude Fable can be configured as a premium planning agent. GPT-5.6 Sol is documented as a future option while access remains limited; GLM 5.2 stays the safe default.
- Critic: GLM 5.2 first, premium review for high-risk or high-cost plans.
- Programmer: GLM 5.2, Gemini Flash, OpenAI-compatible, LiteLLM, Kilo, Aider, or any route you configure.
- Verifier: run deterministic tests outside the harness first, then use cheap model review.
- Premium routes: explicit plan permission plus local config.

See `docs/PROVIDER_STATUS.md`, `docs/CONFIG.md`, `docs/DYNAMIC_WORKFLOWS.md`, and `AGENTS.md`.

## Origin

I built the first versions for personal use around January-February 2026. At the time I had student-plan constraints and wanted to stretch the models I could access: Gemini in Antigravity for worker loops, Opus for plans, and later GLM 5.2 and Codex for stronger planner/critic paths.

The shape stayed the same: spend scarce models on decisions, not on repetitive work.

## Verification

```powershell
python -m ruff check .
python -m py_compile scripts\swarm.py scripts\plan_review.py scripts\workflow_runtime.py scripts\doctor.py scripts\mock_worker.py
python -m pytest tests -q
python scripts/swarm.py doctor
python scripts/swarm.py run --plan docs\workflow_plan_example.json --force --run-id verify-readme --global-max-concurrency 3 --provider-cap mock=3
```

## License

MIT. See `LICENSE`.
