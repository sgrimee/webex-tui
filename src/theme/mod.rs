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

// Re-export copy_bundled_themes for use within the crate
pub(crate) use loader::copy_bundled_themes;
