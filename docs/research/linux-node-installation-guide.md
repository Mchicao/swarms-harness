# Linux Worker Node — Installation & Validation Guide (SWARMS)

Research artifact for standing up and validating a **Linux worker node** that runs
the SWARMS coordinator. This guide is written from repository facts plus verified
local `--help`/`doctor` output. It does **not** assume a remote node was
provisioned; it describes how to prepare and validate a node you already control.

> **Scope note.** SWARMS is local-first and offline by default. Nothing in this
> guide provisions a remote machine, opens firewall ports for inbound traffic, or
> enables a paid provider. Every "real provider" step is opt-in and labeled.

---

## 0. How to read this guide (evidence legend)

| Marker | Meaning |
|---|---|
| **[VERIFIED]** | Stated in repository docs/source or reproduced via local command output on the author's machine. |
| **[RECOMMENDATION]** | Not in repository docs. A sane default the author suggests; adapt to your environment. |
| **[NEEDS CONFIRMATION]** | Depends on your infrastructure, provider accounts, or policy. Confirm before running. |

Source references use `path:line` so they can be opened directly.

---

## 1. Prerequisites

### 1.1 Operating system & base tools **[VERIFIED]**

- Linux is an officially supported platform. `docs/PLATFORM.md:9` lists
  "Linux with Python 3.10+ and Git"; the Rust runtime is cross-platform per
  `docs/RUST_RUNTIME.md:3-6`.
- CI exercises Linux directly: `.github/workflows/ci.yml:22` builds and tests on
  `ubuntu-latest`, and the Python legacy job runs on `ubuntu-latest`
  (`ci.yml:47`).
- Required base tools: `git`.

```bash
# Q-01: base tool check on the Linux node
git --version
uname -a
```

### 1.2 Rust toolchain (the public runtime) **[VERIFIED]**

The **Rust binary is the sole public runtime and is self-contained — no Python
dependency** (`README.md:94-98`, `AGENTS.md` "Prime Directive"). CI installs a
**stable** toolchain via `dtolnay/rust-toolchain@stable` (`ci.yml:25-27`). No
`rust-toolchain`/`rust-toolchain.toml` channel file is pinned in the repo, so
stable is the verified target. Edition is 2021 (`rust/Cargo.toml:4`). Runtime
dependencies are minimal: `serde`, `serde_json`, `ureq` (`rust/Cargo.toml:16-19`).

```bash
# Q-02: install a stable Rust toolchain (use the official installer or your distro)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain stable
# rustup is the installer's own recommendation; the repo itself does not bundle an installer.
source "$HOME/.cargo/env"
rustc --version && cargo --version
```

> **[NEEDS CONFIRMATION]** Installing toolchains system-wide may require root or
> an approved package manager in your environment. `rustup` installs per-user by
> default and needs no root.

### 1.3 Python 3.10+ and uv — **only for the legacy tooling** **[VERIFIED]**

Python is **not required to run the coordinator**. It is only needed for the
legacy benchmark/telemetry scripts (`docs/RUST_RUNTIME.md:147-151`,
`AGENTS.md`). If you intend to use those:

- `requires-python = ">=3.10"` (`pyproject.toml:10`); CI uses **3.11** for the
  legacy job (`ci.yml:53`).
- The repo uses **uv** as its Python package manager: `uv.lock` is present at the
  repo root (verified).

```bash
# Q-03: optional — legacy Python tooling only
python3 --version          # expect 3.10, 3.11, or 3.12
# uv (presence of uv.lock in the repo confirms uv is the manager):
pip install uv             # or follow the upstream uv install docs
```

**[RECOMMENDATION]** If you only want the coordinator (mock or real providers),
skip Python entirely and use the Rust path in §3–§5.

---

## 2. Repository checkout **[VERIFIED]**

```bash
# Q-04: clone to the node (URL from pyproject.toml:39)
git clone https://github.com/Mchicao/swarms-harness.git ~/swarms
cd ~/swarms
```

The committed default config is **offline-only**: only the `mock` provider is
enabled, there are no API keys, and the mock provider makes no network calls
(`docs/SECURITY.md:6-13`, `config/swarm_router.json:30-55`). A fresh clone is
therefore safe to build and run.

---

## 3. Build the Rust runtime **[VERIFIED]**

All commands below are the documented invocation form (`README.md:99-104`,
`docs/RUST_RUNTIME.md:10-15`, `AGENTS.md`):

```bash
# Q-05: from the repo root, build + run the coordinator
cargo run --manifest-path rust/Cargo.toml -- doctor
```

For a release-optimized binary (what CI builds — `ci.yml:34-35`):

```bash
# Q-06: release build, then invoke the binary directly
cargo build --release --manifest-path rust/Cargo.toml
./rust/target/release/swarms-rs doctor
```

For CI or offline work, `--offline` works after Cargo has built dependencies once
(`docs/RUST_RUNTIME.md:17`):

```bash
# Q-07: subsequent offline builds
cargo run --manifest-path rust/Cargo.toml --offline -- doctor
```

> **Note on `install.sh`.** The repo's `install.sh` (`install.sh:14-26`) creates a
> launcher at `$HOME/.local/bin/swarm` that runs `python scripts/swarm.py` — i.e.
> the **legacy Python** entry point, not the Rust runtime. For a Rust-based worker
> node, prefer the `cargo`/binary invocations above. Use `install.sh --uninstall`
> (`install.sh:8-12`) to remove that launcher.

---

## 4. Health check: `doctor` **[VERIFIED]**

`doctor` is the primary offline health check and the **default subcommand when no
arguments are passed** (`install.sh:19-21`, `rust/src/cli.rs:24`). It verifies the
coordinator is available, the router loads, the mock provider is offline-safe, and
the example plan passes static review.

Verified CLI surface (`rust/src/cli.rs:20-94`):

- Subcommands: `doctor | review | dry-run | run`.
- Flags (for `review`/`dry-run`/`run`): `--plan <file>` (required),
  `--run-id <id>`, `--force`, `--global-max-concurrency <N>`,
  `--provider-cap route=count` (repeatable), `--router-config <path>`.
- There is **no `--help` flag** — passing it returns `unknown argument: --help`
  (verified, exit code 2). The usage string is
  `usage: swarms-rs <doctor|review|dry-run|run> --plan <file>` (`cli.rs:24`).
- `--run-id` must be ≤128 chars of `[A-Za-z0-9._-]` (`cli.rs:114-120`).

Verified `doctor` output captured on the author's machine (Windows; on Linux the
platform string changes to `linux`, the rest is identical in shape):

```text
[OK] Rust coordinator available on windows
[OK] router loaded (14 providers)
[OK] mock provider enabled (offline-safe)
[OK] example plan review passed (4 tasks)
EXIT=0
```

```bash
# Q-08: the canonical health check (exit 0 = healthy)
cargo run --manifest-path rust/Cargo.toml -- doctor
echo "doctor exit: $?"
```

> **[VERIFIED]** `[WARN]` lines about enabled real providers or unknown wrappers
> reflect whatever your **local** config enables (see §6). On a clean clone with
> only `mock`, doctor reports no provider warnings. Treat any non-zero exit as a
> blocker before proceeding.

---

## 5. Local router/provider configuration **without secrets** **[VERIFIED]**

SWARMS has two config layers (`docs/CONFIG.md:3-14`, `rust/src/config.rs:35-55`):

1. `config/swarm_router.json` — committed, offline-only default (mock only).
2. `config/swarm_router.local.json` — **gitignored**, automatically **deep-merged**
   on top of the default if present (`config.rs:37-40`). This is where any real
   provider would go.

The merge is recursive: object keys merge; scalars overwrite (`config.rs:16-31`).
The `--router-config <path>` flag overrides the **base** file path
(`cli.rs:68-74`, `cli.rs:122-128`).

**Secret-free setup (mock only) — recommended starting point:**

```bash
# Q-09: confirm the committed default is mock-only; do NOT create a local file yet
#       doctor already proves mock works. No secrets are needed for mock.
cargo run --manifest-path rust/Cargo.toml -- doctor
```

**To prepare (but keep disabled) the local config shape for later** — copy the
example and keep all real providers `enabled: false`:

```bash
# Q-10: stage a private local config from the template (docs/CONFIG.md:10-12)
cp config/swarm_router.local.example.json config/swarm_router.local.json
# The example enables several real providers (see file). To stay secret-free,
# set every non-mock provider "enabled": false before any run.
```

**[VERIFIED]** What the repo says about secrets:

- `.env.example` shows only commented-out, optional keys (`ZAI_API_KEY`,
  `GOOGLE_API_KEY`) and states "SWARMS does not need secrets for the default mock
  provider" (`.env.example:1-5`).
- `docs/SECURITY.md:14-25` lists what must never be committed: `.env`,
  `config/*.local.json`, auth files, OAuth tokens, telemetry/reports, `.agent/`.
  The `.gitignore` covers these (verified: `config/*.local.json`, `.env*`,
  `.agent/`, `rust/target/`, `*.log`, `*.jsonl` are all ignored).

**[NEEDS CONFIRMATION]** Enabling any real route requires provider credentials you
own (API key, CLI login, or plan quota). The repo ships none. Do not enable a
provider on the node until you have confirmed the account, cost, and data-handling
implications. HY3 provider routes and their requirements are documented in
`README.md:15-43` and `docs/CONFIG.md:36-49`.

---

## 6. Offline mock validation **[VERIFIED]**

The mock path runs end-to-end with no network and no credentials. These are the
exact commands CI uses on Linux (`ci.yml:36-43`) and the README documents
(`README.md:99-104`):

```bash
# Q-11: static review of the bundled example plan (4 tasks)
cargo run --manifest-path rust/Cargo.toml -- review --plan docs/workflow_plan_example.json

# Q-12: dry-run (no execution) with a fixed run-id
cargo run --manifest-path rust/Cargo.toml -- dry-run --plan docs/workflow_plan_example.json --force --run-id node-dry

# Q-13: full mock end-to-end with explicit caps (offline, no secrets)
cargo run --manifest-path rust/Cargo.toml -- run \
  --plan docs/workflow_plan_example.json --force \
  --run-id node-mock-e2e \
  --global-max-concurrency 3 \
  --provider-cap mock=3
```

**[VERIFIED]** Run state, logs, worker prompts, task state, lifecycle events,
result JSON, and the final report are written under
`.agent/swarm/runs/<run_id>/` (`README.md:142`). This directory is gitignored
(`docs/SECURITY.md:14-25`), so mock runs do not pollute the repo.

**Success criteria:** review and dry-run exit 0; the `run` produces a report under
`.agent/swarm/runs/node-mock-e2e/` with all tasks completed via the `mock` route
(telemetry fields are `0` for mock — `docs/RUST_RUNTIME.md:138-145`).

---

## 7. Supervised execution (systemd) **[RECOMMENDATION]**

The repository does **not** ship systemd units or any supervisor config. The
following is a template you must review. SWARMS runs as a normal process; mock
runs need no network. Keep `Type=oneshot` for bounded plan runs.

```ini
# /etc/systemd/system/swarms-mock-run.service  -- [RECOMMENDATION], not from the repo
[Unit]
Description=SWARMS offline mock plan run
After=network-online.target
Wants=network-online.target

[Service]
Type=oneshot
# Run as an unprivileged service user you create for SWARMS:
User=swarms
Group=swarms
WorkingDirectory=/home/swarms/swarms
# Use the release binary built in Q-06; mock only, no secrets, bounded caps:
ExecStart=/home/swarms/swarms/rust/target/release/swarms-rs run \
  --plan docs/workflow_plan_example.json --force \
  --run-id svc-mock \
  --global-max-concurrency 3 \
  --provider-cap mock=3
# Hardening (does not replace SWARMS's own review; see §11):
NoNewPrivileges=true
PrivateTmp=true
ProtectSystem=strict
ReadWritePaths=/home/swarms/swarms/.agent
# State lives under .agent/swarm/runs/<run_id>/ (README.md:142)
StandardOutput=append:/var/log/swarms/swarms.log
StandardError=append:/var/log/swarms/swarms.log

[Install]
WantedBy=multi-user.target
```

```bash
# Q-14: [RECOMMENDATION] install + run the unit (requires root; confirm with your admin)
sudo mkdir -p /var/log/swarms && sudo chown swarms:swarms /var/log/swarms
sudo systemctl daemon-reload
sudo systemctl start swarms-mock-run.service
sudo systemctl status swarms-mock-run.service --no-pager
```

> **[NEEDS CONFIRMATION]** systemd hardening directives and the service user must
> match your site policy. `tools_policy=none` in SWARMS is **not** an OS sandbox
> (`docs/SECURITY.md:28-29`); the systemd hardening above is the OS-level layer.
> Do not enable real providers in a system service until §5/§11 are satisfied.

---

## 8. Health checks (scheduled) **[RECOMMENDATION]**

Wrap `doctor` in a timer; it is the documented offline health check (§4).

```bash
# Q-15: [RECOMMENDATION] a trivial cron wrapper
#   crontab -e
#   */15 * * * * cd /home/swarms/swarms && ./rust/target/release/swarms-rs doctor >> /var/log/swarms/doctor.log 2>&1
```

**[RECOMMENDATION]** Alert on non-zero `doctor` exit or on any `[WARN]`/`[ERROR]`
line. Treat `[WARN] real providers enabled: ...` as expected only if you
intentionally enabled those routes in the local config.

---

## 9. Network & VPN assumptions **[VERIFIED]** / **[RECOMMENDATION]**

**[VERIFIED]** From the repository:

- SWARMS is **local-first**; the default clone makes **no network calls** for
  model execution — mock is fully offline (`README.md:5-7`,
  `docs/SECURITY.md:6-13`, mock `strengths: ["offline", ...]` in
  `config/swarm_router.json:53`).
- The only HTTP client in the runtime is `ureq` (`rust/Cargo.toml:19`), used by
  the OpenAI-compatible adapter. Real provider traffic is **outbound only**
  (HTTPS to the provider/gateway you configure, or a local LiteLLM endpoint —
  `docs/CONFIG.md:41`). There is **no documented inbound listener** in the
  coordinator.
- The optional quota guard reads a **local** snapshot file
  (`quota_policy.snapshot_path`, `docs/RUST_RUNTIME.md:84-112`); no external
  network is required for it.

**[RECOMMENDATION]** (the repo gives no VPN guidance):

- No VPN is required for mock validation. If you enable real providers, a VPN is
  between you and your provider/gateway; SWARMS does not manage it.
- If the node is remote, use your standard SSH/VPN access for administration.
  **Do not** expose the SWARMS working directory or `.agent/` state over a network
  share without access controls.
- **[NEEDS CONFIRMATION]** Confirm your provider's egress requirements (endpoints,
  TLS, rate limits) before enabling a real route.

---

## 10. Logs **[VERIFIED]** / **[RECOMMENDATION]**

**[VERIFIED]:**

- Per-run artifacts (prompts, logs, task state, lifecycle events, result JSON,
  final report) live under `.agent/swarm/runs/<run_id>/` (`README.md:142`).
- The runtime normalizes usage into reports; missing usage is marked `"missing"`,
  never fabricated as zeros (`docs/RUST_RUNTIME.md:138-145`).

**[RECOMMENDATION]:**

- For systemd, stdout/stderr go to journald (or the file in the unit above). Query
  with `journalctl -u swarms-mock-run.service`.
- Add log rotation (e.g., `logrotate`) for `/var/log/swarms/*.log` and prune old
  `.agent/swarm/runs/` directories on a schedule you choose.

---

## 11. Security boundaries **[VERIFIED]**

From `docs/SECURITY.md`:

- "Treat every enabled provider as code execution with file access"
  (`SECURITY.md:3`).
- Defaults are offline-only: no API keys, no paid providers, no network calls from
  mock, no real model execution in CI (`SECURITY.md:6-13`).
- `tools_policy=none` disables adapters' auto-approval flags but is **not an
  operating-system sandbox** for every external CLI; artifact paths are statically
  reviewed but not yet enforced against the final filesystem diff
  (`SECURITY.md:28-29`). → Review real-project runs.
- Expensive/scarce models must be disabled by default and used only through
  explicit routing (`SECURITY.md:32-34`).
- Never commit: `.env`, `config/*.local.json`, auth files, OAuth tokens,
  telemetry/reports, `.agent/`, worktrees, agent logs/prompts (`SECURITY.md:14-25`).

**Practical boundaries for the node:**

- Keep the node on **mock** until you have a reason to enable a real provider.
- Store any credentials in the service user's environment or a secrets manager —
  never in git. `.gitignore` already excludes `.env*` and `config/*.local.json`.
- Prefer an unprivileged service user (§7) and read-only repo checkout, with
  writes limited to `.agent/`.

---

## 12. Rollback & recovery **[VERIFIED]** / **[RECOMMENDATION]**

**[VERIFIED]:**

- A run can be **resumed without `--force`**: completed tasks are preserved;
  interrupted tasks (`pending`/`queued`/`in_progress`) reset to `pending` and
  re-execute; blocked tasks stay blocked (`docs/RUST_RUNTIME.md:131-135`).
- `--force` starts a fresh run for the given `--run-id`.
- To discard a run entirely, remove its directory under `.agent/swarm/runs/`.

```bash
# Q-16: [VERIFIED] resume after an interruption (no --force)
cargo run --manifest-path rust/Cargo.toml -- run --plan docs/workflow_plan_example.json --run-id node-mock-e2e

# Q-17: [VERIFIED] discard one run's state
rm -rf .agent/swarm/runs/node-mock-e2e
```

**[RECOMMENDATION]:**

```bash
# Q-18: full local rollback to the clean committed default
rm -f config/swarm_router.local.json   # removes any private overlay (gitignored)
git checkout -- .                      # discard tracked working-tree changes (confirm first)
git clean -xfd .agent                  # remove run state (untracked)
# Re-run doctor to confirm a clean baseline:
cargo run --manifest-path rust/Cargo.toml -- doctor
```

> **[NEEDS CONFIRMATION]** `git checkout -- .` and `git clean -xfd` are
> destructive. Review `git status` first and confirm no private files will be lost.

---

## 13. Node validation checklist

Run these in order; each should be green before moving on.

1. **Toolchain** — `rustc --version && cargo --version` (§1.2). *(Python/uv only
   if you need legacy scripts — §1.3.)*
2. **Checkout** — clone and `cd` into the repo (§2).
3. **Build** — `cargo build --release --manifest-path rust/Cargo.toml` (§3).
4. **Doctor** — `cargo run --manifest-path rust/Cargo.toml -- doctor` → exit 0
   (§4).
5. **Review** — `... -- review --plan docs/workflow_plan_example.json` → exit 0
   (§6).
6. **Dry-run** — `... -- dry-run --plan docs/workflow_plan_example.json --force
   --run-id node-dry` → exit 0 (§6).
7. **Mock E2E** — `... -- run --plan docs/workflow_plan_example.json --force
   --run-id node-mock-e2e --global-max-concurrency 3 --provider-cap mock=3` →
   report under `.agent/swarm/runs/node-mock-e2e/`, all tasks complete (§6).

If all seven pass, the node is ready for **offline/mock** work. Enabling real
providers is an additional, opt-in step gated by §5 and §11.

---

## 14. Items requiring user confirmation (blockers before "done")

- **[NEEDS CONFIRMATION]** Is this node remote? This guide does not provision one.
  You must provide SSH/VPN access and confirm the host exists.
- **[NEEDS CONFIRMATION]** Any real provider enablement (keys, CLI logins, paid
  quotas). The repo ships none and this guide enables none.
- **[NEEDS CONFIRMATION]** systemd service user, hardening directives, and
  logrotate policy must match your site security baseline (§7, §8, §10).
- **[NEEDS CONFIRMATION]** VPN topology and provider egress endpoints when real
  routes are enabled (§9).
- **[RECOMMENDATION]** The repo has **no** `rust/README.md` (verified — only
  `rust/Cargo.toml` and `rust/src/` exist). For Rust architecture, use
  `docs/RUST_RUNTIME.md` instead.

---

## 15. Source references (verified)

- `README.md:5-7, 15-43, 94-104, 109-126, 142, 184-192` — runtime, providers,
  quick start, verification.
- `docs/RUST_RUNTIME.md:3-17, 84-112, 131-145, 147-151` — cross-platform build,
  quota guard, resume, telemetry, Python-is-legacy.
- `docs/CONFIG.md:3-49` — two-layer config, real-provider notes.
- `docs/SECURITY.md:3-34` — offline defaults, do-not-commit, provider safety.
- `docs/PLATFORM.md:6-21` — supported platforms.
- `.github/workflows/ci.yml:22-43` — Linux CI matrix + mock E2E.
- `rust/Cargo.toml:1-19` — edition 2021, deps (serde/serde_json/ureq).
- `rust/src/cli.rs:20-128` — exact subcommands/flags, run-id rules, `--router-config`.
- `rust/src/config.rs:16-55` — local-overlay deep-merge, router loading.
- `install.sh:8-26` — Python launcher (legacy), uninstall.
- `pyproject.toml:10, 29-36, 39` — Python ≥3.10, extras, repo URL.
- `.env.example:1-5` — no secrets needed for mock.
- `.gitignore` — `.env*`, `config/*.local.json`, `.agent/`, `rust/target/`,
  `*.log`, `*.jsonl` ignored (verified by reading the file).
