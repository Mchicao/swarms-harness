# Security

SWARMS is a local orchestration tool that launches coding agents in the current workspace. Treat every enabled provider as code execution with file access.

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

Only enable providers you understand. Some CLIs can request broad file access or execute commands. `tools_policy=none` avoids the adapters' auto-approval flags, but it is not an operating-system sandbox for every external CLI. Artifact paths are statically reviewed and included in prompts; they are not yet enforced against the final filesystem diff. Review real project runs.

## Expensive Model Protection

The primary product objective is to save scarce and expensive model quota. Strong models should be disabled by default, protected by high scarcity scores, and used only through explicit routing or clearly configured roles.
