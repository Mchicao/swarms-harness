# Security Policy

SWARMS is a local orchestration tool for coding agents. Treat every enabled real provider as code execution with file access.

## Supported Versions

The public alpha supports only the current `main` branch after publication.

## Reporting A Vulnerability

Open a private security advisory on GitHub when the repository is public, or contact the maintainer through the GitHub profile at https://github.com/Mchicao.

Please include:

- affected file or command;
- expected impact;
- reproduction steps that do not expose secrets;
- whether real provider execution is required.

## Secret Handling

Never include secrets in issues, pull requests, logs, test fixtures, or examples.

Ignored local-only files include:

- `.env`
- `config/*.local.json`
- `.agent/`
- worktrees
- generated prompts, logs, reports, traces, and telemetry

## Default Safety Boundary

The committed configuration is offline-only. Any real provider adapter must require explicit local configuration and must not run in CI by default.
