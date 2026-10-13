//! Bridge/client boundary of the TUI.
//!
//! Projection functions are pure. The session abstraction performs local UI
//! session IO only when explicitly called; `main` runs `startup` and spawns
//! the runtime (S4). Only this boundary imports bridge DTOs; `legacy` still
//! projects the old Teams transport until the 5E removal.

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
