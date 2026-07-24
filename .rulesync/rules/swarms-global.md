---
root: true
targets: ["*"]
description: "Shared operating rules for SWARMS-managed agent work"
globs: ["**/*"]
---

# SWARMS Global Operating Rules

- Treat the Rust SWARMS runtime as the only workflow coordinator. Do not start
  duplicate workflows for the same workspace.
- Work from a declared workflow contract. Preserve dependency edges, provider
  caps, artifact paths, and verification commands.
- Use English for source code, code comments, docstrings, tests, plans, and
  worker output unless an input artifact requires a localized literal.
- Keep source-derived work generic. A fixture may prove behavior, but must not
  become a hard-coded customer exception.
- Do not expose credentials, tokens, private prompts, raw provider transcripts,
  or generated run state in source control or public reports.
- Validate each implementation with the project-native checks before claiming
  completion. Report the command and result, not an unsupported success claim.
- Use visible Herd panes for worker observability when the runtime requests a
  Herd terminal backend. Do not leave completed worker panes open.

# Data Product Delivery Rules

- Tableau and Power BI source artifacts must be converted to versioned,
  provenance-preserving intermediate representations before presentation work.
- Treat numerical parity, row-level provenance, tenant isolation, and
  deployment controls as acceptance gates. A rendered dashboard alone is not
  proof of a saleable migration.
- Keep validation programmatic unless a human-facing visual check is explicitly
  required. Never claim visual fidelity from serialized report metadata alone.
