# Security

SWARMS is a local orchestration tool that launches coding agents in Git worktrees. Treat every enabled provider as code execution with file access.

## Defaults

The committed default configuration is offline-only:

- no API keys;
- no paid providers;
- no network calls from the mock provider;
- no real model execution in CI.

## Do Not Commit

- `.env`
- `config/*.local.json`
- auth files such as `auth.json`
- OAuth tokens
- generated telemetry and reports
- `.agent/`
- worktrees
- agent logs and prompts

The `.gitignore` covers these by default.

## Provider Safety

Only enable providers you understand. Some CLIs can request broad file access or execute commands. For benchmarks, SWARMS rejects changes outside allowed paths, but real project runs should still be reviewed.

## Expensive Model Protection

The primary product objective is to save scarce and expensive model quota. Strong models should be disabled by default, protected by high scarcity scores, and used only through explicit routing or clearly configured roles.
