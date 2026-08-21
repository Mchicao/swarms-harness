//! SWARMS runtime library — deterministic, self-contained Rust coordinator.

pub mod acp;
pub mod adapter;
pub mod claude_stream;
pub mod cli;
pub mod codex_app_server;
pub mod config;
pub mod model;
pub mod opencode_server;
pub mod quota;
pub mod resources;
pub mod review;
pub mod runtime;
pub mod scaling;
pub mod session;
pub mod steering;
pub mod telemetry;
#[path = "ui_main.rs"]
pub mod ui;

#[cfg(feature = "ui-egui")]
#[path = "ui_theme.rs"]
pub mod ui_theme;
pub mod workflow_ir;

pub use model::{slug, Task, TaskSpec};

#[cfg(test)]
mod tests;
