//! Real handlers for `POST /{fork}/payloads` and `GET /{fork}/payloads/{id}`.

use std::str::FromStr;

use axum::RequestExt;
use axum::extract::{Path, Request, State};
use axum::response::{IntoResponse, Response};
use bytes::Bytes;
use ethrex_blockchain::error::ChainError;
use ethrex_common::U256;
use ethrex_common::types::blobs_bundle::BlobsBundle;
use ethrex_common::types::{Block, Fork};
use ethrex_rlp::encode::RLPEncode;
use libssz_types::{SszList, SszVector};

use crate::engine::payload::{
    handle_new_payload_v1_v2, handle_new_payload_v3, handle_new_payload_v4,
};
use crate::engine_rest::error::ProblemJson;
use crate::engine_rest::extractors::{decode_ssz, is_length_limit_error};
use crate::engine_rest::fork_path::{ForkPath, parse_fork_segment};
use crate::engine_rest::handlers::helpers::check_content_type;
use crate::engine_rest::responses::SszBody;
use crate::engine_rest::types::blobs::BYTES_PER_BLOB;
use crate::engine_rest::types::built_payload::{
    BlobsBundleV1, BlobsBundleV2, BuiltPayloadAmsterdam, BuiltPayloadCancun, BuiltPayloadOsaka,
    BuiltPayloadParis, BuiltPayloadPrague, BuiltPayloadShanghai, MAX_BLOB_COMMITMENTS_PER_BLOCK,
};
use crate::engine_rest::types::common::{
    Bytes20, PayloadId, PayloadStatus as SszPayloadStatus, PayloadStatusCode,
};
use crate::engine_rest::types::conversions::{DecodedNewPayload, EngineCall, IntoEngineCall};
use crate::engine_rest::types::{amsterdam, cancun, paris, prague, shanghai};
use crate::rpc::RpcApiContext;
use crate::types::payload::PayloadValidationStatus;

// ── submit_payload ────────────────────────────────────────────────────────────

pub async fn submit_payload(
    ForkPath(fork): ForkPath,
    State(ctx): State<RpcApiContext>,
    req: Request,
) -> Response {
    if let Err(p) = check_content_type(req.headers()) {
        return p.into_response();
    }
    // `with_limited_body()` honours the DefaultBodyLimit middleware (256 MiB on
    // the auth port). Reading via `req.into_body()` would bypass that cap.
    let body = match axum::body::to_bytes(req.with_limited_body().into_body(), usize::MAX).await {
        Ok(b) => b,
        Err(e) => {
            if is_length_limit_error(&e) {
                return ProblemJson::payload_too_large("request body exceeds configured limit")
                    .into_response();
            }
            return ProblemJson::bad_request(&format!("failed to read body: {e}")).into_response();
        }
    };
    match fork {
        Fork::Paris => decode_and_submit::<paris::ExecutionPayloadEnvelope>(body, ctx).await,
        Fork::Shanghai => decode_and_submit::<shanghai::ExecutionPayloadEnvelope>(body, ctx).await,
        Fork::Cancun => decode_and_submit::<cancun::ExecutionPayloadEnvelope>(body, ctx).await,
        Fork::Prague => decode_and_submit::<prague::ExecutionPayloadEnvelope>(body, ctx).await,
        Fork::Osaka => {
            // Osaka re-exports Prague's envelope (see types/osaka.rs); same shape.
            decode_and_submit::<prague::ExecutionPayloadEnvelope>(body, ctx).await
        }
        Fork::Amsterdam => {
            decode_and_submit::<amsterdam::ExecutionPayloadEnvelope>(body, ctx).await
        }
        // Unreachable: ForkPath's parse_fork_segment rejects all non-spec forks
        // with 400 before the handler runs.
        _ => unreachable!("ForkPath extractor restricts to the 6 spec forks"),
    }
}

async fn decode_and_submit<T>(body: Bytes, ctx: RpcApiContext) -> Response
where
    T: libssz::SszDecode + IntoEngineCall,
{
    // 1. SSZ decode.
    let envelope = match decode_ssz::<T>(&body) {
        Ok(e) => e,
        Err(p) => return p.into_response(),
    };

    // 2. Convert SSZ envelope → Block + dispatch tag + CL-claimed block_hash.
    let DecodedNewPayload {
        block,
        expected_block_hash,
        call,
        block_access_list,
    } = match envelope.into_engine_call() {
        Ok(d) => d,
        Err(problem) => return problem.into_response(),
    };

    // 3. V5/Amsterdam: structural BAL checks (mirror JSON-RPC NewPayloadV5Request).
    //    These are spec-level invalid-params, not block-validity failures, so they
    //    return 400 BadRequest instead of falling through to PayloadStatus::INVALID.
    if let EngineCall::V5 { raw_bal_hash, .. } = &call {
        // (a) Empty BAL → structural error. EIP-7928 makes BAL mandatory in V5.
        if raw_bal_hash.is_none() {
            return ProblemJson::bad_request("block_access_list required for engine_newPayloadV5")
                .into_response();
        }
        // (b) Fork-boundary detector: if rebuilding the header WITHOUT
        // `block_access_list_hash` produces the CL-claimed `block_hash`, the
        // payload is V4-shape misrouted to V5. Match JSON-RPC's -32602 path.
        if block.hash() != expected_block_hash {
            let mut alt_header = block.header.clone();
            alt_header.block_access_list_hash = None;
            let alt_hash = alt_header.compute_block_hash(&ethrex_crypto::NativeCrypto);
            if alt_hash == expected_block_hash {
                return ProblemJson::bad_request(
                    "engine_newPayloadV5 received header missing block_access_list_hash field",
                )
                .into_response();
            }
        }
    }

    // 3b. Fork-boundary checks (mirror JSON-RPC NewPayloadV4/V5 UnsupportedFork,
    //     engine/payload.rs). The fork is pinned by the URL, so a payload whose
    //     timestamp belongs to a different fork era is misrouted; reject it with
    //     400 instead of letting it fall through to a block-hash-mismatch INVALID.
    let chain_config = ctx.storage.get_chain_config();
    let ts = block.header.timestamp;
    match &call {
        // V4 (Prague/Osaka): Amsterdam-era payloads must use V5; pre-Prague must not use V4.
        EngineCall::V4 { .. } => {
            if chain_config.is_amsterdam_activated(ts) || !chain_config.is_prague_activated(ts) {
                return ProblemJson::bad_request(&format!(
                    "unsupported fork for this endpoint: {:?}",
                    chain_config.get_fork(ts)
                ))
                .into_response();
            }
        }
        // V5 (Amsterdam): pre-Amsterdam payloads must use V4.
        EngineCall::V5 { .. } => {
            if !chain_config.is_amsterdam_activated(ts) {
                return ProblemJson::bad_request(&format!(
                    "unsupported fork for this endpoint: {:?}",
                    chain_config.get_fork(ts)
                ))
                .into_response();
            }
        }
        EngineCall::V1V2 | EngineCall::V3 { .. } => {}
    }

    // 4. Dispatch to the appropriate handle_new_payload_* helper. The helpers
    //    only need the expected block_hash from the payload, so we pass it
    //    directly and skip the `JsonExecutionPayload::from_block` intermediate.
    let result = match call {
        EngineCall::V1V2 => {
            handle_new_payload_v1_v2(expected_block_hash, block, ctx, None, false).await
        }
        EngineCall::V3 { .. } => {
            handle_new_payload_v3(expected_block_hash, ctx, block, None, None, false).await
        }
        // Prague (V4) reuses handle_new_payload_v3 — matches the JSON-RPC
        // NewPayloadV4Request::handle behavior in engine/payload.rs.
        EngineCall::V4 { .. } => {
            handle_new_payload_v3(expected_block_hash, ctx, block, None, None, false).await
        }
        EngineCall::V5 { .. } => {
            // Pass the decoded BAL so handle_new_payload_v4 runs validate_ordering,
            // matching the JSON-RPC engine_newPayloadV5 path.
            handle_new_payload_v4(
                expected_block_hash,
                ctx,
                block,
                None,
                block_access_list,
                false,
            )
            .await
        }
    };

    // 5. Map internal PayloadStatus → SSZ PayloadStatus.
    let internal_status = match result {
        Ok(s) => s,
        Err(err) => {
            return ProblemJson::internal(&format!("engine error: {err}")).into_response();
        }
    };

    let status_code: u8 = match internal_status.status {
        PayloadValidationStatus::Valid => PayloadStatusCode::Valid as u8,
        PayloadValidationStatus::Invalid => PayloadStatusCode::Invalid as u8,
        PayloadValidationStatus::Syncing => PayloadStatusCode::Syncing as u8,
        PayloadValidationStatus::Accepted => PayloadStatusCode::Accepted as u8,
    };
    let ssz_status = SszPayloadStatus::new(
        status_code,
        internal_status.latest_valid_hash.map(|h| h.0),
        internal_status.validation_error,
    );

    SszBody(ssz_status).into_response()
}

// ── get_payload ───────────────────────────────────────────────────────────────

pub async fn get_payload(
    Path((fork_str, id_str)): Path<(String, String)>,
    State(ctx): State<RpcApiContext>,
) -> Response {
    // 1. Validate fork segment.
    let fork = match parse_fork_segment(&fork_str) {
        Ok(f) => f,
        Err(problem) => return problem.into_response(),
    };

    // 2. Parse payload id (0x-prefixed hex, 8 bytes).
    let id = match PayloadId::from_str(&id_str) {
        Ok(id) => id,
        Err(msg) => {
            return ProblemJson::bad_request(&format!("invalid payloadId: {msg}")).into_response();
        }
    };

    // 3. Retrieve the built payload from the blockchain.
    let result = match ctx.blockchain.get_payload(id.as_u64()).await {
        Ok(r) => r,
        Err(ChainError::UnknownPayload) => {
            return ProblemJson::not_found("unknown payloadId").into_response();
        }
        Err(err) => {
            return ProblemJson::internal(&format!("get_payload failed: {err}")).into_response();
        }
    };

    // 4. Convert Block → fork-specific SSZ `BuiltPayload` (replaces
    //    engine_getPayloadV1..V6) and return. `block_value`, `blobs_bundle`,
    //    `requests`, and the stored BAL all come from `PayloadBuildResult`.
    //    `should_override_builder` is always false (ethrex has no builder).
    //    The per-fork ExecutionPayload is reused from the envelope builders.
    let block = &result.payload;
    let block_value = u256_to_le_bytes(result.block_value);
    let built: Result<Response, ProblemJson> = match fork {
        Fork::Paris => paris_envelope_from_block(block).map(|env| {
            SszBody(BuiltPayloadParis {
                payload: env.execution_payload,
                block_value,
            })
            .into_response()
        }),
        Fork::Shanghai => shanghai_envelope_from_block(block).map(|env| {
            SszBody(BuiltPayloadShanghai {
                payload: env.execution_payload,
                block_value,
            })
            .into_response()
        }),
        Fork::Cancun => (|| -> Result<Response, ProblemJson> {
            let payload = cancun_envelope_from_block(block)?.execution_payload;
            Ok(SszBody(BuiltPayloadCancun {
                payload,
                block_value,
                blobs_bundle: blobs_bundle_v1(&result.blobs_bundle)?,
                should_override_builder: false,
            })
            .into_response())
        })(),
        Fork::Prague => (|| -> Result<Response, ProblemJson> {
            Ok(SszBody(BuiltPayloadPrague {
                payload: prague_payload_from_block(block)?,
                block_value,
                blobs_bundle: blobs_bundle_v1(&result.blobs_bundle)?,
                execution_requests: ssz_execution_requests(&result.requests)?,
                should_override_builder: false,
            })
            .into_response())
        })(),
        Fork::Osaka => (|| -> Result<Response, ProblemJson> {
            // Osaka payload is structurally identical to Prague; the difference
            // is the cell-proof BlobsBundleV2.
            Ok(SszBody(BuiltPayloadOsaka {
                payload: prague_payload_from_block(block)?,
                block_value,
                blobs_bundle: blobs_bundle_v2(&result.blobs_bundle)?,
                execution_requests: ssz_execution_requests(&result.requests)?,
                should_override_builder: false,
            })
            .into_response())
        })(),
        Fork::Amsterdam => (|| -> Result<Response, ProblemJson> {
            let payload = amsterdam_envelope_from_block(
                block,
                &result.requests,
                result.block_access_list.as_ref(),
            )?
            .execution_payload;
            Ok(SszBody(BuiltPayloadAmsterdam {
                payload,
                block_value,
                blobs_bundle: blobs_bundle_v2(&result.blobs_bundle)?,
                execution_requests: ssz_execution_requests(&result.requests)?,
                should_override_builder: false,
            })
            .into_response())
        })(),
        // Unreachable: parse_fork_segment already validated the fork.
        _ => unreachable!("fork path validation ensures only spec forks reach here"),
    };
    match built {
        Ok(r) => r,
        Err(p) => p.into_response(),
    }
}

// ── BlobsBundle + block_value conversions ─────────────────────────────────────

/// SSZ `uint256` is 32-byte little-endian.
fn u256_to_le_bytes(v: U256) -> [u8; 32] {
    v.to_little_endian()
}

/// Map the internal blobs (`Vec<[u8; BYTES_PER_BLOB]>`) into the SSZ blob list
/// shared by both bundle revisions.
fn ssz_blobs(
    blobs: &[[u8; BYTES_PER_BLOB]],
) -> Result<SszList<SszVector<u8, BYTES_PER_BLOB>, MAX_BLOB_COMMITMENTS_PER_BLOCK>, ProblemJson> {
    let v: Result<Vec<SszVector<u8, BYTES_PER_BLOB>>, ProblemJson> = blobs
        .iter()
        .map(|blob| {
            blob.to_vec()
                .try_into()
                .map_err(|_| ProblemJson::internal("blob is not BYTES_PER_BLOB"))
        })
        .collect();
    v?.try_into()
        .map_err(|_| ProblemJson::internal("blobs exceed MAX_BLOB_COMMITMENTS_PER_BLOCK"))
}

/// `BlobsBundleV1` — one whole-blob proof per blob (Cancun/Prague).
fn blobs_bundle_v1(b: &BlobsBundle) -> Result<BlobsBundleV1, ProblemJson> {
    Ok(BlobsBundleV1 {
        commitments: b.commitments.clone().try_into().map_err(|_| {
            ProblemJson::internal("commitments exceed MAX_BLOB_COMMITMENTS_PER_BLOCK")
        })?,
        proofs: b
            .proofs
            .clone()
            .try_into()
            .map_err(|_| ProblemJson::internal("proofs exceed MAX_BLOB_COMMITMENTS_PER_BLOCK"))?,
        blobs: ssz_blobs(&b.blobs)?,
    })
}

/// `BlobsBundleV2` — cell proofs (Osaka/Amsterdam).
fn blobs_bundle_v2(b: &BlobsBundle) -> Result<BlobsBundleV2, ProblemJson> {
    Ok(BlobsBundleV2 {
        commitments: b.commitments.clone().try_into().map_err(|_| {
            ProblemJson::internal("commitments exceed MAX_BLOB_COMMITMENTS_PER_BLOCK")
        })?,
        proofs: b
            .proofs
            .clone()
            .try_into()
            .map_err(|_| ProblemJson::internal("cell proofs exceed MAX_CELL_PROOFS"))?,
        blobs: ssz_blobs(&b.blobs)?,
    })
}

// ── Block → SSZ envelope conversions ─────────────────────────────────────────

/// Convert a u64 base_fee_per_gas to the 32-byte little-endian SSZ representation.
fn u64_to_ssz_base_fee(v: u64) -> [u8; 32] {
    let mut out = [0u8; 32];
    out[..8].copy_from_slice(&v.to_le_bytes());
    out
}

/// Build an `SszList<SszList<u8, MAX_BYTES>, MAX_TXS>` from a slice of raw transaction bytes.
/// Expands to a `Result<SszList<...>, ProblemJson>`; call with `?` at the use site.
macro_rules! ssz_txs {
    ($txs:expr, $max_bytes:expr, $max_count:expr) => {{
        use libssz_types::SszList;
        (|| -> Result<SszList<SszList<u8, $max_bytes>, $max_count>, ProblemJson> {
            let inner: Result<Vec<SszList<u8, $max_bytes>>, _> = $txs
                .iter()
                .map(|tx| {
                    let raw: Vec<u8> = tx.encode_canonical_to_vec();
                    raw.try_into().map_err(|_| {
                        ProblemJson::internal("transaction exceeds MAX_BYTES_PER_TRANSACTION")
                    })
                })
                .collect();
            inner?.try_into().map_err(|_| {
                ProblemJson::internal("transaction count exceeds MAX_TRANSACTIONS_PER_PAYLOAD")
            })
        })()
    }};
}

/// Build the `SszList<Withdrawal, MAX_WITHDRAWALS>` from the block body.
/// Returns `Err(ProblemJson)` if the withdrawal count exceeds `MAX_WITHDRAWALS_PER_PAYLOAD`.
fn ssz_withdrawals(
    block: &Block,
) -> Result<
    SszList<
        shanghai::Withdrawal,
        { crate::engine_rest::types::common::MAX_WITHDRAWALS_PER_PAYLOAD },
    >,
    ProblemJson,
> {
    let ws: Vec<shanghai::Withdrawal> = block
        .body
        .withdrawals
        .as_deref()
        .unwrap_or(&[])
        .iter()
        .map(|w| shanghai::Withdrawal {
            index: w.index,
            validator_index: w.validator_index,
            address: Bytes20(w.address.0),
            amount: w.amount,
        })
        .collect();
    ws.try_into()
        .map_err(|_| ProblemJson::internal("withdrawal count exceeds MAX_WITHDRAWALS_PER_PAYLOAD"))
}

fn paris_envelope_from_block(
    block: &Block,
) -> Result<paris::ExecutionPayloadEnvelope, ProblemJson> {
    use crate::engine_rest::types::common::{
        MAX_BYTES_PER_TRANSACTION, MAX_TRANSACTIONS_PER_PAYLOAD,
    };

    let h = &block.header;
    let txs: SszList<SszList<u8, MAX_BYTES_PER_TRANSACTION>, MAX_TRANSACTIONS_PER_PAYLOAD> = ssz_txs!(
        &block.body.transactions,
        MAX_BYTES_PER_TRANSACTION,
        MAX_TRANSACTIONS_PER_PAYLOAD
    )?;

    Ok(paris::ExecutionPayloadEnvelope {
        execution_payload: paris::ExecutionPayload {
            parent_hash: h.parent_hash.0,
            fee_recipient: Bytes20(h.coinbase.0),
            state_root: h.state_root.0,
            receipts_root: h.receipts_root.0,
            logs_bloom: h
                .logs_bloom
                .0
                .to_vec()
                .try_into()
                .expect("logs_bloom is exactly 256 bytes"),
            prev_randao: h.prev_randao.0,
            block_number: h.number,
            gas_limit: h.gas_limit,
            gas_used: h.gas_used,
            timestamp: h.timestamp,
            extra_data: h.extra_data.to_vec().try_into().map_err(|_| {
                ProblemJson::internal("stored extra_data exceeds MAX_EXTRA_DATA_BYTES")
            })?,
            base_fee_per_gas: u64_to_ssz_base_fee(h.base_fee_per_gas.unwrap_or(0)),
            block_hash: block.hash().0,
            transactions: txs,
        },
    })
}

fn shanghai_envelope_from_block(
    block: &Block,
) -> Result<shanghai::ExecutionPayloadEnvelope, ProblemJson> {
    use crate::engine_rest::types::common::{
        MAX_BYTES_PER_TRANSACTION, MAX_TRANSACTIONS_PER_PAYLOAD,
    };

    let h = &block.header;
    let txs: SszList<SszList<u8, MAX_BYTES_PER_TRANSACTION>, MAX_TRANSACTIONS_PER_PAYLOAD> = ssz_txs!(
        &block.body.transactions,
        MAX_BYTES_PER_TRANSACTION,
        MAX_TRANSACTIONS_PER_PAYLOAD
    )?;

    Ok(shanghai::ExecutionPayloadEnvelope {
        execution_payload: shanghai::ExecutionPayload {
            parent_hash: h.parent_hash.0,
            fee_recipient: Bytes20(h.coinbase.0),
            state_root: h.state_root.0,
            receipts_root: h.receipts_root.0,
            logs_bloom: h
                .logs_bloom
                .0
                .to_vec()
                .try_into()
                .expect("logs_bloom is exactly 256 bytes"),
            prev_randao: h.prev_randao.0,
            block_number: h.number,
            gas_limit: h.gas_limit,
            gas_used: h.gas_used,
            timestamp: h.timestamp,
            extra_data: h.extra_data.to_vec().try_into().map_err(|_| {
                ProblemJson::internal("stored extra_data exceeds MAX_EXTRA_DATA_BYTES")
            })?,
            base_fee_per_gas: u64_to_ssz_base_fee(h.base_fee_per_gas.unwrap_or(0)),
            block_hash: block.hash().0,
            transactions: txs,
            withdrawals: ssz_withdrawals(block)?,
        },
    })
}

fn cancun_envelope_from_block(
    block: &Block,
) -> Result<cancun::ExecutionPayloadEnvelope, ProblemJson> {
    use crate::engine_rest::types::common::{
        MAX_BYTES_PER_TRANSACTION, MAX_TRANSACTIONS_PER_PAYLOAD,
    };

    let h = &block.header;
    let txs: SszList<SszList<u8, MAX_BYTES_PER_TRANSACTION>, MAX_TRANSACTIONS_PER_PAYLOAD> = ssz_txs!(
        &block.body.transactions,
        MAX_BYTES_PER_TRANSACTION,
        MAX_TRANSACTIONS_PER_PAYLOAD
    )?;

    Ok(cancun::ExecutionPayloadEnvelope {
        execution_payload: cancun::ExecutionPayload {
            parent_hash: h.parent_hash.0,
            fee_recipient: Bytes20(h.coinbase.0),
            state_root: h.state_root.0,
            receipts_root: h.receipts_root.0,
            logs_bloom: h
                .logs_bloom
                .0
                .to_vec()
                .try_into()
                .expect("logs_bloom is exactly 256 bytes"),
            prev_randao: h.prev_randao.0,
            block_number: h.number,
            gas_limit: h.gas_limit,
            gas_used: h.gas_used,
            timestamp: h.timestamp,
            extra_data: h.extra_data.to_vec().try_into().map_err(|_| {
                ProblemJson::internal("stored extra_data exceeds MAX_EXTRA_DATA_BYTES")
            })?,
            base_fee_per_gas: u64_to_ssz_base_fee(h.base_fee_per_gas.unwrap_or(0)),
            block_hash: block.hash().0,
            transactions: txs,
            withdrawals: ssz_withdrawals(block)?,
            blob_gas_used: h.blob_gas_used.unwrap_or(0),
            excess_blob_gas: h.excess_blob_gas.unwrap_or(0),
        },
        parent_beacon_block_root: h.parent_beacon_block_root.unwrap_or_default().0,
    })
}

fn prague_payload_from_block(block: &Block) -> Result<prague::ExecutionPayload, ProblemJson> {
    use crate::engine_rest::types::common::{
        MAX_BYTES_PER_TRANSACTION, MAX_TRANSACTIONS_PER_PAYLOAD,
    };

    let h = &block.header;
    let txs: SszList<SszList<u8, MAX_BYTES_PER_TRANSACTION>, MAX_TRANSACTIONS_PER_PAYLOAD> = ssz_txs!(
        &block.body.transactions,
        MAX_BYTES_PER_TRANSACTION,
        MAX_TRANSACTIONS_PER_PAYLOAD
    )?;

    Ok(prague::ExecutionPayload {
        parent_hash: h.parent_hash.0,
        fee_recipient: Bytes20(h.coinbase.0),
        state_root: h.state_root.0,
        receipts_root: h.receipts_root.0,
        logs_bloom: h
            .logs_bloom
            .0
            .to_vec()
            .try_into()
            .expect("logs_bloom is exactly 256 bytes"),
        prev_randao: h.prev_randao.0,
        block_number: h.number,
        gas_limit: h.gas_limit,
        gas_used: h.gas_used,
        timestamp: h.timestamp,
        extra_data: h
            .extra_data
            .to_vec()
            .try_into()
            .map_err(|_| ProblemJson::internal("stored extra_data exceeds MAX_EXTRA_DATA_BYTES"))?,
        base_fee_per_gas: u64_to_ssz_base_fee(h.base_fee_per_gas.unwrap_or(0)),
        block_hash: block.hash().0,
        transactions: txs,
        withdrawals: ssz_withdrawals(block)?,
        blob_gas_used: h.blob_gas_used.unwrap_or(0),
        excess_blob_gas: h.excess_blob_gas.unwrap_or(0),
    })
}

/// Build the `execution_requests` SSZ list from the built payload's encoded
/// requests, filtering out empty entries per EIP-7685 (matches the JSON-RPC
/// `GetPayloadV4/V5/V6` handlers).
fn ssz_execution_requests(
    requests: &[ethrex_common::types::requests::EncodedRequests],
) -> Result<
    SszList<
        SszList<u8, { crate::engine_rest::types::common::MAX_REQUEST_BYTES }>,
        { crate::engine_rest::types::common::MAX_EXECUTION_REQUESTS_PER_PAYLOAD },
    >,
    ProblemJson,
> {
    use crate::engine_rest::types::common::MAX_REQUEST_BYTES;
    let inner: Result<Vec<SszList<u8, MAX_REQUEST_BYTES>>, _> = requests
        .iter()
        .filter(|r| !r.0.is_empty())
        .map(|r| {
            r.0.to_vec().try_into().map_err(|_| {
                ProblemJson::internal("execution_request entry exceeds MAX_REQUEST_BYTES")
            })
        })
        .collect();
    inner?
        .try_into()
        .map_err(|_| ProblemJson::internal("execution_requests list overflow"))
}

fn amsterdam_envelope_from_block(
    block: &Block,
    requests: &[ethrex_common::types::requests::EncodedRequests],
    bal: Option<&ethrex_common::types::block_access_list::BlockAccessList>,
) -> Result<amsterdam::ExecutionPayloadEnvelope, ProblemJson> {
    use crate::engine_rest::types::common::MAX_BLOCK_ACCESS_LIST_BYTES;

    let h = &block.header;
    // EIP-7928: serialize the BAL as its canonical RLP encoding. The CL recomputes
    // `block_access_list_hash = keccak(rlp_bytes)` and matches it against the
    // hash baked into the header. An empty/missing BAL would invalidate the
    // built payload at the CL.
    let bal_bytes: Vec<u8> = match bal {
        Some(b) => b.encode_to_vec(),
        None => Vec::new(),
    };
    let block_access_list: SszList<u8, MAX_BLOCK_ACCESS_LIST_BYTES> =
        bal_bytes.try_into().map_err(|_| {
            ProblemJson::internal("block_access_list exceeds MAX_BLOCK_ACCESS_LIST_BYTES")
        })?;

    use crate::engine_rest::types::common::{
        MAX_BYTES_PER_TRANSACTION, MAX_TRANSACTIONS_PER_PAYLOAD,
    };

    let txs: SszList<SszList<u8, MAX_BYTES_PER_TRANSACTION>, MAX_TRANSACTIONS_PER_PAYLOAD> = ssz_txs!(
        &block.body.transactions,
        MAX_BYTES_PER_TRANSACTION,
        MAX_TRANSACTIONS_PER_PAYLOAD
    )?;

    Ok(amsterdam::ExecutionPayloadEnvelope {
        execution_payload: amsterdam::ExecutionPayload {
            parent_hash: h.parent_hash.0,
            fee_recipient: Bytes20(h.coinbase.0),
            state_root: h.state_root.0,
            receipts_root: h.receipts_root.0,
            logs_bloom: h
                .logs_bloom
                .0
                .to_vec()
                .try_into()
                .expect("logs_bloom is exactly 256 bytes"),
            prev_randao: h.prev_randao.0,
            block_number: h.number,
            gas_limit: h.gas_limit,
            gas_used: h.gas_used,
            timestamp: h.timestamp,
            extra_data: h.extra_data.to_vec().try_into().map_err(|_| {
                ProblemJson::internal("stored extra_data exceeds MAX_EXTRA_DATA_BYTES")
            })?,
            base_fee_per_gas: u64_to_ssz_base_fee(h.base_fee_per_gas.unwrap_or(0)),
            block_hash: block.hash().0,
            transactions: txs,
            withdrawals: ssz_withdrawals(block)?,
            blob_gas_used: h.blob_gas_used.unwrap_or(0),
            excess_blob_gas: h.excess_blob_gas.unwrap_or(0),
            block_access_list,
            slot_number: h.slot_number.unwrap_or(0),
        },
        parent_beacon_block_root: h.parent_beacon_block_root.unwrap_or_default().0,
        execution_requests: ssz_execution_requests(requests)?,
    })
}
