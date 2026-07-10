# TranSafeTicket + SWARMS lessons (2026-07-10)

## Git integration

Two feature branches both descended from `main`; neither was an ancestor of the other, even though the development tree contained copies of SaaS-ready files. Use `merge-base`, `merge-base --is-ancestor`, and left/right counts before choosing strategy.

A safe pattern was:

1. create `integration/...` from the intended base branch;
2. merge the development branch with `--no-ff`;
3. keep infrastructure semantics from the base where appropriate (Docker service-to-service DB host was `db`, not `localhost`);
4. take the development UI variants only where they carried the intended feature work;
5. run conflict-marker search, frontend build, backend compile/lint, and `git diff --check` before the merge commit.

## Private fetch without leaking a token

The repository `.env` was already ignored and held a company GitHub token. A temporary `GIT_ASKPASS` script read the token from an environment variable, returned `x-access-token` as username, and was deleted after `git fetch`. The token never entered the remote URL or output.

## Model/runtime lessons

- Gemini Flash completed a bounded frontend audit quickly, but suggested calling timer `start` every 30 seconds as heartbeat. That is semantically wrong when `start` rejects a second active timer. Fast-model recommendations need API-contract review.
- GLM 5.2 received a repository-wide backend audit and timed out at 300 seconds. The durable fix is both: split by domain and budget at least 600 seconds, with a longer outer orchestrator timeout.
- A Codex worker discarded its `model` argument and hardcoded the wrong effort key. Workers must forward `--model`; verify the effective runtime banner. On the observed Codex setup the correct key was `model_reasoning_effort`, not `reasoning_effort`.
- OpenCode approval flags are version-dependent. The observed CLI exposed `--auto`; another skill version expected `--dangerously-skip-permissions`. Always probe `opencode run --help`.

## Runtime isolation caveat

The current deterministic runtime isolates prompts, status files, logs and result directories, but does not automatically create a target-repository worktree for every task. Two workers can still collide when their prompts point to the same external repository. Until true worktree provisioning is implemented, parallel writable tasks must have disjoint file ownership; otherwise serialize them with explicit `needs` dependencies.

## Audit quality gates

README claims were not sufficient evidence. Direct inspection found missing tenant isolation, broad unauthenticated routers, incomplete state transitions, no real Alembic migrations, partial pagination, incorrect project/time aggregation and no tests. Require file/line evidence and deterministic checks before declaring SPEC compliance.
