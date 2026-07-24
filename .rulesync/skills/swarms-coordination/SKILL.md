---
name: swarms-coordination
description: "Coordinate durable, observable multi-agent workflows through SWARMS"
targets: ["*"]
---

# SWARMS Coordination

Before delegating, inspect the workflow contract, workspace status, existing
run state, available provider routes, and required local validation commands.
Use one coordinator process per workspace. Treat completed task artifacts as
evidence only after their declared verification passes.

When a task is waiting on a provider wrapper after a terminal completion event,
collect concrete evidence before recovery. Do not use arbitrary execution
timeouts as a substitute for checking real worker progress.

For migration or DataVIZ work, keep extraction, provenance, semantic modeling,
presentation, and SaaS concerns as explicit contracts with testable artifacts.
