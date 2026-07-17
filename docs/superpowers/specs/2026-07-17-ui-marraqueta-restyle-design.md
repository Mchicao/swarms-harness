# UI Restyle — Marraqueta Miga Cálida

**Date:** 2026-07-17
**Status:** Approved design, pending implementation plan
**Author:** ZCode (with Matías Chicao)
**Scope:** Visual refresh of the `swarms-ui` native egui panel. No architecture change, no new runtime dependencies.

---

## 1. Context

SWARMS ships a native egui/eframe 0.32 + Glow observer UI for workflow runs. The stack choice is settled in `docs/UI_RUNTIME_EVALUATION.md` with reproducible benchmarks and is not under review. This spec concerns only the **visual layer**: colors, typography, component styling, and the internal organization of styling code.

### Current state (from code audit)

- `rust/src/ui_main.rs` is a single 3117-line file holding both the pure serde/std contract model and the egui renderer.
- Colors are scattered across **four** locations:
  1. `apply_theme()` (~15 lines): `Visuals::dark()` + hardcoded palette
  2. `accent()` / `muted()` helpers
  3. `status_color(status, stale)` semantic mapper
  4. Inline `Color32::from_rgb` literals in the DAG painter
- No design tokens. No custom fonts (`default_fonts` feature only). No icons.
- Layout: top header (46px) + bottom footer + left runs panel + right task detail + center 4-tab area (Overview DAG, Tasks tree, Activity, Resources).
- Only 2 commits touch these files; the surface is "as first landed".

### Problem

The current UI is functional but visually flat and internally inconsistent (colors duplicated in 4 places, no typographic hierarchy, no cohesive component system). A restyle is wanted **without** changing architecture, runtime deps, or the read-only observer contract.

### Hard constraints (must not violate)

1. **Read-only observer.** Only writes are steering + project-local Skillshare sync (STATE_CONTRACT, SWARM_UI).
2. **No new runtime/dependency.** No WebView, no WGPU, no persistence feature, no images feature. Everything stays inside egui's built-in style system. (UI_RUNTIME_EVALUATION)
3. **No continuous repaint.** No animations driven by `request_repaint` loops. The CPU-risk the design explicitly forbids.
4. **No fabricated data.** Status colors must follow the contract's status derivation. `stale` is a label, not a status. Don't fake steerability or subagent fan-out.
5. **Sanitization stays contractual** (path relativization, 1000-char error cap, token scrubbing).
6. **No SLOP.** Per https://impeccable.style/slop/ — avoid side-tab accent borders, glowing accents, Inter-everywhere, numbered section markers, ghost-card hairline+shadow, gradient text, glassmorphism, decorative shadows.

---

## 2. Design decisions

### 2.1 Visual identity: Marraqueta Miga Cálida (light theme)

A warm cream/bread palette derived from the maintainer's existing landing-page identity (marraqueta = Chilean bread). Light background (the crumb), dark brown text, golden-brown accent.

**Design tokens (Palette E2 — Miga Cálida):**

| Role            | Hex       | Name                  |
|-----------------|-----------|-----------------------|
| `bg`            | `#EBDFC2` | miga (background)     |
| `bg_elevated`   | `#E2D3AF` | orilla (header/panel) |
| `border`        | `#B89A72` | corteza (border)      |
| `border_soft`   | `#C4A88A` | sutil (soft border)   |
| `accent`        | `#9C6620` | dorado tostado        |
| `accent_dim`    | `rgba(156,102,32,0.16)` | selection/hover |
| `text`          | `#2A1D15` | masa oscura (body)    |
| `text_dim`      | `#4A3728` | café (secondary)      |
| `muted`         | `#7A6555` | muted label           |
| `cream`         | `#F5E6C8` | inverted text on fill |

**Semantic status palette (coherent system, used everywhere):**

| Status    | Fill     | Border    | Text on badge | Meaning                |
|-----------|----------|-----------|---------------|------------------------|
| `done`    | `#DCE0B8`| `#7A8A4A` | `#F5E6C8`     | completed, present     |
| `running` | `#E8D5A8`| `#9C6620` | `#9C6620`     | active now             |
| `queued`  | `#F5EBD2`| `#C4A88A` | `#7A6555`     | recedes, not yet       |
| `failed`  | `#E8C9BC`| `#A8351A` | `#7A2410`     | alert, no neon         |
| `blocked` | `#E8D5A8`| `#B07830` | `#7A4E15`     | blocked wait           |
| `stale`   | `#E0D4E0`| `#8A5E8A` | `#5A3868`     | label only (not status)|

There are **two presentation modes** for the same semantic colors:

- **DAG node mode** (wide area, more info per node): light state fill + dark/accent text. Used in the Overview DAG.
- **Badge pill mode** (compact, needs high contrast): solid state fill + cream text. Used in footer, tasks tree, detail panel, runs list. Exception: `queued` keeps muted fill + bg text (same "recedes" semantics); `stale` keeps violet fill + bg text.

| Status    | DAG fill  | DAG text  | Pill fill | Pill text |
|-----------|-----------|-----------|-----------|-----------|
| `done`    | `#DCE0B8` | `#3D4E18` | `#5E7A24` | `#F5E6C8` |
| `running` | `#E8D5A8` | `#9C6620` | `#9C6620` | `#F5E6C8` |
| `queued`  | `#F5EBD2` | `#7A6555` | `#7A6555` | `#EBDFC2` |
| `failed`  | `#E8C9BC` | `#7A2410` | `#A8351A` | `#F5E6C8` |
| `blocked` | `#E8D5A8` | `#7A4E15` | `#B07830` | `#F5E6C8` |
| `stale`   | `#E0D4E0` | `#5A3868` | `#8A5E8A` | `#EBDFC2` |

A single helper `status_badge(ui, status, stale, BadgeMode::Pill|DagNode)` produces either variant from the same palette.

**Contrast check:** muted `#7A6555` on queued `#F5EBD2` ≈ 5.2:1, passes WCAG AA. All other state pairs use solid-fill+cream or solid-fill+dark combos with ≥ 4.5:1.

### 2.2 Density: Balanced (Linear/Raycast)

Comfortable but compact. Sans-serif for UI, monospace for data/code.

### 2.3 Typography: IBM Plex Sans + IBM Plex Mono

**Deliberately not Inter** (Inter is the overused-font tell in the slop catalog). IBM Plex pairs give character, cohesion, and are common in infrastructure tooling (IBM, IPFS, Kubernetes dashboards).

Type scale (target pt):

| Token          | Family             | Size | Weight  | Used for                         |
|----------------|--------------------|------|---------|----------------------------------|
| `wordmark`     | IBM Plex Sans      | 15   | Bold    | `SWARMS` brand                   |
| `heading`      | IBM Plex Sans      | 14   | Medium  | panel titles, stage names        |
| `body`         | IBM Plex Sans      | 13   | Regular | body text, button labels         |
| `caption`      | IBM Plex Sans      | 11   | Regular | secondary text                   |
| `label`        | IBM Plex Sans      | 10   | Medium, uppercase, tracking | group labels, stage labels |
| `mono`         | IBM Plex Mono      | 12   | Regular | task IDs, run IDs, paths         |
| `mono_small`   | IBM Plex Mono      | 11   | Regular | log tail, telemetry counters     |

**Exception to the low-CPU constraint (justified):** two font files (`Inter`-equivalent replacement is **IBM Plex Sans Regular** + **IBM Plex Mono Regular**), compiled in via `include_bytes!`. Impact:
- ~300 KB added to the release binary.
- Zero runtime CPU overhead: fonts load once at startup.
- Not a continuous-repaint loop, not a `wgpu`/`persistence`/`image` feature activation.

This exception will be documented in `docs/UI_RUNTIME_EVALUATION.md` with the justification and the measured binary-size delta.

### 2.4 Spacing tokens

| Token        | Value | Used for                          |
|--------------|-------|-----------------------------------|
| `xs`         | 4     | icon-text gap                     |
| `sm`         | 6     | tight groupings                   |
| `md`         | 8     | default item spacing              |
| `lg`         | 12    | section padding                   |
| `xl`         | 16    | panel padding                     |
| `panel_pad`  | 14    | inner panel padding               |
| `radius_card`| 5     | node/card rounding                |
| `radius_pill`| 3     | badge rounding                    |

Card radius is intentionally small (5px) to read as "tool" not "consumer app blob".

### 2.5 Components

#### Header (52px, up from 46px)
- Vertical separator between brand and breadcrumb.
- Wordmark: IBM Plex Sans 15pt Bold + `RUNTIME` uppercase tracking.
- Breadcrumb: `project / run_id` with `/` separators in border color.
- Status as a **badge pill** with semantic color, not loose `● running` text.
- `Config` promoted to a real button (border + hover).
- `local · native Rust` tag retained.
- Header content stays contract-derived (project / run_id / status). Only presentation changes.

#### Runs list (left panel)
- **Temporal grouping:** Active · now / Earlier today / Yesterday / Older. Buckets by relative age. Pure presentation — does not change run status. Respects "label it, don't change its status" from STATE_CONTRACT.
- **Run IDs in IBM Plex Mono.**
- **Status as compact pill** (semantic palette).
- **Selection by solid dark fill + inverted text** (no left-side stripe — that is the side-tab slop tell).
- **Hover by `accent_dim` fill.**
- Count shown compactly (`12` not `12 tasks`).
- Header shows run-root path (e.g. `~/swarms`) instead of generic "Projects".

#### DAG (Overview tab) — most-visible component
- **State communicated by fill + label, not by stripe or glow.**
- Node states use the semantic palette (see §2.1).
- **Running node emphasized by:** leading glyph `▸` + bold title + 1pt larger + generous padding + slightly elevated fill. This is the chosen method (Option 1) — a Unicode text glyph inline in the title, not an icon container.
- **Waiting/queued nodes** use a lighter-than-background fill (`#F5EBD2` on `#EBDFC2`) so they recede. Text contrast preserved (muted on cream ≈ 5.2:1).
- **Connectors** are simple lines with a small arrowhead (the DAG is directed). Connector color reflects progress: olive when completed, muted when pending. No gradients.
- **Stage names** are plain text (`Plan`, `Code`, `Review`). **No numbered markers** (`1`, `2`, `3` was dropped — the slop catalog flags `01/02/03` numbered section markers as AI editorial scaffolding).
- Optional legend at the DAG foot for large runs.
- Implementation unchanged: `ui.allocate_painter` + `painter.line_segment`. No images, no animations.

#### Status badges pill
- A single helper `status_badge(ui, status, stale)` in `ui_theme.rs` draws every badge everywhere (footer, tasks tree, detail panel, runs list). No duplication.
- egui::Frame with `radius_pill`, fill, padding (4, 2); label in Proportional 9pt bold.

### 2.6 Anti-slop audit

Every tell from https://impeccable.style/slop/ was checked against this design:

| Tell                              | Status    |
|-----------------------------------|-----------|
| Side-tab accent border            | **Avoided.** State via fill; selection via solid fill + inverted text. |
| Border accent on rounded element  | **Avoided.** No unilateral thick borders on cards. |
| Hairline border + wide shadow     | **Avoided.** No decorative shadows anywhere. |
| Glassmorphism / blur              | **Avoided.** Opaque fills only. |
| Dark mode with glowing accents    | **Avoided.** Light theme; running emphasis via glyph + weight, not glow. |
| Overused font (Inter)             | **Avoided.** IBM Plex Sans + Mono. |
| Single font for everything        | **Avoided.** Sans + mono pairing. |
| Numbered section markers          | **Avoided.** Stage names are plain words. |
| Gradient text                     | **Avoided.** Solid colors only. |
| Massive icons / icon tiles        | **Avoided.** `▸` is a text glyph, not an icon container. |
| Monotonous spacing                | **Avoided.** Spacing tokens vary (xs/sm/md/lg/xl). |
| All-caps body text                | **Avoided.** Uppercase restricted to short labels only. |
| Cream / beige palette             | **Flagged but justified.** Comes from the maintainer's existing landing-page identity and the marraqueta concept, not reached for by reflex. Maintained as a deliberate identity decision. |

---

## 3. Architecture

### 3.1 New module: `rust/src/ui_theme.rs`

Sibling of `ui_main.rs`. Feature-gated to `ui-egui` (same as the rest of the egui renderer). Contains:

```rust
// RunStatus already lives in ui_main.rs (rust/src/ui_main.rs:35, in the pure serde/std model half).
// ui_theme imports it from there. No new type introduced.
use crate::ui::RunStatus;

pub enum BadgeMode { DagNode, Pill }

pub struct Theme { pub palette: Palette, pub type_scale: TypeScale, pub spacing: Spacing }
pub struct Palette { /* §2.1 tokens as Color32 */ }
pub struct TypeScale { /* §2.3 pairs as (FontFamily, f32) */ }
pub struct Spacing { /* §2.4 values as f32 */ }

impl Theme {
    pub fn marraqueta() -> Self { /* the one palette we ship */ }
    pub fn apply(&self, ctx: &egui::Context) { /* sets Visuals, Style, fonts */ }
}

pub fn install_fonts(ctx: &egui::Context) { /* include_bytes! for both TTFs */ }

pub fn status_badge(ui: &mut Ui, status: RunStatus, stale: bool, mode: BadgeMode) { /* §2.5 */ }

#[deprecated(note = "use theme.palette.accent")]
pub fn accent() -> Color32 { Theme::marraqueta().palette.accent }
```

All colors live in exactly one place. The existing `apply_theme()` becomes a thin wrapper around `Theme::marraqueta().apply(ctx)`.

### 3.2 Asset layout

```
rust/assets/fonts/
├── IBMPlexSans-Regular.ttf    (~150 KB, SIL OFL 1.1)
└── IBMPlexMono-Regular.ttf    (~140 KB, SIL OFL 1.1)
rust/assets/fonts/LICENSE      (SIL OFL 1.1 for both)
```

Loaded via `include_bytes!("../assets/fonts/IBMPlexSans-Regular.ttf")`. No runtime file IO.

### 3.3 What does NOT change

- The pure serde/std contract model in `ui_main.rs` (lines ~23–1230). Untouched.
- Status derivation logic. Untouched.
- Sanitization. Untouched.
- Polling cadence (1s active, 5s idle). Untouched.
- Event buffer cap (500), log cap (256 KiB). Untouched.
- `request_repaint_after` discipline. Untouched.
- No new crates in `Cargo.toml` beyond what `egui`/`eframe` already pull.

---

## 4. Migration plan

Four phases, each leaves the codebase compiling and tests passing.

### Phase 0 — Scaffolding (no rendering change)
- Create `rust/src/ui_theme.rs` with `Theme`, `Palette`, `TypeScale`, `Spacing`.
- Add `rust/assets/fonts/` with both TTFs + LICENSE.
- Feature-gate `ui_theme` to `ui-egui`.
- Unit tests for palette determinism (status → expected color).
- **Visible change:** none.
- **Verification:** `cargo build`, `cargo test`.

### Phase 1 — Font install + theme bootstrap
- Call `install_fonts(ctx)` once in `run()` (before first `eframe` loop).
- Convert `apply_theme(ctx)` to delegate to `Theme::marraqueta().apply(ctx)`.
- Replace inline `accent()` / `muted()` callers gradually; keep `#[deprecated]` wrappers pointing at the theme.
- **Visible change:** new palette and fonts apply app-wide; component layouts unchanged.
- **Verification:** `cargo build --release`, manual smoke test of `swarms-ui`.

### Phase 2 — Components, one PR per sub-phase
- **2a — Header:** new 52px layout, separator, breadcrumb, status badge. Replaces `render_header`.
- **2b — Runs list:** temporal grouping, pills, fill-based selection. Replaces `render_runs_panel`.
- **2c — DAG:** state fills, arrow connectors, `▸` marker for running. Replaces `render_overview`.
- **2d — Footer + tasks + detail:** reuse `status_badge`, tab touch-ups.
- **Visible change:** each sub-phase visibly upgrades one panel.
- **Verification:** full per-phase validation block.

### Phase 3 — Cleanup
- Remove `#[deprecated]` wrappers; update all call sites to `theme.palette.*`.
- Migrate the DAG's inline `rect_filled` / `rect_stroke` / `line_segment` colors to the palette.
- Document the fonts exception in `docs/UI_RUNTIME_EVALUATION.md`.
- Optionally add an `ADR/` entry recording why egui-native was kept (in light of the OpenCode-to-Electron discussion).
- **Visible change:** none (refactor only).
- **Verification:** full validation block + grep that no `Color32::from_rgb` literals remain outside `ui_theme.rs`.

### Per-phase validation block

```bash
cargo fmt --manifest-path rust/Cargo.toml -- --check
cargo clippy --manifest-path rust/Cargo.toml --all-targets --all-features -- -D warnings
cargo test --manifest-path rust/Cargo.toml --all-features
cargo build --release --manifest-path rust/Cargo.toml --all-features
cargo run --manifest-path rust/Cargo.toml -- doctor
cargo run --manifest-path rust/Cargo.toml --features ui-egui --bin swarms-ui   # manual visual smoke
```

---

## 5. Open questions

None blocking. The cream-palette tension with the slop catalog is resolved by treating it as an identity decision (see §2.6).

---

## 6. Out of scope

- Migrating to Slint, Dioxus, or Tauri. Explicitly rejected by constraints.
- Re-architecting `ui_main.rs` into a `ui/` module tree (the single-file structure is documented and preserved; only `ui_theme.rs` is extracted).
- Adding the `persistence`, `wgpu`, or `image` egui features.
- Animations, charts, or any continuous-repaint visuals.
- Changing the read-only observer contract.
- Mobile or web targets.

---

## 7. References

- `docs/SWARM_UI.md` — UI behavior and constraints
- `docs/SWARM_UI_CONTRACT.md` — data contract (sanitization, status derivation)
- `docs/STATE_CONTRACT.md` — file layout and steering contract
- `docs/UI_RUNTIME_EVALUATION.md` — framework choice + benchmark protocol
- https://impeccable.style/slop/ — anti-slop catalog (applied as design constraint)
