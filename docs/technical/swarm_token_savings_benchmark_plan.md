# Plan: SWARM Token-Savings Benchmark & Telemetry Instrumentation (V1.1)

This plan integrates the review feedback from Codex GPT 5.5 High to verify the token cost performance of the SWARM Engine.

---

## 🎯 Primary Goal
Implement a benchmark to validate that the `SWARM` engine successfully optimizes expensive coordinator token consumption ($\le 10\%$) while keeping total execution costs economically viable ($\le 50\%$) compared to baseline runs.

### ⚠️ Metrics Suite
The benchmark will calculate:
1. **Coordinator Token Reduction Ratio:**
   $$\text{CTR} = \frac{\text{Coordinator Expensive Tokens (With Swarm)}}{\text{Baseline Expensive Tokens (Without Swarm)}} \le 0.10$$
2. **Total Cost Ratio:**
   $$\text{TCR} = \frac{\text{Total Swarm Costs (Coordinator + Workers + Overhead)}}{\text{Total Baseline Costs}} \le 0.50$$
3. **Total Token Amplification:**
   $$\text{TTA} = \frac{\text{Total Swarm Tokens}}{\text{Baseline Tokens}}$$
4. **Functional Pass Rate & Cost per Resolved Task**.

---

## 🛠️ Telemetry Specifications
We will record telemetry into a central append-only JSONL file (`.agent/traces/telemetry.jsonl`) with the following format:

```json
{
  "schema_version": "1.0",
  "run_id": "uuid",
  "benchmark_id": "uuid",
  "phase": "baseline|swarm|watcher|retry|goal_eval",
  "provider": "codex|agy|zai|kilo|claude",
  "model": "string",
  "role": "coordinator|worker|overhead",
  "task_id": "string",
  "input_tokens": 0,
  "cache_read_tokens": 0,
  "cache_write_tokens": 0,
  "output_tokens": 0,
  "reasoning_tokens": 0,
  "usage_source": "api_reported|cli_reported|estimated|missing",
  "success": true,
  "started_at": "ISO-Timestamp",
  "ended_at": "ISO-Timestamp"
}
```

### Discovery & Integration Rules
1. **Codex CLI (`gpt-5.5-codex`):** Parse JSON logs from `.agent_codex_out.txt`.
2. **Antigravity CLI (`agy`):** Check if logs/outputs contain token usage metadata, else fall back to tokenizer estimation. *Note: `agy` also supports invoking Claude Opus 4.6 under a highly limited capacity; when active, telemetry logs must map and estimate this model's pricing accordingly.*
3. **Z.AI Coding Plan (`glm-5.2`):** Handle dict-like vs object-like usage objects safely:
    ```python
    details = getattr(usage, "prompt_tokens_details", None)
    cached = details.get("cached_tokens", 0) if isinstance(details, dict) else (getattr(details, "cached_tokens", 0) if details else 0)
    ```
4. **Kilo Models & Swarm Overhead:** Capture tokens consumed by `kilo` runs, Watcher AI Review, Paramedic Retries, and Goal Evaluators, classifying them as `overhead`.

---

## 🏁 Benchmark Execution Flow
1. **Capsule Pagination:** Parse `swebench_pallets_flask_tasks.json` to load lightweight "Task Capsules" one by one to avoid context limit issues.
2. **Strict Baselines:** Execute a baseline coordinator run under identical repository checkout state, timeouts, and success criteria as the swarm run.
3. **Swarm Run:** Deploy parallel workers to resolve the tasks.
4. **Reporting:** Generate a dashboard/summary outputting CTR, TCR, TTA, and functional success rates.
