# UI Restyle — Marraqueta Miga Cálida Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Apply a light "Miga Cálida" visual identity and consolidated design-token module to the `swarms-ui` native egui panel, without changing architecture, runtime dependencies, or the read-only observer contract.

**Architecture:** A new feature-gated `ui_theme` module becomes the single source of truth for colors, typography, spacing, and status badges. The existing `ui_egui` module in `rust/src/ui_main.rs` is migrated incrementally: `apply_theme()` delegates to the new module first, then each panel renderer (`render_header`, `render_runs_panel`, `render_overview`, footer/tasks/detail) is updated one PR at a time. No new crates; two font TTFs are compiled in via `include_bytes!`.

**Tech Stack:** Rust 2021, egui 0.32, eframe 0.32 (Glow), IBM Plex Sans/Mono (SIL OFL 1.1).

**Spec:** `docs/superpowers/specs/2026-07-17-ui-marraqueta-restyle-design.md`

---

## File Structure

**Created:**
- `rust/src/ui_theme.rs` — new module: `Theme`, `Palette`, `TypeScale`, `Spacing`, `BadgeMode`, `install_fonts()`, `status_badge()`, deprecated `accent()` / `muted()` shims.
- `rust/assets/fonts/IBMPlexSans-Regular.ttf` — IBM Plex Sans Regular (SIL OFL 1.1).
- `rust/assets/fonts/IBMPlexMono-Regular.ttf` — IBM Plex Mono Regular (SIL OFL 1.1).
- `rust/assets/fonts/LICENSE` — SIL OFL 1.1 text.

**Modified:**
- `rust/Cargo.toml` — no dependency changes; fonts are `include_bytes!`, not crate deps.
- `rust/src/lib.rs` — add `#[cfg(feature = "ui-egui")] #[path = "ui_theme.rs"] pub mod ui_theme;` after the existing `ui` module declaration (line 14-15).
- `rust/src/ui_main.rs` — `apply_theme()` becomes a 1-line wrapper; `accent()`/`muted()` become deprecated shims; `status_color()` becomes deprecated shim; `run()` calls `ui_theme::install_fonts()`; panel renderers restyled (header, runs, overview, footer/tasks/detail).

**Unchanged:** the pure serde/std contract model in `ui_main.rs` (lines ~23-1230), `RunStatus` enum, status derivation, sanitization, polling cadence, `request_repaint_after` discipline, event buffer / log caps.

---

## Conventions

**Validation block** — run after every task that changes code:

```bash
cargo fmt --manifest-path rust/Cargo.toml -- --check
cargo clippy --manifest-path rust/Cargo.toml --all-targets --all-features -- -D warnings
cargo test --manifest-path rust/Cargo.toml --all-features
cargo build --manifest-path rust/Cargo.toml --all-features
```

**Commit style:** conventional commits, scope `ui`. One commit per task unless noted.

**Branch:** create `codex/ui-marraqueta-restyle` off `main` before Task 1 (per `AGENTS.md` external-contribution policy).

**Anti-slop contract** (per https://impeccable.style/slop/): no side-tab accent borders, no unilateral thick stripes on rounded cards, no decorative shadows, no glow, no Inter, no numbered section markers, no gradient text. State is communicated by fill + typography + (for running) the `▸` glyph + bold.

---

## Phase 0 — Scaffolding (no rendering change)

### Task 0.1: Create branch

- [ ] **Step 1: Branch off main**

```bash
cd C:/Proyectos/SWARMS
git checkout main
git pull --ff-only
git checkout -b codex/ui-marraqueta-restyle
```

Expected: `Switched to a new branch 'codex/ui-marraqueta-restyle'`.

### Task 0.2: Add font assets + LICENSE

**Files:**
- Create: `rust/assets/fonts/IBMPlexSans-Regular.ttf`
- Create: `rust/assets/fonts/IBMPlexMono-Regular.ttf`
- Create: `rust/assets/fonts/LICENSE`

- [ ] **Step 1: Create the assets directory**

```bash
mkdir -p rust/assets/fonts
```

- [ ] **Step 2: Download IBM Plex Sans Regular**

Download `IBMPlexSans-Regular.ttf` from the official IBM Plex release (SIL OFL 1.1):
https://github.com/ibm/plex/releases — file lives in `IBM-Plex-Sans.zip` → `fonts/complete/ttf/IBMPlexSans-Regular.ttf`.

Place it at `rust/assets/fonts/IBMPlexSans-Regular.ttf`.

- [ ] **Step 3: Download IBM Plex Mono Regular**

Download `IBMPlexMono-Regular.ttf` from the same release, inside `IBM-Plex-Mono.zip` → `fonts/complete/ttf/IBMPlexMono-Regular.ttf`.

Place it at `rust/assets/fonts/IBMPlexMono-Regular.ttf`.

- [ ] **Step 4: Verify file sizes are plausible**

```bash
ls -l rust/assets/fonts/*.ttf
```

Expected: each file between 100 KB and 200 KB. If either is 0 bytes or missing, the download failed — retry.

- [ ] **Step 5: Save the SIL OFL 1.1 license**

Create `rust/assets/fonts/LICENSE` with the full SIL Open Font License 1.1 text from https://scripts.sil.org/OFL. (Both IBM Plex fonts are released under SIL OFL 1.1.)

- [ ] **Step 6: Commit**

```bash
git add rust/assets/fonts/
git commit -m "chore(ui): add IBM Plex Sans/Mono font assets (SIL OFL 1.1)"
```

### Task 0.3: Create `ui_theme.rs` module with palette + types (no renderer integration yet)

**Files:**
- Create: `rust/src/ui_theme.rs`
- Modify: `rust/src/lib.rs:14-16` (add module declaration)

- [ ] **Step 1: Write the failing test for palette determinism**

Create `rust/src/ui_theme.rs` with only the test stub first:

```rust
//! SWARMS UI design tokens. Single source of truth for colors, typography,
//! spacing, and status badges. Feature-gated to `ui-egui`.
//!
//! See `docs/superpowers/specs/2026-07-17-ui-marraqueta-restyle-design.md`.

#![cfg(feature = "ui-egui")]

use eframe::egui::{self, Color32, FontFamily, Ui};

/// The one palette SWARMS ships.
#[derive(Clone, Copy, Debug)]
pub struct Palette {
    pub bg: Color32,
    pub bg_elevated: Color32,
    pub border: Color32,
    pub border_soft: Color32,
    pub accent: Color32,
    pub accent_dim: Color32,
    pub text: Color32,
    pub text_dim: Color32,
    pub muted: Color32,
    pub cream: Color32,
    // semantic DAG-node fills (light variants)
    pub node_done: Color32,
    pub node_done_border: Color32,
    pub node_run: Color32,
    pub node_run_border: Color32,
    pub node_queued: Color32,
    pub node_queued_border: Color32,
    pub node_failed: Color32,
    pub node_failed_border: Color32,
    pub node_blocked: Color32,
    pub node_blocked_border: Color32,
    pub node_stale: Color32,
    pub node_stale_border: Color32,
    // semantic badge-pill fills (solid variants)
    pub pill_done: Color32,
    pub pill_run: Color32,
    pub pill_queued: Color32,
    pub pill_failed: Color32,
    pub pill_blocked: Color32,
    pub pill_stale: Color32,
}

#[derive(Clone, Copy, Debug)]
pub struct TypeScale {
    pub wordmark: f32,
    pub heading: f32,
    pub body: f32,
    pub caption: f32,
    pub label: f32,
    pub mono: f32,
    pub mono_small: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct Spacing {
    pub xs: f32,
    pub sm: f32,
    pub md: f32,
    pub lg: f32,
    pub xl: f32,
    pub panel_pad: f32,
    pub radius_card: f32,
    pub radius_pill: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct Theme {
    pub palette: Palette,
    pub type_scale: TypeScale,
    pub spacing: Spacing,
}

/// Which presentation variant of a status color to use.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BadgeMode {
    /// Wide area: light fill + dark/accent text. Used by DAG nodes.
    DagNode,
    /// Compact: solid fill + cream text. Used by pills in lists/footer/detail.
    Pill,
}

impl Theme {
    /// The Marraqueta Miga Cálida palette (spec §2.1).
    pub fn marraqueta() -> Self {
        let palette = Palette {
            bg: Color32::from_rgb(0xEB, 0xDF, 0xC2),
            bg_elevated: Color32::from_rgb(0xE2, 0xD3, 0xAF),
            border: Color32::from_rgb(0xB8, 0x9A, 0x72),
            border_soft: Color32::from_rgb(0xC4, 0xA8, 0x8A),
            accent: Color32::from_rgb(0x9C, 0x66, 0x20),
            accent_dim: Color32::from_rgb(0xD9, 0xC7, 0xA8), // approx rgba(156,102,32,0.16) over bg
            text: Color32::from_rgb(0x2A, 0x1D, 0x15),
            text_dim: Color32::from_rgb(0x4A, 0x37, 0x28),
            muted: Color32::from_rgb(0x7A, 0x65, 0x55),
            cream: Color32::from_rgb(0xF5, 0xE6, 0xC8),
            // DAG node fills (light)
            node_done: Color32::from_rgb(0xDC, 0xE0, 0xB8),
            node_done_border: Color32::from_rgb(0x7A, 0x8A, 0x4A),
            node_run: Color32::from_rgb(0xE8, 0xD5, 0xA8),
            node_run_border: Color32::from_rgb(0x9C, 0x66, 0x20),
            node_queued: Color32::from_rgb(0xF5, 0xEB, 0xD2),
            node_queued_border: Color32::from_rgb(0xC4, 0xA8, 0x8A),
            node_failed: Color32::from_rgb(0xE8, 0xC9, 0xBC),
            node_failed_border: Color32::from_rgb(0xA8, 0x35, 0x1A),
            node_blocked: Color32::from_rgb(0xE8, 0xD5, 0xA8),
            node_blocked_border: Color32::from_rgb(0xB0, 0x78, 0x30),
            node_stale: Color32::from_rgb(0xE0, 0xD4, 0xE0),
            node_stale_border: Color32::from_rgb(0x8A, 0x5E, 0x8A),
            // Badge pill fills (solid)
            pill_done: Color32::from_rgb(0x5E, 0x7A, 0x24),
            pill_run: Color32::from_rgb(0x9C, 0x66, 0x20),
            pill_queued: Color32::from_rgb(0x7A, 0x65, 0x55),
            pill_failed: Color32::from_rgb(0xA8, 0x35, 0x1A),
            pill_blocked: Color32::from_rgb(0xB0, 0x78, 0x30),
            pill_stale: Color32::from_rgb(0x8A, 0x5E, 0x8A),
        };
        let type_scale = TypeScale {
            wordmark: 15.0,
            heading: 14.0,
            body: 13.0,
            caption: 11.0,
            label: 10.0,
            mono: 12.0,
            mono_small: 11.0,
        };
        let spacing = Spacing {
            xs: 4.0,
            sm: 6.0,
            md: 8.0,
            lg: 12.0,
            xl: 16.0,
            panel_pad: 14.0,
            radius_card: 5.0,
            radius_pill: 3.0,
        };
        Self { palette, type_scale, spacing }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marraqueta_palette_has_documented_values() {
        let p = Theme::marraqueta().palette;
        assert_eq!(p.bg, Color32::from_rgb(0xEB, 0xDF, 0xC2));
        assert_eq!(p.accent, Color32::from_rgb(0x9C, 0x66, 0x20));
        assert_eq!(p.text, Color32::from_rgb(0x2A, 0x1D, 0x15));
        assert_eq!(p.cream, Color32::from_rgb(0xF5, 0xE6, 0xC8));
    }

    #[test]
    fn type_scale_has_minimum_ratios() {
        let t = Theme::marraqueta().type_scale;
        // heading/body ratio >= 1.05 so hierarchy reads
        assert!((t.heading / t.body) >= 1.05);
        // wordmark is the largest
        assert!(t.wordmark > t.heading);
    }
}
```

- [ ] **Step 2: Register the module in lib.rs**

Modify `rust/src/lib.rs`. The current block at lines 14-15:

```rust
#[path = "ui_main.rs"]
pub mod ui;
```

Change to:

```rust
#[path = "ui_main.rs"]
pub mod ui;

#[cfg(feature = "ui-egui")]
#[path = "ui_theme.rs"]
pub mod ui_theme;
```

- [ ] **Step 3: Run the tests — they should pass (this task defines the values, not behavior to fail first)**

```bash
cargo test --manifest-path rust/Cargo.toml --features ui-egui ui_theme
```

Expected: 2 passed, 0 failed.

- [ ] **Step 4: Run the full validation block**

```bash
cargo fmt --manifest-path rust/Cargo.toml -- --check
cargo clippy --manifest-path rust/Cargo.toml --all-targets --all-features -- -D warnings
cargo test --manifest-path rust/Cargo.toml --all-features
cargo build --manifest-path rust/Cargo.toml --all-features
```

Expected: all pass. The new module compiles but is not yet called by any renderer.

- [ ] **Step 5: Commit**

```bash
git add rust/src/ui_theme.rs rust/src/lib.rs
git commit -m "feat(ui): add ui_theme module with Marraqueta palette + tokens"
```

### Task 0.4: Add `install_fonts()` and `apply()` to `ui_theme`

**Files:**
- Modify: `rust/src/ui_theme.rs`

- [ ] **Step 1: Add font + style application code**

Append to `rust/src/ui_theme.rs` (inside the file, before the `#[cfg(test)] mod tests` block):

```rust
impl Theme {
    /// Install IBM Plex Sans + Mono via `include_bytes!`. Call once at startup.
    pub fn install_fonts(ctx: &egui::Context) {
        let mut fonts = egui::FontDefinitions::default();
        fonts.font_data.insert(
            "ibm-plex-sans".into(),
            egui::FontData::new_static(include_bytes!("../assets/fonts/IBMPlexSans-Regular.ttf")),
        );
        fonts.font_data.insert(
            "ibm-plex-mono".into(),
            egui::FontData::new_static(include_bytes!("../assets/fonts/IBMPlexMono-Regular.ttf")),
        );
        // Make Plex Sans the default Proportional family.
        fonts.families.entry(egui::FontFamily::Proportional).or_default().insert(0, "ibm-plex-sans".into());
        // Named families for explicit use by renderers.
        fonts.families.entry(egui::FontFamily::Name("IBM Plex Sans".into())).or_default().push("ibm-plex-sans".into());
        fonts.families.entry(egui::FontFamily::Name("IBM Plex Mono".into())).or_default().push("ibm-plex-mono".into());
        // Make Plex Mono the default Monospace family too.
        fonts.families.entry(egui::FontFamily::Monospace).or_default().insert(0, "ibm-plex-mono".into());
        ctx.set_fonts(fonts);
    }

    /// Apply palette + spacing + visuals to an egui context. Idempotent.
    pub fn apply(&self, ctx: &egui::Context) {
        let mut style = (*ctx.style()).clone();
        style.spacing.item_spacing = egui::vec2(self.spacing.md, self.spacing.sm);
        style.spacing.button_padding = egui::vec2(self.spacing.md, self.spacing.xs);
        // Light visuals (this is a light theme).
        let mut visuals = egui::Visuals::light();
        visuals.panel_fill = self.palette.bg;
        visuals.window_fill = self.palette.bg_elevated;
        visuals.extreme_bg_color = self.palette.bg_elevated;
        visuals.faint_bg_color = self.palette.bg_elevated;
        visuals.selection.bg_fill = self.palette.accent_dim;
        visuals.selection.stroke = egui::Stroke::new(1.0, self.palette.accent);
        visuals.hyperlink_color = self.palette.accent;
        visuals.widgets.inactive.weak_bg_fill = self.palette.bg_elevated;
        visuals.widgets.hovered.weak_bg_fill = self.palette.accent_dim;
        visuals.widgets.inactive.fg_stroke = egui::Stroke::new(1.0, self.palette.text_dim);
        visuals.widgets.hovered.fg_stroke = egui::Stroke::new(1.0, self.palette.text);
        style.visuals = visuals;
        ctx.set_style(style);
    }
}
```

- [ ] **Step 2: Add a smoke test for `apply` (does not panic with a real Context)**

Append inside the `mod tests` block:

```rust
    #[test]
    fn apply_does_not_panic_with_default_context() {
        // We can't easily build an egui::Context in a unit test without a window,
        // but we can at least confirm install_fonts is never called here and that
        // apply() is a pure function of (Theme, Context). The real smoke test is
        // the manual visual check in Task 1.2.
        let _theme = Theme::marraqueta();
        // If apply() ever needs preconditions, they should fail at compile time.
    }
```

- [ ] **Step 3: Run the validation block**

```bash
cargo fmt --manifest-path rust/Cargo.toml -- --check
cargo clippy --manifest-path rust/Cargo.toml --all-targets --all-features -- -D warnings
cargo test --manifest-path rust/Cargo.toml --all-features
cargo build --manifest-path rust/Cargo.toml --all-features
```

Expected: all pass. `include_bytes!` resolves at compile time — if either TTF is missing, build fails with a clear error (revisit Task 0.2).

- [ ] **Step 4: Commit**

```bash
git add rust/src/ui_theme.rs
git commit -m "feat(ui): add install_fonts + Theme::apply to ui_theme"
```

### Task 0.5: Add `status_badge()` helper

**Files:**
- Modify: `rust/src/ui_theme.rs`

- [ ] **Step 1: Add the `status_badge` function**

Append to `rust/src/ui_theme.rs` (before the `#[cfg(test)]` block):

```rust
/// Resolve (fill, text_color, border_color) for a task status string and a
/// `stale` flag, in either DAG-node or pill presentation. The `status` string
/// follows the existing contract values: "completed", "in_progress", "queued",
/// "failed", "blocked", plus anything else falls back to the muted/queued look.
///
/// `stale` always wins (returns the stale palette) per STATE_CONTRACT: stale is
/// a label, not a status change.
pub fn status_colors(status: &str, stale: bool, mode: BadgeMode, palette: &Palette) -> (Color32, Color32, Color32) {
    let p = *palette;
    if stale {
        return match mode {
            BadgeMode::DagNode => (p.node_stale, p.cream, p.node_stale_border),
            BadgeMode::Pill => (p.pill_stale, p.cream, p.pill_stale),
        };
    }
    match (status, mode) {
        ("completed", BadgeMode::DagNode) => (p.node_done, Color32::from_rgb(0x3D, 0x4E, 0x18), p.node_done_border),
        ("completed", BadgeMode::Pill) => (p.pill_done, p.cream, p.pill_done),
        ("in_progress", BadgeMode::DagNode) => (p.node_run, p.accent, p.node_run_border),
        ("in_progress", BadgeMode::Pill) => (p.pill_run, p.cream, p.pill_run),
        ("queued", BadgeMode::DagNode) => (p.node_queued, p.muted, p.node_queued_border),
        ("queued", BadgeMode::Pill) => (p.pill_queued, p.bg, p.pill_queued),
        ("failed", BadgeMode::DagNode) => (p.node_failed, Color32::from_rgb(0x7A, 0x24, 0x10), p.node_failed_border),
        ("failed", BadgeMode::Pill) => (p.pill_failed, p.cream, p.pill_failed),
        ("blocked", BadgeMode::DagNode) => (p.node_blocked, Color32::from_rgb(0x7A, 0x4E, 0x15), p.node_blocked_border),
        ("blocked", BadgeMode::Pill) => (p.pill_blocked, p.cream, p.pill_blocked),
        _ => match mode {
            BadgeMode::DagNode => (p.node_queued, p.muted, p.node_queued_border),
            BadgeMode::Pill => (p.pill_queued, p.bg, p.pill_queued),
        },
    }
}

/// Render a status badge inline. Returns the resolved (fill, text) so callers
/// can render text in matching color if they prefer. The badge is drawn with
/// egui::Frame; no shadows, no stripes, no glow.
pub fn status_badge(ui: &mut Ui, status: &str, stale: bool, mode: BadgeMode, palette: &Palette) {
    let (fill, text, _border) = status_colors(status, stale, mode, palette);
    let label = if stale { "stale" } else { status };
    egui::Frame::group(ui.style())
        .fill(fill)
        .corner_radius(palette.radius_pill.into())
        .inner_margin(egui::Margin::symmetric(palette.xs + 2.0, palette.xs - 2.0))
        .stroke(egui::Stroke::NONE)
        .show(ui, |ui| {
            ui.label(
                egui::RichText::new(label)
                    .size(9.0)
                    .strong()
                    .color(text),
            );
        });
}
```

- [ ] **Step 2: Add tests for `status_colors`**

Append inside `mod tests`:

```rust
    #[test]
    fn stale_overrides_status_in_status_colors() {
        let p = Theme::marraqueta().palette;
        let (fill_running, _, _) = status_colors("in_progress", false, BadgeMode::Pill, &p);
        let (fill_stale, _, _) = status_colors("in_progress", true, BadgeMode::Pill, &p);
        assert_eq!(fill_stale, p.pill_stale);
        assert_ne!(fill_stale, fill_running);
    }

    #[test]
    fn pill_run_uses_accent_fill() {
        let p = Theme::marraqueta().palette;
        let (fill, text, _) = status_colors("in_progress", false, BadgeMode::Pill, &p);
        assert_eq!(fill, p.pill_run);
        assert_eq!(text, p.cream);
    }

    #[test]
    fn dagnode_queued_uses_light_fill() {
        let p = Theme::marraqueta().palette;
        let (fill, _, _) = status_colors("queued", false, BadgeMode::DagNode, &p);
        assert_eq!(fill, p.node_queued);
    }

    #[test]
    fn unknown_status_falls_back_to_queued() {
        let p = Theme::marraqueta().palette;
        let (fill, _, _) = status_colors("??", false, BadgeMode::Pill, &p);
        assert_eq!(fill, p.pill_queued);
    }
```

- [ ] **Step 3: Run the validation block**

```bash
cargo fmt --manifest-path rust/Cargo.toml -- --check
cargo clippy --manifest-path rust/Cargo.toml --all-targets --all-features -- -D warnings
cargo test --manifest-path rust/Cargo.toml --all-features
cargo build --manifest-path rust/Cargo.toml --all-features
```

Expected: all pass. 6 tests in `ui_theme` (2 from 0.3 + 4 from 0.5).

- [ ] **Step 4: Commit**

```bash
git add rust/src/ui_theme.rs
git commit -m "feat(ui): add status_colors + status_badge helper to ui_theme"
```

---

## Phase 1 — Font install + theme bootstrap

### Task 1.1: Make `apply_theme()` delegate to `ui_theme`

**Files:**
- Modify: `rust/src/ui_main.rs:1241-1256` (the `apply_theme` function in `mod ui_egui`)

- [ ] **Step 1: Replace `apply_theme` with a thin delegate**

In `rust/src/ui_main.rs`, find the function at line 1241:

```rust
    fn apply_theme(ctx: &egui::Context) {
        let mut style = (*ctx.style()).clone();
        style.spacing.item_spacing = egui::vec2(8.0, 6.0);
        style.spacing.button_padding = egui::vec2(8.0, 4.0);
        style.visuals = egui::Visuals::dark();
        style.visuals.panel_fill = egui::Color32::from_rgb(14, 15, 18);
        style.visuals.window_fill = egui::Color32::from_rgb(17, 18, 22);
        style.visuals.extreme_bg_color = egui::Color32::from_rgb(9, 10, 12);
        style.visuals.faint_bg_color = egui::Color32::from_rgb(22, 24, 29);
        style.visuals.selection.bg_fill = egui::Color32::from_rgb(45, 42, 78);
        style.visuals.selection.stroke.color = egui::Color32::from_rgb(176, 166, 255);
        style.visuals.hyperlink_color = egui::Color32::from_rgb(176, 166, 255);
        style.visuals.widgets.inactive.weak_bg_fill = egui::Color32::from_rgb(22, 24, 29);
        style.visuals.widgets.hovered.weak_bg_fill = egui::Color32::from_rgb(31, 33, 40);
        ctx.set_style(style);
    }
```

Replace with:

```rust
    fn apply_theme(ctx: &egui::Context) {
        crate::ui_theme::Theme::marraqueta().apply(ctx);
    }
```

- [ ] **Step 2: Make `accent()` and `muted()` deprecated shims**

In `rust/src/ui_main.rs`, find lines 1233-1239:

```rust
    fn accent() -> egui::Color32 {
        egui::Color32::from_rgb(128, 108, 255)
    }

    fn muted() -> egui::Color32 {
        egui::Color32::from_rgb(132, 135, 145)
    }
```

Replace with:

```rust
    #[deprecated(note = "use crate::ui_theme::Theme::marraqueta().palette.accent")]
    fn accent() -> egui::Color32 {
        crate::ui_theme::Theme::marraqueta().palette.accent
    }

    #[deprecated(note = "use crate::ui_theme::Theme::marraqueta().palette.muted")]
    fn muted() -> egui::Color32 {
        crate::ui_theme::Theme::marraqueta().palette.muted
    }
```

- [ ] **Step 3: Run validation block**

```bash
cargo fmt --manifest-path rust/Cargo.toml -- --check
cargo clippy --manifest-path rust/Cargo.toml --all-targets --all-features -- -D warnings
cargo test --manifest-path rust/Cargo.toml --all-features
cargo build --manifest-path rust/Cargo.toml --all-features
```

Expected: all pass. `clippy -D warnings` will warn about deprecated call sites — that's expected and is the point (Phase 3 removes them). If clippy fails the build on `deprecated`, allow it temporarily by adding `#[allow(deprecated)]` at the top of the call sites only if needed. Do NOT change the `clippy -D warnings` policy.

Note: if clippy errors (rather than warns) on deprecated, add this at the top of `mod ui_egui`:

```rust
#![allow(deprecated)]
```

and document that Phase 3 removes the allow.

- [ ] **Step 4: Commit**

```bash
git add rust/src/ui_main.rs
git commit -m "refactor(ui): apply_theme delegates to ui_theme; accent/muted deprecated"
```

### Task 1.2: Call `install_fonts()` in `run()`

**Files:**
- Modify: `rust/src/ui_main.rs:2955-2963` (the `eframe::run_native` closure in `run()`)

- [ ] **Step 1: Add the `install_fonts` call**

In `rust/src/ui_main.rs`, find the closure at line 2958-2962:

```rust
            Box::new(move |cc| {
                egui_extras::install_image_loaders(&cc.egui_ctx);
                apply_theme(&cc.egui_ctx);
                Ok(Box::new(app))
            }),
```

Replace with:

```rust
            Box::new(move |cc| {
                egui_extras::install_image_loaders(&cc.egui_ctx);
                crate::ui_theme::Theme::install_fonts(&cc.egui_ctx);
                apply_theme(&cc.egui_ctx);
                Ok(Box::new(app))
            }),
```

- [ ] **Step 2: Run validation block**

```bash
cargo fmt --manifest-path rust/Cargo.toml -- --check
cargo clippy --manifest-path rust/Cargo.toml --all-targets --all-features -- -D warnings
cargo test --manifest-path rust/Cargo.toml --all-features
cargo build --manifest-path rust/Cargo.toml --all-features
```

Expected: all pass.

- [ ] **Step 3: Manual smoke test**

```bash
cargo run --manifest-path rust/Cargo.toml --features ui-egui --bin swarms-ui
```

Expected: the window opens with a **light** cream background (`#EBDFC2`), dark brown text, and IBM Plex typography. Layout still has the old header/runs-list shape (Phase 2 changes those), but the palette and fonts are visibly new.

- [ ] **Step 4: Commit**

```bash
git add rust/src/ui_main.rs
git commit -m "feat(ui): install IBM Plex fonts at swarms-ui startup"
```

---

## Phase 2 — Components (one PR per sub-task)

Each sub-task here ends with a commit and the full validation block. The Phase-2 tasks touch the actual renderers in `rust/src/ui_main.rs`.

### Task 2.1: Restyle the header

**Files:**
- Modify: `rust/src/ui_main.rs` — `render_header` function (search for `TopBottomPanel::top("app_header")` around line 1692).

- [ ] **Step 1: Locate the header renderer**

Run:

```bash
grep -n "app_header" rust/src/ui_main.rs
```

Expected: one line pointing at the `TopBottomPanel::top("app_header")` call. Open that function and read it end-to-end before editing.

- [ ] **Step 2: Rewrite `render_header` to the new spec**

Replace the existing header rendering code (inside the function that wraps the `app_header` panel) with the new layout. Use the design tokens from `ui_theme`. The header must still derive its content from the existing `contract.run.status.label()` and `contract.run.run_id` — do not invent data.

Key changes:
- Panel height: `46.0` → `52.0` (find the exact_height argument).
- Wordmark: `SWARMS` in `IBM Plex Sans`, size 15, strong. `RUNTIME` in size 10, color muted, tracking.
- Vertical `Separator` between brand block and breadcrumb.
- Breadcrumb: `project / run_id` using `/` separator characters in border color.
- Status as a pill badge via `ui_theme::status_badge(ui, contract.run.status.label(), false, BadgeMode::Pill, palette)`.
- `Config` button: real `ui.add(egui::Button::new(...))` instead of `selectable_label`.
- `local · native Rust` tag: existing `Label` restyled with `palette.border` border and `palette.bg` fill.

Because the exact existing code spans many lines and varies, the implementer must read the function in full and adapt the structure. The contract is: same data sources, new visual treatment, no new state.

- [ ] **Step 3: Run validation block**

```bash
cargo fmt --manifest-path rust/Cargo.toml -- --check
cargo clippy --manifest-path rust/Cargo.toml --all-targets --all-features -- -D warnings
cargo test --manifest-path rust/Cargo.toml --all-features
cargo build --manifest-path rust/Cargo.toml --all-features
```

Expected: all pass.

- [ ] **Step 4: Manual smoke test**

```bash
cargo run --manifest-path rust/Cargo.toml --features ui-egui --bin swarms-ui
```

Expected: header is 52px tall, cream background, IBM Plex wordmark, status pill in semantic color, Config is a real button.

- [ ] **Step 5: Commit**

```bash
git add rust/src/ui_main.rs
git commit -m "feat(ui): restyle header with Miga Cálida tokens + status pill"
```

### Task 2.2: Restyle the runs panel (left sidebar)

**Files:**
- Modify: `rust/src/ui_main.rs` — the function rendering `SidePanel::left("runs")` (around line 1774).

- [ ] **Step 1: Locate and read the runs renderer**

```bash
grep -n 'SidePanel::left("runs")' rust/src/ui_main.rs
```

Read the full function.

- [ ] **Step 2: Add a temporal-bucket helper**

Inside `mod ui_egui`, add a private helper that, given a list of runs with timestamps and `now_ms`, returns four buckets: `Active` (now), `Earlier today`, `Yesterday`, `Older`. The function signature:

```rust
fn temporal_bucket(age_ms: u128) -> &'static str {
    const HOUR: u128 = 3_600_000;
    const DAY: u128 = 24 * HOUR;
    if age_ms < HOUR { "Active" }
    else if age_ms < 6 * HOUR { "Earlier today" }
    else if age_ms < DAY { "Earlier today" }
    else if age_ms < 2 * DAY { "Yesterday" }
    else { "Older" }
}
```

This is presentation only — it does not change the stored run status.

- [ ] **Step 3: Rewrite the runs panel body**

Rebuild the run rows using:
- The new `status_badge` pill for each run's status.
- Run IDs in `IBM Plex Mono` via `RichText::new(id).family(FontFamily::Name("IBM Plex Mono".into())).size(12.0)`.
- **Selection by solid dark fill + inverted cream text** — use `ui.style_mut().visuals.widgets.active.weak_bg_fill = palette.text` before drawing the selected row, then restore. **No left-side stripe** (that's the slop tell).
- Hover by setting the row's frame fill to `palette.accent_dim`.
- Compact count (`12` not `12 tasks`).
- Header: `Runs` title + run-root path in mono, muted.

- [ ] **Step 4: Run validation block**

```bash
cargo fmt --manifest-path rust/Cargo.toml -- --check
cargo clippy --manifest-path rust/Cargo.toml --all-targets --all-features -- -D warnings
cargo test --manifest-path rust/Cargo.toml --all-features
cargo build --manifest-path rust/Cargo.toml --all-features
```

- [ ] **Step 5: Manual smoke test**

```bash
cargo run --manifest-path rust/Cargo.toml --features ui-egui --bin swarms-ui
```

Expected: runs grouped under Active / Earlier today / Yesterday / Older headers, each row has a status pill, selected row is dark with cream text (no left stripe).

- [ ] **Step 6: Commit**

```bash
git add rust/src/ui_main.rs
git commit -m "feat(ui): restyle runs panel with temporal grouping + status pills"
```

### Task 2.3: Restyle the DAG (Overview tab)

**Files:**
- Modify: `rust/src/ui_main.rs` — `render_overview` (around line 2132) and the DAG painter around lines 2207-2236.

- [ ] **Step 1: Locate the DAG renderer and its painter**

```bash
grep -n "fn render_overview\|allocate_painter\|rect_filled\|line_segment" rust/src/ui_main.rs | head -20
```

Read the whole `render_overview` and the inline DAG painter.

- [ ] **Step 2: Replace node fill/stroke with palette tokens**

In the DAG painter, every `painter.rect_filled` and `painter.rect_stroke` for a task node must use `ui_theme::status_colors(&node.status, node.is_stale(now_ms, interval), BadgeMode::DagNode, &palette)`. The returned `(fill, text_color, border_color)` drives `rect_filled`, the node label's `RichText` color, and `rect_stroke` respectively.

Replace every `Color32::from_rgb(...)` literal in the DAG painter (lines ~2207-2236) with a reference to a palette token. After this task, `grep -n "Color32::from_rgb" rust/src/ui_main.rs` should return **zero** hits inside the DAG painter.

- [ ] **Step 3: Add the `▸` glyph + bold + 1pt for running nodes**

When `node.status == "in_progress"` and `!node.is_stale(...)`, the node label text becomes:

```rust
format!("▸ {}", node.id_or_label)
```

Rendered with `RichText::new(...).size(12.0).strong().color(palette.accent).family(FontFamily::Name("IBM Plex Mono".into()))`.

For all other nodes, the label stays at size 11, regular weight, with the status color from `status_colors`.

- [ ] **Step 4: Replace the connector drawing with an arrow**

The existing connector is a `painter.line_segment` at 1px gray. Replace it with a 1px line in `palette.muted` (or `palette.pill_done` for completed segments) plus a small arrowhead: a second `line_segment` or a filled triangle using `painter.add(Shape::convex_polygon(...))` with two points offset from the arrow tip.

No gradient, no shadow, no decoration.

- [ ] **Step 5: Remove any numbered stage markers**

If the current code emits `"1 Plan"`, `"2 Code"`, etc., remove the numeric prefix. Stage labels are the bare stage name (`Plan`, `Code`, `Review`).

- [ ] **Step 6: Run validation block**

```bash
cargo fmt --manifest-path rust/Cargo.toml -- --check
cargo clippy --manifest-path rust/Cargo.toml --all-targets --all-features -- -D warnings
cargo test --manifest-path rust/Cargo.toml --all-features
cargo build --manifest-path rust/Cargo.toml --all-features
```

- [ ] **Step 7: Verify no inline colors remain in the DAG painter**

```bash
awk '/fn render_overview/,/^    }$/' rust/src/ui_main.rs | grep "Color32::from_rgb"
```

Expected: no output.

- [ ] **Step 8: Manual smoke test**

```bash
cargo run --manifest-path rust/Cargo.toml --features ui-egui --bin swarms-ui
```

Expected: DAG nodes show state by fill (done/running/queued/failed), running node has `▸` prefix + bold, connectors have arrowheads, no numbered stage labels.

- [ ] **Step 9: Commit**

```bash
git add rust/src/ui_main.rs
git commit -m "feat(ui): restyle DAG with state fills + arrow connectors + running marker"
```

### Task 2.4: Restyle footer + tasks tree + detail panel (use `status_badge`)

**Files:**
- Modify: `rust/src/ui_main.rs` — `render_footer` (the `TopBottomPanel::bottom("footer")` renderer, ~line 1737), `render_tree` (~line 2361), `render_detail` (~line 1853).

- [ ] **Step 1: Replace `status_color(...)` call sites with `status_badge` or `status_colors`**

Search for every call to `status_color(`:

```bash
grep -n "status_color(" rust/src/ui_main.rs
```

For each call site outside the DAG painter (which Task 2.3 already migrated):
- In the footer status dot: replace the `painter.circle_filled(..., status_color(...))` with a pill badge via `ui_theme::status_badge(ui, ..., BadgeMode::Pill, palette)`.
- In the tasks tree: replace `.color(status_color(...))` on the row label with a status_badge pill at the row's trailing edge.
- In the detail panel: replace `.color(status_color(...))` on the status row with a pill badge.

- [ ] **Step 2: Deprecate the local `status_color` function**

In `rust/src/ui_main.rs` find `fn status_color(status: &str, stale: bool) -> egui::Color32` at line 2891. Add `#[deprecated(note = "use ui_theme::status_colors")]` above it and make it delegate:

```rust
    #[deprecated(note = "use ui_theme::status_colors")]
    fn status_color(status: &str, stale: bool) -> egui::Color32 {
        let p = crate::ui_theme::Theme::marraqueta().palette;
        let (fill, _, _) = crate::ui_theme::status_colors(status, stale, crate::ui_theme::BadgeMode::DagNode, &p);
        fill
    }
```

If `mod ui_egui` has a top-level `#![allow(deprecated)]` from Task 1.1, keep it until Phase 3.

- [ ] **Step 3: Run validation block**

```bash
cargo fmt --manifest-path rust/Cargo.toml -- --check
cargo clippy --manifest-path rust/Cargo.toml --all-targets --all-features -- -D warnings
cargo test --manifest-path rust/Cargo.toml --all-features
cargo build --manifest-path rust/Cargo.toml --all-features
```

- [ ] **Step 4: Manual smoke test**

```bash
cargo run --manifest-path rust/Cargo.toml --features ui-egui --bin swarms-ui
```

Expected: footer status appears as a pill, tasks tree rows have a pill at the right, detail panel shows a status pill.

- [ ] **Step 5: Commit**

```bash
git add rust/src/ui_main.rs
git commit -m "feat(ui): footer/tasks/detail use status_badge; status_color deprecated"
```

---

## Phase 3 — Cleanup + docs

### Task 3.1: Remove deprecated shims

**Files:**
- Modify: `rust/src/ui_main.rs`

- [ ] **Step 1: Migrate every `accent()` call site to the palette**

```bash
grep -n "accent()" rust/src/ui_main.rs
```

For each match, replace `accent()` with `crate::ui_theme::Theme::marraqueta().palette.accent`. (If `palette` is already in scope from a local binding, use that instead.)

- [ ] **Step 2: Migrate every `muted()` call site**

```bash
grep -n "muted()" rust/src/ui_main.rs
```

Replace with `crate::ui_theme::Theme::marraqueta().palette.muted` (or the in-scope `palette.muted`).

- [ ] **Step 3: Delete the `accent`, `muted`, and `status_color` functions**

After all call sites are migrated, delete the three deprecated functions (lines 1233-1239 and 2891-2903 area).

- [ ] **Step 4: Remove the `#![allow(deprecated)]` if it was added**

If Task 1.1 / 2.4 added `#![allow(deprecated)]` at the top of `mod ui_egui`, remove it now. `clippy -D warnings` must pass with no allows.

- [ ] **Step 5: Run validation block**

```bash
cargo fmt --manifest-path rust/Cargo.toml -- --check
cargo clippy --manifest-path rust/Cargo.toml --all-targets --all-features -- -D warnings
cargo test --manifest-path rust/Cargo.toml --all-features
cargo build --manifest-path rust/Cargo.toml --all-features
```

Expected: all pass with zero deprecated warnings.

- [ ] **Step 6: Commit**

```bash
git add rust/src/ui_main.rs
git commit -m "refactor(ui): remove deprecated accent/muted/status_color shims"
```

### Task 3.2: Verify no inline `Color32::from_rgb` outside `ui_theme.rs`

**Files:**
- Inspect: `rust/src/ui_main.rs`

- [ ] **Step 1: Grep for stray literals**

```bash
grep -n "Color32::from_rgb" rust/src/ui_main.rs
```

Expected: zero matches. Every color now flows through `ui_theme`.

If any remain, migrate them to the palette. Do not add new palette tokens for one-off colors unless a doc-string justifies them.

- [ ] **Step 2: Commit if anything changed**

```bash
git add rust/src/ui_main.rs
git commit -m "refactor(ui): migrate remaining inline colors to ui_theme"
```

(If grep returned nothing in Step 1, skip this commit.)

### Task 3.3: Document the fonts exception in UI_RUNTIME_EVALUATION.md

**Files:**
- Modify: `docs/UI_RUNTIME_EVALUATION.md`

- [ ] **Step 1: Add a "Font assets" subsection**

Add a new subsection (after the existing framework-choice section) explaining:
- Two TTFs (IBM Plex Sans Regular + IBM Plex Mono Regular) are compiled in via `include_bytes!`.
- Combined size ~300 KB; verified against the release binary size.
- Loaded once at startup via `Theme::install_fonts()`; zero runtime CPU overhead.
- Not a `wgpu`/`persistence`/`image` feature activation; complies with the "no features until functional need" rule because legible typography is the functional need.
- License: SIL OFL 1.1 for both fonts; license text in `rust/assets/fonts/LICENSE`.

- [ ] **Step 2: Commit**

```bash
git add docs/UI_RUNTIME_EVALUATION.md
git commit -m "docs(ui): document IBM Plex font exception in UI_RUNTIME_EVALUATION"
```

### Task 3.4: Run the full SWARMS validation suite

- [ ] **Step 1: Run every validation command from AGENTS.md**

```bash
cargo fmt --manifest-path rust/Cargo.toml -- --check
cargo clippy --manifest-path rust/Cargo.toml --all-targets --all-features -- -D warnings
cargo test --manifest-path rust/Cargo.toml --all-features
cargo build --release --manifest-path rust/Cargo.toml --all-features
cargo run --manifest-path rust/Cargo.toml -- doctor
cargo run --manifest-path rust/Cargo.toml -- run --plan docs/workflow_plan_example.json --force --run-id verify-ui-restyle --global-max-concurrency 3 --provider-cap mock=3
```

Expected: every command exits 0. The `run` command produces a completed workflow with `verify-ui-restyle` run id; the UI should show that run with the new theme if you open `swarms-ui` against it.

- [ ] **Step 2: Manual smoke against the verify run**

```bash
cargo run --manifest-path rust/Cargo.toml --features ui-egui --bin swarms-ui -- --run-id verify-ui-restyle
```

Expected: window opens on the verify-ui-restyle run, all four tabs render with the new theme, no panic, no layout overflow.

### Task 3.5: Open PR

- [ ] **Step 1: Push the branch**

```bash
git push -u origin codex/ui-marraqueta-restyle
```

- [ ] **Step 2: Open the PR**

```bash
gh pr create --title "feat(ui): Marraqueta Miga Cálida restyle" --body "Implements docs/superpowers/specs/2026-07-17-ui-marraqueta-restyle-design.md.

- New ui_theme module: single source of truth for colors, typography, spacing, status badges.
- Light Miga Cálida palette derived from maintainer identity.
- IBM Plex Sans + Mono (SIL OFL 1.1), compiled in via include_bytes!.
- Header / runs panel / DAG / footer migrated incrementally.
- Anti-slop audited against impeccable.style/slop/.
- No architecture change, no new runtime deps, read-only contract preserved.

Validation: cargo fmt/clippy/test/build --all-features pass; doctor + verify run pass."
```

- [ ] **Step 3: Paste the PR URL back into the session for the user.**

---

## Self-review notes (post-write)

**Spec coverage check:**
- §2.1 palette → Task 0.3 ✓
- §2.3 typography + font exception → Task 0.4 + 1.2 + 3.3 ✓
- §2.4 spacing → Task 0.3 ✓
- §2.5 header → Task 2.1 ✓
- §2.5 runs list → Task 2.2 ✓
- §2.5 DAG + running glyph → Task 2.3 ✓
- §2.5 status badges → Task 0.5 (helper) + 2.4 (deployment) ✓
- §2.6 anti-slop → enforced in each Phase 2 task ✓
- §3.1 new module → Task 0.3 ✓
- §3.2 assets → Task 0.2 ✓
- §4 migration phases → Phase 0/1/2/3 mapping 1:1 ✓

**Type consistency:**
- `Theme::marraqueta()` defined in 0.3, used everywhere.
- `Palette` fields used in `status_colors` (0.5) match the struct in 0.3.
- `BadgeMode::DagNode|Pill` consistent across 0.5, 2.3, 2.4.
- `install_fonts` signature `(ctx: &egui::Context)` matches call site at 1.2.
- `apply` signature `(ctx: &egui::Context)` matches `apply_theme` in 1.1.

**Placeholder scan:** none. Every step has exact code or exact commands.

**Open implementation note:** Task 2.1 / 2.2 / 2.3 / 2.4 say "read the function and adapt" rather than paste full replacement code, because the existing renderers are 100+ lines each and the implementer must preserve data-flow contracts. This is intentional, not a placeholder — the contracts (which data sources feed each element) are spelled out.
