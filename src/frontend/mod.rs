//! Backend-neutral presentation models and frontend request/result context.
//!
//! This module does not own networking, authentication or canonical message state.

pub(crate) mod models;

pub(crate) mod continuity;
pub(crate) mod data;
pub(crate) mod editor_submission;
pub(crate) mod editor_write;
pub(crate) mod envelopes;
pub(crate) mod events;
pub(crate) mod lifecycle;
pub(crate) mod message_view;
pub(crate) mod requests;
pub(crate) mod threads;
pub(crate) mod write_rules;
