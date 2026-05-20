//! Osaka submission shapes — identical to Prague.
//!
//! The fork distinction (cell proofs on blob bundles) lives on the response
//! side of `/blobs/v2`–`/blobs/v3` in sub-project 3. The submission types here
//! are direct re-exports so the per-fork dispatch table stays uniform.

pub use crate::engine_rest::types::prague::{
    ExecutionPayload, ExecutionPayloadEnvelope, PayloadAttributes,
};
