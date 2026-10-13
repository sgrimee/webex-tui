//! Unconnected bridge/client boundary for the staged TUI migration.
//!
//! Projection functions are pure. The session abstraction performs local UI
//! session IO only when explicitly called; no startup/task is installed here,
//! and pairing/Keychain/Teams remain untouched. Only this boundary imports
//! bridge DTOs and projects the still-live legacy transport into frontend values.

pub(crate) mod commands;
pub(crate) mod controller;
pub(crate) mod events;
pub(crate) mod legacy;
pub(crate) mod pairing;
pub(crate) mod presentation;
pub(crate) mod projection;
pub(crate) mod read_dispatch;
pub(crate) mod runtime;
pub(crate) mod session;
pub(crate) mod snapshot;
pub(crate) mod startup;
mod text;
pub(crate) mod write_coordinator;
pub(crate) mod write_dispatch;

#[cfg(test)]
mod continuity_tests;
#[cfg(test)]
mod tests;
