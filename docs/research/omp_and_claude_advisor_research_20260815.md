# OMP and Claude Advisor: SOTA models advising cheaper models vs SOTA planning + cheap implementation

**Cutoff date:** August 16, 2026 (updated after the user supplied the real OMP URLs; first pass dated August 15).
**Scope:** companion to @docs/research/model_routing_deep_dive_20260815.md and @docs/research/cursor_model_orchestration_research_20260814.md. Question: is "a SOTA model continuously advising cheaper executors" better than SWARMS' current "SOTA plans once, cheap models implement, critic verifies"?
**Verification note:** the first pass misidentified both named systems; the user's URLs resolved them. "OMP" is **Oh My Pi (omp)**, can1357's Pi-fork terminal coding agent (omp.sh) whose advisor feature is a persistent second model that reviews every turn and injects advice — not an "Optimal Model Power" metric, and unrelated to the LLM Council paper. "Claude Advisor" is Anthropic's advisor tool (April 2026, executor-initiated, Anthropic executors only). omp's advisor is push-style (advisor initiates); Anthropic's is pull-style (executor initiates). Details in sections 1-3.

## TL;DR

No public head-to-head comparison of "advisor-in-the-loop" vs "plan-once + cheap execution" exists; every quantitative claim below is a vendor measurement. The verified evidence says the two patterns solve different problems rather than compete. Anthropic's advisor tool (a stronger model consulted mid-task by a cheaper executor) measurably lifts weak executors on serial tasks that are hard in spots: +2.7 pp on SWE-bench Multilingual at 11.9% lower cost per task for Sonnet+Opus, and 41.2% vs 19.7% on BrowseComp for Haiku+Opus (vendor claims). But the same vendor's deeper measurements show the lift depends on a capability gap between executor and advisor and, fragilely, on the executor actually choosing to consult; when the executor stops consulting, the pairing scores below the executor alone, and at the top of the capability range a single model at tuned effort matches the pairing for the same money. Anthropic's own guidance keeps plan-once-style orchestration (strong planner, cheap workers) for divisible or fan-out work and reserves the advisor for serial work whose difficulty is concentrated in a few decisions. "OMP" is now identified: **Oh My Pi (omp.sh)**, a Pi-fork coding agent that ships a push-style advisor — a second model reading every turn and injecting nit/concern/blocker notes, with hold-and-reconfirm delivery and catch-up stalls (design documented in detail by the pi-omplike-advisor port, whose default advisor model is GLM 5.2 at low thinking). omp itself ships plan-once machinery side by side (plan mode, `orchestrate`, `workflowz`, `/vibe`), treating the two patterns as per-task choices. Neither omp nor the port publishes advisor-effectiveness benchmarks — omp's public numbers are about harness/edit formats — so continuous advising remains a design-convergence signal (two independent implementations exist) rather than measured evidence.

## 1. Claude Advisor (Anthropic advisor tool) — verified primary sources

### What it is, architecturally

- The advisor tool is a beta server-side tool (`advisor_20260301`, beta header `advisor-tool-2026-03-01`). A cheaper **executor** model (Sonnet or Haiku) runs the whole task end-to-end; when it hits a decision it cannot reasonably solve, it emits a tool call and Anthropic runs a separate inference pass on a stronger **advisor** model (Opus or Fable), inside a single `/v1/messages` request. The advisor receives the executor's full transcript (system prompt, tools, prior turns, current turn), runs without tools and without context management, and returns only advice text; the executor continues. [Advisor tool API docs](https://platform.claude.com/docs/en/agents-and-tools/tool-use/advisor-tool)
- The **executor decides when to consult**; it tends to call before committing to an approach, on recurring errors, and before declaring completion. There is no built-in conversation-level cap; callers cap via `max_uses` (per request) or by removing the tool. [API docs](https://platform.claude.com/docs/en/agents-and-tools/tool-use/advisor-tool)
- Claude Code exposes it as `/advisor`, `advisorModel`, and `--advisor`. Pairing is enforced: the advisor must be at least as capable as the main model (Haiku main accepts Opus/Fable/Sonnet advisors; a Fable main accepts only a Fable advisor). In Claude Code, each advisor call re-processes the full transcript uncached (the main model's prompt cache is preserved; the API now offers an opt-in `caching` object for the advisor transcript). [Claude Code advisor docs](https://code.claude.com/docs/en/advisor)
- Anthropic explicitly frames this as the **inverse of the planner/worker pattern**: "This inverts a common sub-agent pattern, where a larger orchestrator model decomposes work and delegates to smaller worker models. In the advisor strategy, a smaller, more cost-effective model drives and escalates without decomposition, a worker pool, or orchestration logic." (vendor claim) [The advisor strategy, April 9, 2026](https://claude.com/blog/the-advisor-strategy)
- Claude Code ships both patterns side by side and documents when to use each: advisor (strong model at decision points mid-task) vs `opusplan` (stronger model during plan mode, then Sonnet executes) vs subagents with a set model (strong model for a whole delegated subtask). [Comparison table](https://code.claude.com/docs/en/advisor#compare-with-related-features)

### Quantitative claims (all Anthropic-internal evaluations — vendor claims)

From the announcement post ([claude.com/blog/the-advisor-strategy](https://claude.com/blog/the-advisor-strategy), with benchmark conditions in its footnotes):

- SWE-bench Multilingual: Sonnet 4.6 + Opus 4.6 advisor scored **+2.7 pp over Sonnet solo while cutting cost per agentic task by 11.9%** (five trials of 300 problems across nine languages).
- BrowseComp: **Haiku 4.5 + Opus advisor scored 41.2% vs 19.7% solo** (more than double); it trails Sonnet solo by 29% in score but costs 85% less per task (1,266 problems, one attempt each).
- BrowseComp and Terminal-Bench 2.0: Sonnet + advisor improved scores **while costing less per task than Sonnet alone** (exact deltas shown only as charts).
- Typical advisor output is **400-700 text tokens** per consultation, billed at advisor rates, with executor tokens at executor rates and separate usage reporting.

From the platform cost/intelligence guide ([Optimizing for cost and intelligence](https://platform.claude.com/docs/en/about-claude/models/optimizing-for-cost-and-intelligence)):

- Mechanism: across measured pairings, the advisor **closed 60-90% of the gap to the stronger model**, which was paid for only on consultations.
- The gain is **gap-dependent**: on GPQA Diamond a Haiku executor gained a lot from an Opus 5 advisor, a Sonnet 5 executor gained a few points, and a frontier executor gained almost nothing.
- The **consult rate is the fragile factor**: a low-effort executor "can stop noticing it is stuck"; on DeepSWE a low-effort Sonnet 5 executor kept consulting and **gained 23 points**, while on SWE-bench Pro the same executor stopped consulting and fell below executor-alone. Under-calling is common enough that Anthropic ships a suggested system prompt (~2-3 consults per task) and documents a **mid-conversation nudge**: reminding a Haiku executor that has not consulted raised pass rates ~7 pp, had no measurable effect on Sonnet, and slightly lowered pass rates on Opus ([API docs, nudge section](https://platform.claude.com/docs/en/agents-and-tools/tool-use/advisor-tool)).
- At the top of the range the pairing buys little: on an internal agentic-coding benchmark, Opus 5 + Fable 5 advisor scored 85.7% at $8.40/attempt vs Opus 5 alone at default effort 84.4% at $8.50 and **Fable 5 alone at medium effort 83.4% at $8.20** — Anthropic itself notes the single model at tuned effort "reaches about the same accuracy as the pairing for about the same money".
- Where it does pay: on Chartography, a low-effort Opus 5 executor with a Fable 5 advisor scored 67.5 at $0.60/task (86% consult rate), beating both models' own effort curves at the low end.

The same guide's orchestrator results are the direct plan-once comparison point (section 4).

### Practical notes for interpreting the vendor numbers

- All advisor benchmarks are Anthropic-run, mostly single-run, and the post recommends customers "run your existing eval suite against Sonnet solo, Sonnet + Opus advisor, and Opus solo" — i.e., Anthropic does not claim universal wins. [Blog](https://claude.com/blog/the-advisor-strategy)
- The guide's meta-finding undercuts multi-model enthusiasm generally: "a multi-model configuration that looked cheaper than the default single model cost more than that same model at lower effort" in internal measurements — sweep effort on one model before adding a second. [Guide](https://platform.claude.com/docs/en/about-claude/models/optimizing-for-cost-and-intelligence)

## 2. OMP = Oh My Pi (omp.sh) — push-style advisor over the main model

### Identification

- **omp ("Oh My Pi")**, by Can Bölük (can1357): [omp.sh](https://omp.sh/), [GitHub can1357/oh-my-pi](https://github.com/can1357/oh-my-pi), npm `@oh-my-pi/pi-coding-agent`. A terminal coding agent forked from Mario Zechner's [Pi](https://github.com/badlogic/pi-mono) (badlogic/pi-mono) with a ~80k-line Rust core. "OMP" is the product name; it is not a metric and has nothing to do with the LLM Council paper (the wrong "Optimal Model Power" lead is kept in section 6).
- Name-collision note: the unrelated npm package `oh-my-pi` (acidsugarx, a Pi-CLI orchestrator-prompt framework, [registry](https://registry.npmjs.org/oh-my-pi)) is a different project. The advisor port below explicitly ports can1357's omp advisor.

### The advisor feature (omp README, feature 06)

- "A second model, watching every turn": pair a reviewer model to the `advisor` role and "it reads every turn the main agent takes, injecting notes inline — a quiet aside, a concern, or a hard blocker. It runs on its own context and its own model, so it catches what the doer rushed past. The main agent sees the note and course-corrects, or tells you why it won't." The README's example runs the advisor on `openai-codex/gpt-5.5` over the main agent. [README](https://github.com/can1357/oh-my-pi#readme)
- Model **roles** route work by intent — `default`, `smol` (cheap subagent fan-out), `slow` (deep reasoning), `plan` (plan mode), `commit`, `vision`, `designer`, `task`, `advisor`, `tiny` — so an expensive advisor over a cheaper driver is a first-class configuration, not a hack. [README](https://github.com/can1357/oh-my-pi#readme)
- omp ships **plan-once-style machinery side by side**: plan mode (`plan` role), the `orchestrate` keyword ("run substantial independent work through parallel subagents and verify each phase"), `workflowz` ("build a deterministic multi-subagent workflow"), and `/vibe` (a director model driving persistent `fast`/`good` worker sessions with a read-only toolset). The product treats continuous advising and orchestration as complementary per-task choices — the same positioning as Anthropic's docs.
- **No advisor benchmarks are published.** omp's public quantitative claims are about the harness/edit formats (Grok Code Fast 1 6.7%→68.3% pass rate, Grok 4 Fast −61% output tokens, MiniMax 2.1× pass rate) in [The harness problem](https://blog.can.ac/2026/02/12/the-harness-problem/) (Feb 12, 2026) — vendor claims, and not about the advisor.

### pi-omplike-advisor: the design, documented (community port)

pasky's [pi-omplike-advisor](https://github.com/pasky/pi-omplike-advisor) (v1.0.2, Jul 6 2026; MIT; 105 stars; [pi.dev package page](https://pi.dev/packages/pi-omplike-advisor)) ports omp's advisor onto upstream pi's extension surface; its README is the most detailed public description of the design:

- The advisor is a **long-lived, read-only Agent**: own model, read-only tools (`read`/`grep`/`find`) plus one `advise` tool. It is fed the primary agent's transcript **one turn-delta at a time**; review is asynchronous (seconds behind). It cannot edit files, run commands, or change session state.
- **Severity-tiered delivery**: `nit` ships immediately (mild staleness acceptable; at terminal turns it must survive final-review reconfirmation), while `concern`/`blocker` are **always held and re-confirmed by the next review** — the advisor re-raises survivors and stays silent on resolved ones. This targets advice staleness, the structural problem of push review.
- **Catch-up blocks**: while a high-severity note is held (or a turn is about to idle), the primary's next step is stalled so the advisor can catch up; backoff 15s → 30s → 60s → 120s cap; never a hard interrupt.
- **Self-compaction**: the advisor's own context is compacted proactively at 80% (`ADVISOR_COMPACT_AT`, clamped 50–95) and reactively on overflow, with held notes riding along outside the transcript — so long sessions keep getting reviewed instead of silently failing.
- **Default advisor model: `openrouter/z-ai/glm-5.2`, thinkingLevel `low`** — a mid-tier model, not a frontier one, as the community default advisor.
- `WATCHDOG.md` in the working directory is appended to the advisor's system prompt as project-specific review guidance.

### Architectural contrast: omp push vs Anthropic pull

| Dimension | omp advisor (push) | Anthropic advisor tool (pull) |
| --- | --- | --- |
| Who initiates | Advisor reviews every turn unconditionally | Executor decides when to consult |
| Known failure mode | Stale advice, review latency (mitigated by hold-and-reconfirm, catch-up blocks) | Executor never consults, esp. at low effort (Anthropic-measured) |
| Advisor cost shape | Every turn-delta, incremental context + self-compaction | Only on consult, but each call re-reads the full transcript (uncached by default) |
| Advice authority | Advisory; main agent "course-corrects, or tells you why it won't" | Advisory; executor continues with advice text |
| Evidence | Shipped in two implementations (omp + pi port); no benchmarks | Anthropic-internal evals only (vendor) |

The push design directly eliminates the pull design's documented weakness — the executor failing to recognize it is stuck — at the price of paying advisor tokens on every turn and adding latency gates. No published measurement of that trade exists.

## 3. LLM Council (adjacent, kept for the record)

The initial lead tied "OMP" to a metric in the "Language Model Council" paper; that tie was wrong, but the paper is real adjacent evidence on collective oversight: **Language Model Council: Democratically Benchmarking Foundation Models on Highly Subjective Tasks** (Zhao, Plaza-del-Arco, Genchel, Cercas Curry; arXiv [2406.08598](https://arxiv.org/abs/2406.08598), v1 June 2024, v4 March 2025; [HTML v4](https://arxiv.org/html/2406.08598v4)):

- A council of 20 LLMs collaboratively authored, answered, and peer-evaluated tests on emotional intelligence; council rankings were **more separable and more stable, and closer to human evaluations than any individual LLM judge**.
- A model's **task success does not correlate with its judging ability**; consistent judging, neutral voting, and low contrarianism correlate with higher separability.
- Monte Carlo simulations: **larger councils are more robust to adversarial/random judges, with diminishing marginal utility**.
- Relevance: supports "a panel of mutually evaluating models can beat a single judge" for single-turn subjective evaluation. It is not about directing an executor mid-task. Karpathy's [llm-council](https://github.com/karpathy/llm-council) repo (24k+ stars) implements the same shape (independent answers → anonymized peer review → chairman synthesis); self-described as a "Saturday hack", not research.

## 4. Adjacent primary evidence on "strong oversees / advises, cheap acts" vs "strong plans once, cheap executes"

### Anthropic, Building Effective Agents (Dec 19, 2024)

[Engineering post](https://www.anthropic.com/engineering/building-effective-agents). Distills workflows (predefined code paths) vs agents (model-directed). Directly relevant points:

- Recommends the **simplest solution that works**; agentic systems trade latency and cost for task performance.
- **Orchestrator-workers** (a central LLM decomposes, delegates to workers, synthesizes) is recommended for complex tasks where subtasks cannot be predicted up front — the example given is coding changes across multiple files. This is plan-once's home turf.
- **Evaluator-optimizer** (generator + critic loop) is recommended when there are clear evaluation criteria and iterative refinement measurably helps — the critic-afterward pattern SWARMS uses.
- Routing example explicitly sends easy traffic to a cheap model and hard traffic to a stronger one — harness-decided, not worker-decided.

### Anthropic, Optimizing for cost and intelligence (platform docs, 2026)

The [guide](https://platform.claude.com/docs/en/about-claude/models/optimizing-for-cost-and-intelligence) measures both patterns against single-model baselines (all vendor-internal):

- **Orchestrator (frontier planner + cheap workers) pays in two measured cases only**: (1) insurance against cost-tail blowups on routine work (on a deliberately easy BrowseComp slice, a Fable coordinator + Sonnet worker cost about half of Fable solo on average and about a third at the 90th percentile, $12 vs $33); (2) work exceeding one context window (on a 21.6M-token corpus benchmark the coordinator cost 55% less than the frontier model solo at every effort setting, scoring 3-7 points below its best). **"In every such case measured"** where the work was one dependent chain or fit in one context, **the coordinator's model alone at lower effort came out ahead**.
- The decision rule the guide gives: does the work split into independent pieces (orchestrator) or is it one serial chain hard in spots (advisor)?
- Independent external work is cited by the guide for the same pattern ([ref 5 in its benchmark list]; the Cursor SQLite experiment recorded in @docs/research/cursor_model_orchestration_research_20260814.md reached the same conclusion).

### LLM monitors over cheaper executors (arXiv, verified)

These bear on the "SOTA model watches a cheap executor" design space:

- **How does information access affect LLM monitors' ability to detect sabotage?** ([arXiv 2601.21112](https://arxiv.org/abs/2601.21112)). Monitors often perform **better with less information** ("less-is-more effect"); an extract-and-evaluate hierarchy (one monitor isolates excerpts, another scores them) improved sabotage detection by 16.8 pp on BigCodeBench-Sabotage with a GPT-4.1-mini monitor; agents unaware of being monitored are far easier to catch; information-filtering techniques occupy the cost-performance Pareto frontier. Implication: monitor **design** (what the overseer sees) matters as much as monitor size.
- **Detecting High-Stakes Interactions with Activation Probes** ([arXiv 2506.10805](https://arxiv.org/abs/2506.10805), NeurIPS 2025). Probes match prompted or finetuned medium-LLM monitors at ~six orders of magnitude lower cost, motivating hierarchical monitoring where cheap filters escalate to expensive oversight.
- **CoT-Guard: Small Models for Strong Monitoring** ([arXiv 2605.12746](https://arxiv.org/abs/2605.12746)). Small (4B-8B) open models substantially underperform frontier monitors zero-shot: oversight quality is capability-bound, so "cheap monitor over strong actor" degrades the monitor, not the actor.

### METR monitor work (what actually exists there)

No METR paper or post titled "Large Language Monitors" exists (section 6). METR's real adjacent output: [Early Results on Monitorability in QA Settings](https://metr.org/notes/2025-10-06-early-results-on-monitorability-in-qa-settings/) (harder side tasks are more detectable; small models can learn to evade larger monitors) and [Early work on monitorability evaluations](https://metr.org/blog/2026-01-19-early-work-on-monitorability-evaluations/). These study monitors vs agents, not "small actor + frontier monitor" vs "single frontier agent" cost-quality tradeoffs.

## 5. Advisor-in-the-loop vs plan-once + cheap execution

| Dimension | Pull advisor (Anthropic tool) | Push advisor (omp / pi-omplike-advisor) | Plan-once + cheap execution (SWARMS today) |
| --- | --- | --- | --- |
| Token cost shape | Bulk at executor rates + advisor-rate consultations (~400-700 output tokens each, each call re-reading the full transcript, uncached by default) | Bulk at executor rates + advisor processes every turn-delta on its own context (self-compacting at 80%) | Planner tokens once + bulk at worker rates; planner never reads worker trajectories |
| Cost predictability | Governed by the consult rate, which varies by task and collapses at low executor effort; needs caps (`max_uses`) | Advisor tokens scale with trajectory length by construction; bounded by compaction and severity policy | High: fixed plan, per-leaf budgets, verifier runs at leaf boundaries |
| Error recovery | Mid-task, at the exact failure point (recurring errors, pre-completion check) — but only when consulted | Every turn in principle; review lags seconds, so concerns/blockers are held until reconfirmed | At gate/verify boundaries; a wrong decomposition is caught late, usually by the critic or tests |
| Task drift | Advisor reads the full live transcript each call, so drift is visible in principle — but only when consulted | Always visible to the advisor; staleness handled by hold-and-reconfirm | Plan rot over long horizons; requires the plan to be right up front |
| Wins when | Serial work hard in spots; wide executor/advisor capability gap; consult rate stays high (prompted/nudged) | Long serial sessions where the doer will not self-detect being stuck; advisor must be cheap enough to run every turn (community default: GLM 5.2, thinking low) | Divisible work, independent fan-out, corpora beyond one context window; predictable subtasks |
| Failure modes | Executor never asks (esp. at low effort) and scores below executor-alone; consultation latency on the critical path | Stale or mistimed advice; latency stalls (catch-up blocks 15-120s); advisor context overflow (mitigated by self-compaction) | Wrong decomposition discovered late; planner must compress all ambiguity before any worker token is spent |
| Evidence strength | Vendor-only (Anthropic internal evals, some single-run), consistent mechanism | Shipped in two independent implementations; zero published benchmarks | Vendor-only but two independent vendors (Anthropic orchestrator measurements + Cursor SQLite experiment), plus Building Effective Agents guidance |

### Synthesis with confidence levels

- **High confidence (vendor-measured, mechanism replicated across several Anthropic benchmarks):** decision-point consulting lifts a cheap executor on serial tasks, and the lift scales with the executor/advisor capability gap (Haiku gains a lot, Sonnet a little, frontier almost nothing).
- **High confidence:** the advisor's benefit is conditional on the executor actually consulting. This is the pattern's structural weakness: the cheap model must recognize it is stuck. Prompting and nudges partially fix it; effort reductions break it.
- **Medium confidence:** at narrow capability gaps or top-of-range quality targets, a single model with tuned effort matches the advisor pairing for similar money — Anthropic's own numbers (85.7% @$8.40 vs 83.4% @$8.20 vs 84.4% @$8.50) sit within single-run noise. Baseline any advisor design against one model at lower effort.
- **Medium confidence:** plan-once + cheap execution is the better default for divisible, fan-out, or beyond-one-context work; two independent vendors converge on this, and Anthropic measured the orchestrator losing to "coordinator's model alone at lower effort" on every single-chain case.
- **No evidence either way:** a direct head-to-head trial of "Claude Advisor vs plan-once cheap execute" on the same task set. It does not exist publicly. Anthropic positions the two as complementary (serial-hard-in-spots vs fan-out), which is the only defensible reading of the current evidence.
- **No evidence (but design convergence):** the push variant (omp's advisor, pi-omplike-advisor) eliminates the pull variant's measured weakness — the executor failing to ask — by reviewing every turn unconditionally, paying advisor tokens per turn and adding hold-and-reconfirm latency. Two independent implementations exist; neither publishes effectiveness numbers. Treat as complementary tooling, not a proven upgrade.
- **No evidence:** any "Optimal Model Power" claim. The metric never existed; "OMP" is the product name Oh My Pi (section 2). Do not use the metric framing to justify architecture.

## 6. Unverified / not found

- **OMP (resolved August 16)**: the first pass searched for an "Optimal Model Power" metric because no URL was given; the user's URLs identify OMP as the product **Oh My Pi (omp.sh)**. No metric by that name exists (arXiv API phrase search: 0; LMC full text: 0 occurrences) — the metric interpretation was a wrong inference, corrected in section 2.
- **omp advisor effectiveness**: no benchmarks published by omp or by the pi-omplike-advisor port; omp's quantitative claims cover harness/edit formats only ([The harness problem](https://blog.can.ac/2026/02/12/the-harness-problem/)). The advisor's value is publicly unmeasured.
- **arXiv 2505.24127** resolves to "Estimating dynamic transmission rates with a Black-Karasinski process in stochastic SIHR models" ([abs](https://arxiv.org/abs/2505.24127)) — not an LLM paper.
- **arXiv 2507.04473** resolves to "Tight Guarantees for Cut-Relative Survivable Network Design" ([abs](https://arxiv.org/abs/2507.04473)) — not METR, not monitors.
- **METR "Large Language Monitors"** (paper, arXiv id, or metr.org blog version): not found. Checked [metr.org/blog](https://metr.org/blog/), [metr.org/research](https://metr.org/research/), [metr.org/notes](https://metr.org/notes/), arXiv phrase searches, and web search.
- **Claude Advisor "announced ~January 2026"**: not found. The earliest primary source located is the April 9, 2026 blog post; the tool's version string (`advisor_20260301`) indicates March 2026. `anthropic.com/news/claude-advisor` returns 404 and no advisor page appears in the anthropic.com news sitemap; the announcement lives on claude.com/blog and the docs on code.claude.com / platform.claude.com.
- **"Claude advises Codex and third-party agents"**: not found in any official source. The advisor runs executor-side inside Anthropic's own API, Claude Code, and Claude Managed Agents, with Anthropic models as executor; no Codex or third-party-agent integration is documented anywhere I could verify.
- **Exact Sonnet+advisor deltas on BrowseComp and Terminal-Bench 2.0**: published only as charts in the blog post; not extractable as numbers.
- No Anthropic engineering-blog post specifically about the advisor was found; the closest engineering material is the [managed-agents post](https://www.anthropic.com/engineering/managed-agents) (not fetched; out of scope).

## 7. Implications for SWARMS

- Keep plan-once + cheap workers as the default for divisible DAGs. Both Anthropic's orchestrator measurements and the Cursor SQLite experiment (prior note) favor it for fan-out work, and SWARMS' DAG/wave scheduler is exactly the substrate it needs.
- The advisor pattern is not a replacement but an escape hatch: the evidence supports mid-task consultation at three specific moments — before committing to an approach, on a recurring error, and before declaring done. A SWARMS "consult" step available to workers maps cleanly onto these.
- Consult rate must be engineered, not assumed. Prompt for ~2-3 consults per task; consider a nudge after N turns for weak workers (Anthropic measured ~+7 pp on Haiku-class executors, no effect on Sonnet-class, slightly negative on Opus-class — calibrate by worker tier).
- The capability-gap rule matters for SWARMS specifically: advisor-style consulting pays when worker and planner models differ widely. With GLM 5.2 as both planner and worker, expect near-zero advisor benefit; it becomes interesting only on genuinely cheaper worker routes (mock, Flash-class, OpenAI-compat small models).
- Advisor calls re-read the full worker transcript uncached — bound them with per-task caps (SWARMS' provider-cap mechanism is the natural place) or filter what the advisor sees; the less-is-more monitor result (arXiv 2601.21112) says filtered excerpts often oversee better than full trajectories.
- Always baseline against (a) worker-only and (b) strong-model-at-lower-effort before shipping any advisor mode. Anthropic's own data shows effort tuning matching advisor pairings at the top of the range, and it is the cheaper experiment.
- SWARMS' critic-verifies-afterward stays justified (Anthropic's evaluator-optimizer guidance), but feed the critic structured artifacts and excerpts rather than raw trajectories where possible — monitor evidence favors information filtering.
- Near-continuous observation now has two shipped implementations (omp's advisor; pi-omplike-advisor) but still zero published effectiveness numbers, and its token cost scales with trajectory length by construction. Decision-point consulting (Anthropic-style) dominates it on cost with no measured quality disadvantage against it. Do not build continuous watching into SWARMS without a local A/B.
- If an advisor mode is built, the omp design is the better template for SWARMS than Anthropic's pull tool: SWARMS already persists per-turn worker state (task files, logs, status), so a Rust-side advisor role can read turn-deltas without touching worker prompts. Severity tiers (nit/concern/blocker) with hold-and-reconfirm delivery, and WATCHDOG.md-style project guidance, are cheap production-proven design choices worth copying.
- omp's catch-up blocks (15s→120s backoff) are a TUI-interaction compromise; SWARMS' equivalent is cheaper — run the advisor at wave boundaries or task-gate waits, where workers are already idle, avoiding omp's mid-turn stalls entirely.
- The port's default advisor (GLM 5.2, thinking low) matches SWARMS' existing planner/critic tier — an omp-style advisor over cheap worker routes reuses the role policy already configured; no new provider needed.

## Sources

- Can Bölük (can1357), omp / Oh My Pi: https://omp.sh/ ; README https://github.com/can1357/oh-my-pi ; npm https://www.npmjs.com/package/@oh-my-pi/pi-coding-agent ; "The harness problem" https://blog.can.ac/2026/02/12/the-harness-problem/
- pasky, pi-omplike-advisor: https://pi.dev/packages/pi-omplike-advisor ; https://github.com/pasky/pi-omplike-advisor
- Upstream Pi (omp's fork base): https://github.com/badlogic/pi-mono ; name-collision package (unrelated): https://registry.npmjs.org/oh-my-pi
- Anthropic, "The advisor strategy: Give agents an intelligence boost" (April 9, 2026): https://claude.com/blog/the-advisor-strategy
- Anthropic, advisor tool (Claude API docs, beta): https://platform.claude.com/docs/en/agents-and-tools/tool-use/advisor-tool
- Anthropic, "Escalate hard decisions with the advisor tool" (Claude Code docs): https://code.claude.com/docs/en/advisor
- Anthropic, "Optimizing for cost and intelligence" (platform docs): https://platform.claude.com/docs/en/about-claude/models/optimizing-for-cost-and-intelligence
- Anthropic, "Building effective agents" (engineering, Dec 19, 2024): https://www.anthropic.com/engineering/building-effective-agents
- Zhao, Plaza-del-Arco, Genchel, Cercas Curry, "Language Model Council: Democratically Benchmarking Foundation Models on Highly Subjective Tasks": https://arxiv.org/abs/2406.08598 (full text v4: https://arxiv.org/html/2406.08598v4)
- Karpathy, llm-council (GitHub): https://github.com/karpathy/llm-council
- "How does information access affect LLM monitors' ability to detect sabotage?": https://arxiv.org/abs/2601.21112
- "Detecting High-Stakes Interactions with Activation Probes": https://arxiv.org/abs/2506.10805
- "CoT-Guard: Small Models for Strong Monitoring": https://arxiv.org/abs/2605.12746
- METR, "Early Results on Monitorability in QA Settings": https://metr.org/notes/2025-10-06-early-results-on-monitorability-in-qa-settings/
- METR, "Early work on monitorability evaluations": https://metr.org/blog/2026-01-19-early-work-on-monitorability-evaluations/
- Wrong-ID evidence: https://arxiv.org/abs/2505.24127 and https://arxiv.org/abs/2507.04473 ; arXiv API phrase searches via https://export.arxiv.org/api/query ; Anthropic news sitemap https://www.anthropic.com/sitemap.xml and code.claude.com sitemap https://code.claude.com/sitemap.xml
- Prior internal notes: @docs/research/model_routing_deep_dive_20260815.md, @docs/research/cursor_model_orchestration_research_20260814.md
