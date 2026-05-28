//! GET /capabilities — advertises supported forks, endpoints, and request limits.
//!
//! Spec: replaces `engine_exchangeCapabilities`.

use std::collections::BTreeMap;

use axum::Json;
use serde::{Deserialize, Serialize};

/// Per-endpoint limits advertised to the CL.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EndpointLimits {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_count: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Capabilities {
    pub supported_forks: Vec<String>,
    pub endpoints: BTreeMap<String, EndpointLimits>,
    pub blobs: Vec<String>,
}

pub const PAYLOAD_MAX_BYTES: u64 = 268_435_456; // 256 MiB, matches DefaultBodyLimit in rpc.rs
pub const BODIES_MAX_COUNT: u32 = 128;
pub const BLOBS_MAX_COUNT: u32 = 128;

pub fn capabilities() -> Capabilities {
    let mut endpoints = BTreeMap::new();
    endpoints.insert(
        "POST /{fork}/payloads".to_string(),
        EndpointLimits {
            max_bytes: Some(PAYLOAD_MAX_BYTES),
            max_count: None,
        },
    );
    endpoints.insert(
        "GET /{fork}/payloads/{id}".to_string(),
        EndpointLimits {
            max_bytes: Some(PAYLOAD_MAX_BYTES),
            max_count: None,
        },
    );
    endpoints.insert(
        "POST /{fork}/forkchoice".to_string(),
        EndpointLimits {
            // The only enforced body cap is the shared 256 MiB DefaultBodyLimit
            // applied to the whole authrpc router in rpc.rs; advertise that
            // rather than a smaller per-route limit we don't actually enforce.
            max_bytes: Some(PAYLOAD_MAX_BYTES),
            max_count: None,
        },
    );
    endpoints.insert(
        "POST /{fork}/bodies/hash".to_string(),
        EndpointLimits {
            max_bytes: None,
            max_count: Some(BODIES_MAX_COUNT),
        },
    );
    endpoints.insert(
        "GET /{fork}/bodies".to_string(),
        EndpointLimits {
            max_bytes: None,
            max_count: Some(BODIES_MAX_COUNT),
        },
    );
    for v in ["v1", "v2", "v3", "v4"] {
        endpoints.insert(
            format!("POST /blobs/{v}"),
            EndpointLimits {
                max_bytes: None,
                max_count: Some(BLOBS_MAX_COUNT),
            },
        );
    }

    Capabilities {
        supported_forks: vec![
            "paris".into(),
            "shanghai".into(),
            "cancun".into(),
            "prague".into(),
            "osaka".into(),
            "amsterdam".into(),
        ],
        endpoints,
        blobs: vec!["v1".into(), "v2".into(), "v3".into(), "v4".into()],
    }
}

pub async fn get_capabilities() -> Json<Capabilities> {
    Json(capabilities())
}
