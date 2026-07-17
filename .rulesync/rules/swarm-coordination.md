---
root: true
targets:
  - agentsmd
  - claudecode
  - codexcli
  - geminicli
  - opencode
  - antigravity
---

# SWARMS coordinated-agent policy

- Treat the user-defined worker, concurrency, depth, child, round, provider,
  tool, time, token, and cost budgets as hard limits.
- Do not create subagents unless the assigned task explicitly allows it.
- Never build recursive or self-replicating agent trees. Prefer the smallest
  number of independent workers that can materially improve the result.
- A worker may propose additional work to the coordinator, but must not bypass
  the shared task graph or provider limits.
- Preserve task ownership. Do not revert or overwrite another worker's edits.
- Report completion only with an existing artifact and fresh verification.
