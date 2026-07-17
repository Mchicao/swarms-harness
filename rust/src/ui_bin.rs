//! Feature-gated SWARMS observer binary entrypoint.

#![cfg_attr(windows, windows_subsystem = "windows")]

fn main() -> eframe::Result {
    swarms_runtime::ui::ui_egui::run()
}
