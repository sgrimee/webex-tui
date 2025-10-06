//! Theme system for webex-tui
//!
//! This module provides configurable theming support through external YAML files.
//! Themes define color palettes and semantic role assignments for UI elements.

pub mod models;
pub mod loader;
pub mod parser;

#[cfg(test)]
mod tests;

pub use models::Theme;
pub use loader::load_theme;