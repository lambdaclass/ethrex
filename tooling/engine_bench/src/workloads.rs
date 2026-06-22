//! Per-workload run loops. Each fires N iterations of one workload on one
//! transport, recording wall-time + bytes per iteration.
//!
//! Timed window (identical on both transports):
//!   typed request struct → wire bytes → HTTP round-trip → raw response bytes.
//! Response decoding is excluded on both sides — it only happens after the
//! timer stops, to count hits. Pure encode/decode costs are measured by the
//! criterion bench (`cargo bench -p ethrex-rpc --bench engine_transport`).

use crate::cli::{ForkArg, Transport, Workload};
use crate::fixtures;
use crate::transports::{json_rpc, rest_ssz};
use ethrex_common::H256;
use eyre::Result;
use libssz::{SszDecode, SszEncode};
use reqwest::{Client, Method};
use serde_json::Value;
use std::time::Instant;

pub const ZERO_HASH: &str = "0x0000000000000000000000000000000000000000000000000000000000000000";

/// Fallback payload id when no real one is available; the server answers with
/// an "unknown payload" error, so getPayload degrades to an error round-trip.
pub const SYNTHETIC_PAYLOAD_ID: &str = "0x0102030405060708";

/// Run-time configuration resolved once per fork in main (see `setup`).
#[derive(Clone)]
pub struct WorkloadContext {
    pub fork: ForkArg,
    /// Blobs endpoint version (2 or 3).
    pub blobs_version: u8,
    pub payload_id: String,
    /// Versioned hashes for the blobs workload. Random unless supplied via
    /// `--blob-hashes-file`, in which case they can actually hit the pool.
    pub blob_hashes: Vec<H256>,
    pub bodies_from: u64,
    pub bodies_count: u64,
    pub iterations: usize,
    /// Leading iterations dropped from the results.
    pub warmup: usize,
}

#[derive(Debug, Clone)]
pub struct IterationRecord {
    pub fork: ForkArg,
    pub workload: Workload,
    /// Blobs endpoint version, set for the blobs workload only.
    pub blobs_version: Option<u8>,
    pub transport: Transport,
    pub iteration: usize,
    pub bytes_sent: usize,
    pub bytes_received: usize,
    pub wall_time_us: u128,
    pub http_status: u16,
    /// Non-null / available entries in the response, for workloads whose
    /// response is a nullable list (blobs, bodies). `None` = not applicable
    /// or undecodable.
    pub hits: Option<usize>,
}

pub async fn run_one(
    client: &Client,
    url_base: &str,
    secret: &[u8],
    ctx: &WorkloadContext,
    workload: Workload,
    transport: Transport,
) -> Result<Vec<IterationRecord>> {
    let mut out = Vec::with_capacity(ctx.iterations);
    for i in 0..(ctx.warmup + ctx.iterations) {
        // Refresh JWT per iteration (cheap and stays well inside the ±60s window).
        let token = crate::jwt::mint(secret)?;
        let mut rec = match (workload, transport) {
            (Workload::NewPayload, Transport::Json) => {
                run_newpayload_json(client, url_base, &token, ctx).await?
            }
            (Workload::NewPayload, Transport::Ssz) => {
                run_newpayload_ssz(client, url_base, &token, ctx).await?
            }
            (Workload::GetPayload, Transport::Json) => {
                run_getpayload_json(client, url_base, &token, ctx).await?
            }
            (Workload::GetPayload, Transport::Ssz) => {
                run_getpayload_ssz(client, url_base, &token, ctx).await?
            }
            (Workload::Blobs, Transport::Json) => {
                run_blobs_json(client, url_base, &token, ctx).await?
            }
            (Workload::Blobs, Transport::Ssz) => {
                run_blobs_ssz(client, url_base, &token, ctx).await?
            }
            (Workload::Bodies, Transport::Json) => {
                run_bodies_json(client, url_base, &token, ctx).await?
            }
            (Workload::Bodies, Transport::Ssz) => {
                run_bodies_ssz(client, url_base, &token, ctx).await?
            }
        };
        // The first `warmup` iterations absorb connection setup (TCP + h2c
        // handshake) and server cold paths; they are not recorded.
        if i >= ctx.warmup {
            rec.iteration = i - ctx.warmup;
            out.push(rec);
        }
    }
    Ok(out)
}

/// Count non-null entries of a JSON-RPC `result` array (blobs/bodies). A null
/// result (blobs v2 all-or-nothing miss) counts as zero hits.
fn json_result_hits(resp: &json_rpc::JsonResponse) -> Option<usize> {
    let v = resp.json()?;
    let result = v.get("result")?;
    if result.is_null() {
        return Some(0);
    }
    let count = result.as_array()?.iter().filter(|e| !e.is_null()).count();
    Some(count)
}

async fn run_newpayload_json(
    client: &Client,
    url_base: &str,
    token: &str,
    ctx: &WorkloadContext,
) -> Result<IterationRecord> {
    // The synthetic block hash is invalid on purpose: the server fully decodes
    // the payload, then rejects it, isolating transport + decode from
    // execution. Requests param (V4/V5) is empty, matching the SSZ envelopes.
    use fixtures::PayloadEra;
    let payload = match ctx.fork {
        ForkArg::Paris => fixtures::payload_json(
            fixtures::DEFAULT_SEED,
            fixtures::DEFAULT_TX_COUNT,
            PayloadEra::Paris,
        ),
        ForkArg::Shanghai => fixtures::payload_json(
            fixtures::DEFAULT_SEED,
            fixtures::DEFAULT_TX_COUNT,
            PayloadEra::Shanghai,
        ),
        ForkArg::Cancun | ForkArg::Prague | ForkArg::Osaka => fixtures::payload_json(
            fixtures::DEFAULT_SEED,
            fixtures::DEFAULT_TX_COUNT,
            PayloadEra::Cancun,
        ),
        ForkArg::Amsterdam => fixtures::amsterdam_payload_json(
            fixtures::DEFAULT_SEED,
            fixtures::DEFAULT_TX_COUNT,
            fixtures::DEFAULT_BAL_ACCOUNTS,
        ),
    };
    let t0 = Instant::now();
    // Param arity follows the spec: V1/V2 take [payload], V3 adds versioned
    // hashes + beacon root, V4/V5 add execution requests.
    let resp = match ctx.fork {
        ForkArg::Paris => {
            json_rpc::call(client, url_base, token, "engine_newPayloadV1", (&payload,)).await?
        }
        ForkArg::Shanghai => {
            json_rpc::call(client, url_base, token, "engine_newPayloadV2", (&payload,)).await?
        }
        ForkArg::Cancun => {
            json_rpc::call(
                client,
                url_base,
                token,
                "engine_newPayloadV3",
                (&payload, Vec::<Value>::new(), ZERO_HASH),
            )
            .await?
        }
        ForkArg::Prague | ForkArg::Osaka => {
            json_rpc::call(
                client,
                url_base,
                token,
                "engine_newPayloadV4",
                (
                    &payload,
                    Vec::<Value>::new(),
                    ZERO_HASH,
                    Vec::<Value>::new(),
                ),
            )
            .await?
        }
        ForkArg::Amsterdam => {
            json_rpc::call(
                client,
                url_base,
                token,
                "engine_newPayloadV5",
                (
                    &payload,
                    Vec::<Value>::new(),
                    ZERO_HASH,
                    Vec::<Value>::new(),
                ),
            )
            .await?
        }
    };
    let wall_time_us = t0.elapsed().as_micros();
    Ok(IterationRecord {
        fork: ctx.fork,
        blobs_version: None,
        workload: Workload::NewPayload,
        transport: Transport::Json,
        iteration: 0,
        bytes_sent: resp.bytes_sent,
        bytes_received: resp.bytes_received,
        wall_time_us,
        http_status: resp.status,
        hits: None,
    })
}

async fn run_newpayload_ssz(
    client: &Client,
    url_base: &str,
    token: &str,
    ctx: &WorkloadContext,
) -> Result<IterationRecord> {
    let url = format!("{url_base}/engine/v2/{}/payloads", ctx.fork.path());
    let (body, t0) = match ctx.fork {
        ForkArg::Paris => {
            let envelope =
                fixtures::paris_newpayload_ssz(fixtures::DEFAULT_SEED, fixtures::DEFAULT_TX_COUNT);
            let t0 = Instant::now();
            (envelope.to_ssz(), t0)
        }
        ForkArg::Shanghai => {
            let envelope = fixtures::shanghai_newpayload_ssz(
                fixtures::DEFAULT_SEED,
                fixtures::DEFAULT_TX_COUNT,
            );
            let t0 = Instant::now();
            (envelope.to_ssz(), t0)
        }
        ForkArg::Cancun => {
            let envelope =
                fixtures::cancun_newpayload_ssz(fixtures::DEFAULT_SEED, fixtures::DEFAULT_TX_COUNT);
            let t0 = Instant::now();
            (envelope.to_ssz(), t0)
        }
        ForkArg::Prague | ForkArg::Osaka => {
            let envelope =
                fixtures::prague_newpayload_ssz(fixtures::DEFAULT_SEED, fixtures::DEFAULT_TX_COUNT);
            let t0 = Instant::now();
            (envelope.to_ssz(), t0)
        }
        ForkArg::Amsterdam => {
            let envelope = fixtures::amsterdam_newpayload_ssz(
                fixtures::DEFAULT_SEED,
                fixtures::DEFAULT_TX_COUNT,
                fixtures::DEFAULT_BAL_ACCOUNTS,
            );
            let t0 = Instant::now();
            (envelope.to_ssz(), t0)
        }
    };
    let resp = rest_ssz::call(client, Method::POST, &url, token, body).await?;
    let wall_time_us = t0.elapsed().as_micros();
    Ok(IterationRecord {
        fork: ctx.fork,
        blobs_version: None,
        workload: Workload::NewPayload,
        transport: Transport::Ssz,
        iteration: 0,
        bytes_sent: resp.bytes_sent,
        bytes_received: resp.bytes_received,
        wall_time_us,
        http_status: resp.status,
        hits: None,
    })
}

async fn run_getpayload_json(
    client: &Client,
    url_base: &str,
    token: &str,
    ctx: &WorkloadContext,
) -> Result<IterationRecord> {
    let method = match ctx.fork {
        ForkArg::Paris => "engine_getPayloadV1",
        ForkArg::Shanghai => "engine_getPayloadV2",
        ForkArg::Cancun => "engine_getPayloadV3",
        ForkArg::Prague => "engine_getPayloadV4",
        ForkArg::Osaka => "engine_getPayloadV5",
        ForkArg::Amsterdam => "engine_getPayloadV6",
    };
    let t0 = Instant::now();
    let resp = json_rpc::call(client, url_base, token, method, (ctx.payload_id.as_str(),)).await?;
    let wall_time_us = t0.elapsed().as_micros();
    Ok(IterationRecord {
        fork: ctx.fork,
        blobs_version: None,
        workload: Workload::GetPayload,
        transport: Transport::Json,
        iteration: 0,
        bytes_sent: resp.bytes_sent,
        bytes_received: resp.bytes_received,
        wall_time_us,
        http_status: resp.status,
        hits: None,
    })
}

async fn run_getpayload_ssz(
    client: &Client,
    url_base: &str,
    token: &str,
    ctx: &WorkloadContext,
) -> Result<IterationRecord> {
    let url = format!(
        "{url_base}/engine/v2/{}/payloads/{}",
        ctx.fork.path(),
        ctx.payload_id
    );
    let t0 = Instant::now();
    let resp = rest_ssz::call(client, Method::GET, &url, token, vec![]).await?;
    let wall_time_us = t0.elapsed().as_micros();
    Ok(IterationRecord {
        fork: ctx.fork,
        blobs_version: None,
        workload: Workload::GetPayload,
        transport: Transport::Ssz,
        iteration: 0,
        bytes_sent: resp.bytes_sent,
        bytes_received: resp.bytes_received,
        wall_time_us,
        http_status: resp.status,
        hits: None,
    })
}

async fn run_blobs_json(
    client: &Client,
    url_base: &str,
    token: &str,
    ctx: &WorkloadContext,
) -> Result<IterationRecord> {
    let method = match ctx.blobs_version {
        1 => "engine_getBlobsV1",
        2 => "engine_getBlobsV2",
        _ => "engine_getBlobsV3",
    };
    let t0 = Instant::now();
    let resp = json_rpc::call(client, url_base, token, method, (&ctx.blob_hashes,)).await?;
    let wall_time_us = t0.elapsed().as_micros();
    let hits = json_result_hits(&resp);
    Ok(IterationRecord {
        fork: ctx.fork,
        blobs_version: Some(ctx.blobs_version),
        workload: Workload::Blobs,
        transport: Transport::Json,
        iteration: 0,
        bytes_sent: resp.bytes_sent,
        bytes_received: resp.bytes_received,
        wall_time_us,
        http_status: resp.status,
        hits,
    })
}

async fn run_blobs_ssz(
    client: &Client,
    url_base: &str,
    token: &str,
    ctx: &WorkloadContext,
) -> Result<IterationRecord> {
    use ethrex_rpc::engine_rest::types::blobs::{
        BlobsV1Response, BlobsV2Response, VersionedHashList,
    };
    let hashes_arr: Vec<[u8; 32]> = ctx.blob_hashes.iter().map(|h| h.0).collect();
    let req: VersionedHashList = hashes_arr
        .try_into()
        .expect("blob hashes capped at MAX_BLOBS_REQUEST in setup");
    let url = format!("{url_base}/engine/v2/blobs/v{}", ctx.blobs_version);
    let t0 = Instant::now();
    let body = req.to_ssz();
    let resp = rest_ssz::call(client, Method::POST, &url, token, body).await?;
    let wall_time_us = t0.elapsed().as_micros();
    // 204 = all-or-nothing miss (v2): zero hits by definition.
    // `BlobsV2Response` and `BlobsV3Response` are the same SSZ type.
    let hits = match (resp.status, ctx.blobs_version) {
        (204, _) => Some(0),
        (200, 1) => BlobsV1Response::from_ssz_bytes(&resp.body)
            .ok()
            .map(|r| r.entries.iter().filter(|e| e.available).count()),
        (200, _) => BlobsV2Response::from_ssz_bytes(&resp.body)
            .ok()
            .map(|r| r.entries.iter().filter(|e| e.available).count()),
        _ => None,
    };
    Ok(IterationRecord {
        fork: ctx.fork,
        blobs_version: Some(ctx.blobs_version),
        workload: Workload::Blobs,
        transport: Transport::Ssz,
        iteration: 0,
        bytes_sent: resp.bytes_sent,
        bytes_received: resp.bytes_received,
        wall_time_us,
        http_status: resp.status,
        hits,
    })
}

async fn run_bodies_json(
    client: &Client,
    url_base: &str,
    token: &str,
    ctx: &WorkloadContext,
) -> Result<IterationRecord> {
    let method = match ctx.fork {
        ForkArg::Amsterdam => "engine_getPayloadBodiesByRangeV2",
        _ => "engine_getPayloadBodiesByRangeV1",
    };
    let t0 = Instant::now();
    let resp = json_rpc::call(
        client,
        url_base,
        token,
        method,
        (
            format!("0x{:x}", ctx.bodies_from),
            format!("0x{:x}", ctx.bodies_count),
        ),
    )
    .await?;
    let wall_time_us = t0.elapsed().as_micros();
    let hits = json_result_hits(&resp);
    Ok(IterationRecord {
        fork: ctx.fork,
        blobs_version: None,
        workload: Workload::Bodies,
        transport: Transport::Json,
        iteration: 0,
        bytes_sent: resp.bytes_sent,
        bytes_received: resp.bytes_received,
        wall_time_us,
        http_status: resp.status,
        hits,
    })
}

async fn run_bodies_ssz(
    client: &Client,
    url_base: &str,
    token: &str,
    ctx: &WorkloadContext,
) -> Result<IterationRecord> {
    use ethrex_rpc::engine_rest::types::bodies::{
        BodiesResponseAmsterdam, BodiesResponseParis, BodiesResponseShanghai,
    };
    let url = format!(
        "{url_base}/engine/v2/{}/bodies?from={}&count={}",
        ctx.fork.path(),
        ctx.bodies_from,
        ctx.bodies_count
    );
    let t0 = Instant::now();
    let resp = rest_ssz::call(client, Method::GET, &url, token, vec![]).await?;
    let wall_time_us = t0.elapsed().as_micros();
    let hits = if resp.status == 200 {
        match ctx.fork {
            ForkArg::Paris => BodiesResponseParis::from_ssz_bytes(&resp.body)
                .ok()
                .map(|r| r.entries.iter().filter(|e| e.available).count()),
            ForkArg::Shanghai | ForkArg::Cancun | ForkArg::Prague | ForkArg::Osaka => {
                BodiesResponseShanghai::from_ssz_bytes(&resp.body)
                    .ok()
                    .map(|r| r.entries.iter().filter(|e| e.available).count())
            }
            ForkArg::Amsterdam => BodiesResponseAmsterdam::from_ssz_bytes(&resp.body)
                .ok()
                .map(|r| r.entries.iter().filter(|e| e.available).count()),
        }
    } else {
        None
    };
    Ok(IterationRecord {
        fork: ctx.fork,
        blobs_version: None,
        workload: Workload::Bodies,
        transport: Transport::Ssz,
        iteration: 0,
        bytes_sent: resp.bytes_sent,
        bytes_received: resp.bytes_received,
        wall_time_us,
        http_status: resp.status,
        hits,
    })
}
