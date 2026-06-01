//! Osaka submission shapes — identical to Prague.
//!
//! The fork distinction (cell proofs on blob bundles) lives on the response
//! side of `/blobs/v2`–`/blobs/v3` in sub-project 3. These are direct re-exports
//! for readability and so tests can name the Osaka types explicitly; the
//! `payloads`/`forkchoice` handlers dispatch `Fork::Osaka` straight to the
//! `prague::` types rather than going through this module.

pub use crate::engine_rest::types::prague::{
    ExecutionPayload, ExecutionPayloadEnvelope, PayloadAttributes,
};
