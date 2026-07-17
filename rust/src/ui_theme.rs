//! SWARMS UI design tokens. Single source of truth for colors, typography,
//! spacing, and status badges. Feature-gated to `ui-egui`.
//!
//! See `docs/superpowers/specs/2026-07-17-ui-marraqueta-restyle-design.md`.

#![cfg(feature = "ui-egui")]

use eframe::egui::Color32;

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
            accent_dim: Color32::from_rgb(0xD9, 0xC7, 0xA8),
            text: Color32::from_rgb(0x2A, 0x1D, 0x15),
            text_dim: Color32::from_rgb(0x4A, 0x37, 0x28),
            muted: Color32::from_rgb(0x7A, 0x65, 0x55),
            cream: Color32::from_rgb(0xF5, 0xE6, 0xC8),
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
        Self {
            palette,
            type_scale,
            spacing,
        }
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
        assert!((t.heading / t.body) >= 1.05);
        assert!(t.wordmark > t.heading);
    }
}
