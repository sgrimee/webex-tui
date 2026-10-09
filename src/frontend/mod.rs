//! Backend-neutral presentation models and frontend request/result context.
//!
//! This module does not own networking, authentication or canonical message state.

pub(crate) mod models;

// Stage 5B scaffold: current Teams dispatch is intentionally unchanged. Remove
// these narrow allowances as the bridge adapter starts consuming each boundary.
#[allow(dead_code)]
pub(crate) mod continuity;
#[allow(dead_code)]
pub(crate) mod data;
#[allow(dead_code)]
pub(crate) mod envelopes;
#[allow(dead_code)]
pub(crate) mod events;
#[allow(dead_code)]
pub(crate) mod lifecycle;
#[allow(dead_code)]
pub(crate) mod requests;
#[allow(dead_code)]
pub(crate) mod threads;
