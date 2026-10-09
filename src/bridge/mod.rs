//! Compile-only bridge/client boundary for the staged TUI migration.
//!
//! No function here starts a session, pairs, connects a socket, dispatches work
//! or touches the existing Teams backend. Only this boundary imports bridge DTOs.

pub(crate) mod events;
pub(crate) mod projection;
mod text;

#[cfg(test)]
mod tests;
