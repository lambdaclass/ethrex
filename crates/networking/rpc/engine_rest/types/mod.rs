//! SSZ wire types for the engine REST API.
//!
//! `common` holds fork-invariant types (PayloadStatus, ForkchoiceState, PayloadId).
//! Per-fork modules hold the ExecutionPayload, ExecutionPayloadEnvelope, and
//! PayloadAttributes shapes that grow progressively with each fork.

pub mod amsterdam;
pub mod blobs;
pub mod bodies;
pub mod cancun;
pub mod common;
pub mod conversions;
pub mod forkchoice_update;
pub mod osaka;
pub mod paris;
pub mod prague;
pub mod shanghai;
