# SWARMS

![SWARMS workflow cover](images/swarms-cover.png)

Local-first orchestration for coding agents.

SWARMS lets you decide which model plans, which model codes, which model reviews, and how many workers may run at the same time. The repo runs offline out of the box. Model calls happen only when you configure your own plans, APIs, CLIs, and routing policy.

Website: https://swarms-orchestrator.vercel.app/

I have used versions of this workflow personally since January-February 2026. The original idea came from Ralph-style coding loops: keep a strong model on planning and review, then let cheaper workers handle implementation, QA, issue triage, and repeated validation.

Español: [README.es.md](README.es.md)

## HY3 Provider Routes

SWARMS includes several configurable HY3 routes. Provider pricing, promotions,
and account requirements can change; verify them with the provider before a
real run.

| Route | Provider | Model ID | Free? |
|---|---|---|---|
| `hy3_opencode` | OpenCode Zen | `opencode/hy3-free` | Free tier |
| `hy3_gitlawb` | GitLawb OpenGateway | `tencent/hy3` | Free promo |
| `hy3_openrouter` | OpenRouter | `tencent/hy3:free` | Free variant |
| `hy3_kilo` | Kilo AI Gateway | `tencent/hy3` | Provider-dependent |
| `hy3_siliconflow` | SiliconFlow | `tencent/Hy3` | Paid |

The bundled example plan is intentionally mock-only. For a private plan whose
tasks explicitly use `hy3_gitlawb`, the execution shape is:

```bash
python scripts/swarm.py run --plan path/to/hy3-plan.json --force \
  --global-max-concurrency 3 --provider-cap hy3_gitlawb=3
```

The **Hermes Agent** adapter can run a tool-calling subagent only after local
configuration pins an explicit model. An unpinned route is rejected.

All HY3 routes are **disabled by default** (mock stays the safe open-source
default). Enable the ones you want in `config/swarm_router.local.json` and set
the matching API keys in your environment.

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
- A bundled SWARMS skill in `skills/swarms/` so a user's agent can help configure plans, providers, caps, and verification.

The committed router enables only `mock`. That keeps a clone local and free. Your private setup lives in ignored files such as `config/swarm_router.local.json` and your own environment variables.

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
Copy-Item -Recurse -Force .\skills\swarms "$env:USERPROFILE\.codex\skills\swarms"
```

After that, an agent can inspect your local provider setup, draft a plan, review it, and run the offline validation path before you enable real routes.

## Quick Start

Requires Python 3.10+ and Git.

```powershell
python scripts/swarm.py doctor
python scripts/swarm.py review --plan docs/workflow_plan_example.json
python scripts/swarm.py dry-run --plan docs/workflow_plan_example.json --force
python scripts/swarm.py run --plan docs/workflow_plan_example.json --force --global-max-concurrency 3 --provider-cap mock=3
```

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

The runtime stores state under `.agent/swarm/runs/<run_id>/`. It keeps worker prompts, logs, task state, lifecycle events, result JSON, and final reports out of the coordinator context.

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
