//! Backend-neutral presentation models and frontend request/result context.
//!
//! This module does not own networking, authentication or canonical message state.

pub(crate) mod models;

// Stage 5B scaffold: current Teams dispatch is intentionally unchanged. Remove
// this narrow allowance when the bridge adapter starts using request contexts.
#[allow(dead_code)]
pub(crate) mod requests;
