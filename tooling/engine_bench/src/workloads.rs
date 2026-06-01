//! Per-workload run loops. Each fires N iterations of one workload on one
//! transport, recording wall-time + bytes per iteration.

use crate::cli::{Transport, Workload};
use crate::fixtures;
use crate::transports::{json_rpc, rest_ssz};
use eyre::Result;
use libssz::SszEncode;
use reqwest::{Client, Method};
use serde_json::json;
use std::time::Instant;

#[derive(Debug, Clone)]
pub struct IterationRecord {
    pub workload: Workload,
    pub transport: Transport,
    pub iteration: usize,
    pub bytes_sent: usize,
    pub bytes_received: usize,
    pub wall_time_us: u128,
    pub http_status: u16,
}

pub async fn run_one(
    client: &Client,
    url_base: &str,
    secret: &[u8],
    workload: Workload,
    transport: Transport,
    iterations: usize,
) -> Result<Vec<IterationRecord>> {
    let mut out = Vec::with_capacity(iterations);
    for i in 0..iterations {
        // Refresh JWT per iteration (cheap and stays well inside the ±60s window).
        let token = crate::jwt::mint(secret)?;
        let rec = match (workload, transport) {
            (Workload::NewPayload, Transport::Json) => {
                run_newpayload_json(client, url_base, &token, i).await?
            }
            (Workload::NewPayload, Transport::Ssz) => {
                run_newpayload_ssz(client, url_base, &token, i).await?
            }
            (Workload::GetPayload, Transport::Json) => {
                run_getpayload_json(client, url_base, &token, i).await?
            }
            (Workload::GetPayload, Transport::Ssz) => {
                run_getpayload_ssz(client, url_base, &token, i).await?
            }
            (Workload::Blobs, Transport::Json) => {
                run_blobs_json(client, url_base, &token, i).await?
            }
            (Workload::Blobs, Transport::Ssz) => run_blobs_ssz(client, url_base, &token, i).await?,
            (Workload::Bodies, Transport::Json) => {
                run_bodies_json(client, url_base, &token, i).await?
            }
            (Workload::Bodies, Transport::Ssz) => {
                run_bodies_ssz(client, url_base, &token, i).await?
            }
        };
        out.push(rec);
    }
    Ok(out)
}

async fn run_newpayload_json(
    client: &Client,
    url_base: &str,
    token: &str,
    i: usize,
) -> Result<IterationRecord> {
    let payload = fixtures::cancun_payload_json(
        fixtures::DEFAULT_SEED,
        fixtures::DEFAULT_TX_COUNT,
        fixtures::DEFAULT_BLOB_HASH_COUNT,
    );
    let params = json!([
        payload,
        [],
        "0x0000000000000000000000000000000000000000000000000000000000000000"
    ]);
    let t0 = Instant::now();
    let resp = json_rpc::call(client, url_base, token, "engine_newPayloadV3", params).await?;
    let wall_time_us = t0.elapsed().as_micros();
    Ok(IterationRecord {
        workload: Workload::NewPayload,
        transport: Transport::Json,
        iteration: i,
        bytes_sent: resp.bytes_sent,
        bytes_received: resp.bytes_received,
        wall_time_us,
        http_status: resp.status,
    })
}

async fn run_newpayload_ssz(
    client: &Client,
    url_base: &str,
    token: &str,
    i: usize,
) -> Result<IterationRecord> {
    let envelope = fixtures::cancun_payload_ssz(
        fixtures::DEFAULT_SEED,
        fixtures::DEFAULT_TX_COUNT,
        fixtures::DEFAULT_BLOB_HASH_COUNT,
    );
    let body = envelope.to_ssz();
    let url = format!("{url_base}/engine/v2/cancun/payloads");
    let t0 = Instant::now();
    let resp = rest_ssz::call(client, Method::POST, &url, token, body).await?;
    let wall_time_us = t0.elapsed().as_micros();
    Ok(IterationRecord {
        workload: Workload::NewPayload,
        transport: Transport::Ssz,
        iteration: i,
        bytes_sent: resp.bytes_sent,
        bytes_received: resp.bytes_received,
        wall_time_us,
        http_status: resp.status,
    })
}

// getPayload: harness uses a synthetic 0x0102030405060708 payloadId; expects 404.
// Documented in the design doc as the round-trip fallback.

async fn run_getpayload_json(
    client: &Client,
    url_base: &str,
    token: &str,
    i: usize,
) -> Result<IterationRecord> {
    let params = json!(["0x0102030405060708"]);
    let t0 = Instant::now();
    let resp = json_rpc::call(client, url_base, token, "engine_getPayloadV3", params).await?;
    let wall_time_us = t0.elapsed().as_micros();
    Ok(IterationRecord {
        workload: Workload::GetPayload,
        transport: Transport::Json,
        iteration: i,
        bytes_sent: resp.bytes_sent,
        bytes_received: resp.bytes_received,
        wall_time_us,
        http_status: resp.status,
    })
}

async fn run_getpayload_ssz(
    client: &Client,
    url_base: &str,
    token: &str,
    i: usize,
) -> Result<IterationRecord> {
    let url = format!("{url_base}/engine/v2/cancun/payloads/0x0102030405060708");
    let t0 = Instant::now();
    let resp = rest_ssz::call(client, Method::GET, &url, token, vec![]).await?;
    let wall_time_us = t0.elapsed().as_micros();
    Ok(IterationRecord {
        workload: Workload::GetPayload,
        transport: Transport::Ssz,
        iteration: i,
        bytes_sent: resp.bytes_sent,
        bytes_received: resp.bytes_received,
        wall_time_us,
        http_status: resp.status,
    })
}

async fn run_blobs_json(
    client: &Client,
    url_base: &str,
    token: &str,
    i: usize,
) -> Result<IterationRecord> {
    let hashes = fixtures::blob_versioned_hashes(
        fixtures::DEFAULT_SEED,
        fixtures::DEFAULT_BLOB_REQUEST_COUNT,
    );
    let params = json!([hashes]);
    let t0 = Instant::now();
    let resp = json_rpc::call(client, url_base, token, "engine_getBlobsV1", params).await?;
    let wall_time_us = t0.elapsed().as_micros();
    Ok(IterationRecord {
        workload: Workload::Blobs,
        transport: Transport::Json,
        iteration: i,
        bytes_sent: resp.bytes_sent,
        bytes_received: resp.bytes_received,
        wall_time_us,
        http_status: resp.status,
    })
}

async fn run_blobs_ssz(
    client: &Client,
    url_base: &str,
    token: &str,
    i: usize,
) -> Result<IterationRecord> {
    use ethrex_rpc::engine_rest::types::blobs::VersionedHashList;
    let hashes = fixtures::blob_versioned_hashes(
        fixtures::DEFAULT_SEED,
        fixtures::DEFAULT_BLOB_REQUEST_COUNT,
    );
    let hashes_arr: Vec<[u8; 32]> = hashes.iter().map(|h| h.0).collect();
    let req: VersionedHashList = hashes_arr.try_into().expect("blob hashes fit");
    let body = req.to_ssz();
    let url = format!("{url_base}/engine/v2/blobs/v1");
    let t0 = Instant::now();
    let resp = rest_ssz::call(client, Method::POST, &url, token, body).await?;
    let wall_time_us = t0.elapsed().as_micros();
    Ok(IterationRecord {
        workload: Workload::Blobs,
        transport: Transport::Ssz,
        iteration: i,
        bytes_sent: resp.bytes_sent,
        bytes_received: resp.bytes_received,
        wall_time_us,
        http_status: resp.status,
    })
}

async fn run_bodies_json(
    client: &Client,
    url_base: &str,
    token: &str,
    i: usize,
) -> Result<IterationRecord> {
    let params = json!(["0x1", format!("0x{:x}", fixtures::DEFAULT_BODIES_COUNT)]);
    let t0 = Instant::now();
    let resp = json_rpc::call(
        client,
        url_base,
        token,
        "engine_getPayloadBodiesByRangeV1",
        params,
    )
    .await?;
    let wall_time_us = t0.elapsed().as_micros();
    Ok(IterationRecord {
        workload: Workload::Bodies,
        transport: Transport::Json,
        iteration: i,
        bytes_sent: resp.bytes_sent,
        bytes_received: resp.bytes_received,
        wall_time_us,
        http_status: resp.status,
    })
}

async fn run_bodies_ssz(
    client: &Client,
    url_base: &str,
    token: &str,
    i: usize,
) -> Result<IterationRecord> {
    let url = format!(
        "{url_base}/engine/v2/cancun/bodies?from=1&count={}",
        fixtures::DEFAULT_BODIES_COUNT
    );
    let t0 = Instant::now();
    let resp = rest_ssz::call(client, Method::GET, &url, token, vec![]).await?;
    let wall_time_us = t0.elapsed().as_micros();
    Ok(IterationRecord {
        workload: Workload::Bodies,
        transport: Transport::Ssz,
        iteration: i,
        bytes_sent: resp.bytes_sent,
        bytes_received: resp.bytes_received,
        wall_time_us,
        http_status: resp.status,
    })
}
