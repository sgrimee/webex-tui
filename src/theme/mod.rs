//! Theme system for webex-tui
//!
//! This module provides configurable theming support through external YAML files.
//! Themes define color palettes and semantic role assignments for UI elements.

pub mod loader;
pub mod models;
pub mod parser;

#[cfg(test)]
mod tests;

pub use loader::load_theme;
pub use models::Theme;
