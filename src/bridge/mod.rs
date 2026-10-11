//! Unconnected bridge/client boundary for the staged TUI migration.
//!
//! Projection functions are pure. The session abstraction performs local UI
//! session IO only when explicitly called; no startup/task is installed here,
//! and pairing/Keychain/Teams remain untouched. Only this boundary imports DTOs.

pub(crate) mod controller;
pub(crate) mod events;
pub(crate) mod projection;
pub(crate) mod session;
mod text;

#[cfg(test)]
mod continuity_tests;
#[cfg(test)]
mod tests;
