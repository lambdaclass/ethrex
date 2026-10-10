//! GET /capabilities — advertises supported forks, fork-scoped and unscoped
//! endpoints, independently-versioned blob revisions, and per-resource limits.
//!
//! Spec (execution-apis #793, `refactor.md § GET /engine/v1/capabilities`):
//! replaces `engine_exchangeCapabilities`. JSON shape:
//! ```json
//! {
//!   "supported_forks": ["paris", ...],
//!   "fork_scoped_endpoints": ["payloads", "forkchoice", "bodies"],
//!   "independently_versioned": { "blobs": ["v1", ...] },
//!   "unscoped_endpoints": ["capabilities", "identity"],
//!   "limits": { "bodies.max_count": N, "blobs.max_versioned_hashes": N, "payload.max_bytes": N }
//! }
//! ```
//! `limits` uses flat dot-notation keys with scalar values, matching the spec
//! and the Nethermind/consensoor implementations (not method+path keys).

use std::collections::BTreeMap;

use axum::Json;
use serde::{Deserialize, Serialize};

use crate::engine_rest::types::blobs::MAX_BLOBS_REQUEST;
use crate::engine_rest::types::bodies::MAX_BODIES_PER_REQUEST;

/// Blob endpoints are versioned independently of the fork (`/blobs/vN`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndependentlyVersioned {
    pub blobs: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Capabilities {
    pub supported_forks: Vec<String>,
    pub fork_scoped_endpoints: Vec<String>,
    pub independently_versioned: IndependentlyVersioned,
    pub unscoped_endpoints: Vec<String>,
    /// Flat dot-notation resource limits per `refactor.md` (e.g. `payload.max_bytes`,
    /// `bodies.max_count`, `blobs.max_versioned_hashes`).
    pub limits: BTreeMap<String, u64>,
}

// 256 MiB, matches the DefaultBodyLimit in rpc.rs. Operator-chosen and
// deliberately above the spec's MAX_REQUEST_BODY_SIZE floor (2**26 = 64 MiB,
// execution-apis #793): ethrex accepts up to 256 MiB on the auth port for parity
// with the JSON-RPC engine path. Advertising a larger limit than the spec is the
// safe direction (the EL is more permissive); the interop hazard runs the other way.
pub const PAYLOAD_MAX_BYTES: u64 = 268_435_456;
// Mirror the SSZ list bounds (the single source of truth in `types/`) as u32, so
// the advertised capability and the request-cap guards can never drift from the
// wire-type capacity. `MAX_BODIES_REQUEST = 2**5`, `MAX_BLOBS_REQUEST = 128`.
pub const BODIES_MAX_COUNT: u32 = MAX_BODIES_PER_REQUEST as u32;
pub const BLOBS_MAX_COUNT: u32 = MAX_BLOBS_REQUEST as u32;

pub fn capabilities() -> Capabilities {
    let limits = BTreeMap::from([
        ("bodies.max_count".to_string(), BODIES_MAX_COUNT as u64),
        (
            "blobs.max_versioned_hashes".to_string(),
            BLOBS_MAX_COUNT as u64,
        ),
        ("payload.max_bytes".to_string(), PAYLOAD_MAX_BYTES),
    ]);

    Capabilities {
        supported_forks: vec![
            "paris".into(),
            "shanghai".into(),
            "cancun".into(),
            "prague".into(),
            "osaka".into(),
            "amsterdam".into(),
        ],
        fork_scoped_endpoints: vec!["payloads".into(), "forkchoice".into(), "bodies".into()],
        independently_versioned: IndependentlyVersioned {
            blobs: vec!["v1".into(), "v2".into(), "v3".into(), "v4".into()],
        },
        unscoped_endpoints: vec!["capabilities".into(), "identity".into()],
        limits,
    }
}

pub async fn get_capabilities() -> Json<Capabilities> {
    Json(capabilities())
}
