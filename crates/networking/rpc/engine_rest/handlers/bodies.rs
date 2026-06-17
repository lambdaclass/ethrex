//! /{fork}/bodies/* — body retrieval by hash and by range.

use std::sync::Arc;

use axum::extract::{Query, State};
use axum::response::{IntoResponse, Response};
use ethrex_blockchain::Blockchain;
use ethrex_common::H256;
use ethrex_common::types::{Block, BlockBody, Fork};
use ethrex_rlp::encode::RLPEncode;
use serde::Deserialize;

use crate::engine_rest::error::ProblemJson;
use crate::engine_rest::extractors::Ssz;
use crate::engine_rest::fork_path::ForkPath;
use crate::engine_rest::handlers::capabilities::BODIES_MAX_COUNT;
use crate::engine_rest::responses::SszBody;
use crate::engine_rest::types::bodies::{
    BlockHashList, BodiesResponseAmsterdam, BodiesResponseParis, BodiesResponseShanghai,
    BodyAmsterdam, BodyEntryAmsterdam, BodyEntryParis, BodyEntryShanghai, BodyParis, BodyShanghai,
};
use crate::engine_rest::types::common::Bytes20;
use crate::engine_rest::types::shanghai::Withdrawal as SszWithdrawal;
use crate::rpc::RpcApiContext;

// ── Handlers ──────────────────────────────────────────────────────────────────

pub async fn bodies_by_hash(
    ForkPath(fork): ForkPath,
    State(ctx): State<RpcApiContext>,
    Ssz(hashes): Ssz<BlockHashList>,
) -> Response {
    if hashes.len() > BODIES_MAX_COUNT as usize {
        return ProblemJson::payload_too_large(&format!(
            "request exceeds BODIES_MAX_COUNT ({BODIES_MAX_COUNT})"
        ))
        .into_response();
    }

    let mut blocks: Vec<Option<Block>> = Vec::with_capacity(hashes.len());
    for h in hashes.iter() {
        match ctx.storage.get_block_by_hash(H256::from(*h)).await {
            Ok(b) => blocks.push(b),
            Err(e) => return ProblemJson::internal(&format!("storage: {e}")).into_response(),
        }
    }

    build_bodies_response(fork, blocks, ctx).await
}

#[derive(Deserialize)]
pub struct BodiesRangeParams {
    pub from: Option<u64>,
    pub count: Option<u64>,
}

pub async fn bodies_by_range(
    ForkPath(fork): ForkPath,
    State(ctx): State<RpcApiContext>,
    Query(params): Query<BodiesRangeParams>,
) -> Response {
    let from = match params.from {
        Some(v) if v >= 1 => v,
        Some(_) => return ProblemJson::bad_request("from must be >= 1").into_response(),
        None => return ProblemJson::bad_request("missing from query parameter").into_response(),
    };
    let count = match params.count {
        Some(v) if v >= 1 => v,
        Some(_) => return ProblemJson::bad_request("count must be >= 1").into_response(),
        None => return ProblemJson::bad_request("missing count query parameter").into_response(),
    };
    if count > BODIES_MAX_COUNT as u64 {
        return ProblemJson::payload_too_large(&format!(
            "count exceeds BODIES_MAX_COUNT ({BODIES_MAX_COUNT})"
        ))
        .into_response();
    }

    let latest = match ctx.storage.get_latest_block_number().await {
        Ok(n) => n,
        Err(e) => return ProblemJson::internal(&format!("storage: {e}")).into_response(),
    };
    // Truncate the range at the chain head — do NOT pad past-head numbers with
    // `available=false` entries. This follows the legacy "no trailing nulls" rule
    // and matches Nethermind. Note execution-apis #793 is ambiguous here: its text
    // reads as pad-to-`count` (which Erigon implements). Revisit if the spec pins
    // pad-to-`count`. Saturating math guards against an absurd `from` (an unbounded
    // u64 query param) overflowing `from + count`.
    let last = latest.min(from.saturating_add(count).saturating_sub(1));

    // Fetch every body in one storage transaction (like the JSON-RPC
    // `getPayloadBodiesByRange` handler) instead of one async round-trip per
    // block; the headers — needed only for the per-entry fork-era check — are
    // cheap synchronous point reads.
    let mut blocks: Vec<Option<Block>> = Vec::new();
    if last >= from {
        let bodies = match ctx.storage.get_block_bodies(from, last).await {
            Ok(b) => b,
            Err(e) => return ProblemJson::internal(&format!("storage: {e}")).into_response(),
        };
        blocks.reserve(bodies.len());
        for (i, body) in bodies.into_iter().enumerate() {
            let block = match body {
                Some(body) => match ctx.storage.get_block_header(from + i as u64) {
                    Ok(Some(header)) => Some(Block::new(header, body)),
                    Ok(None) => None,
                    Err(e) => {
                        return ProblemJson::internal(&format!("storage: {e}")).into_response();
                    }
                },
                None => None,
            };
            blocks.push(block);
        }
    }

    build_bodies_response(fork, blocks, ctx).await
}

// ── Response builder ──────────────────────────────────────────────────────────

/// Build the per-fork bodies response from the resolved blocks. A block is
/// `available` only when it exists AND its timestamp falls inside the URL fork's
/// active range (spec #793); otherwise the entry is `available == false` with a
/// zero-valued body. The fork-shape grouping (Paris / Shanghai / Amsterdam) is
/// independent of the per-entry era check, which always tests the specific URL
/// fork.
async fn build_bodies_response(
    fork: Fork,
    blocks: Vec<Option<Block>>,
    ctx: RpcApiContext,
) -> Response {
    let chain_config = ctx.storage.get_chain_config();
    let in_era = |block: &Block| chain_config.get_fork(block.header.timestamp) == fork;

    match fork {
        Fork::Paris => {
            let mut entries: Vec<BodyEntryParis> = Vec::with_capacity(blocks.len());
            for block_opt in blocks {
                let entry = match block_opt {
                    Some(block) if in_era(&block) => match paris_body_from_internal(block.body) {
                        Ok(body) => BodyEntryParis::available(body),
                        Err(p) => return p.into_response(),
                    },
                    _ => BodyEntryParis::unavailable(),
                };
                entries.push(entry);
            }
            match TryInto::<BodiesResponseParis>::try_into(entries) {
                Ok(resp) => SszBody(resp).into_response(),
                Err(_) => bodies_overflow().into_response(),
            }
        }
        Fork::Shanghai | Fork::Cancun | Fork::Prague | Fork::Osaka => {
            let mut entries: Vec<BodyEntryShanghai> = Vec::with_capacity(blocks.len());
            for block_opt in blocks {
                let entry = match block_opt {
                    Some(block) if in_era(&block) => {
                        match shanghai_body_from_internal(block.body) {
                            Ok(body) => BodyEntryShanghai::available(body),
                            Err(p) => return p.into_response(),
                        }
                    }
                    _ => BodyEntryShanghai::unavailable(),
                };
                entries.push(entry);
            }
            match TryInto::<BodiesResponseShanghai>::try_into(entries) {
                Ok(resp) => SszBody(resp).into_response(),
                Err(_) => bodies_overflow().into_response(),
            }
        }
        Fork::Amsterdam => {
            let mut entries: Vec<BodyEntryAmsterdam> = Vec::with_capacity(blocks.len());
            for block_opt in blocks {
                let entry = match block_opt {
                    Some(block) if in_era(&block) => {
                        // Fast path: the BAL persisted at import time is a
                        // synchronous point read. Only the re-execution
                        // fallback (BAL absent) is CPU-bound enough to need
                        // the blocking-thread helper. On a normally-operating
                        // node BALs are persisted at import, so the fallback is
                        // the rare path (e.g. snap-synced pre-cutover blocks).
                        let (bal_bytes, block) =
                            match ctx.storage.get_block_access_list(block.hash()) {
                                Ok(Some(bal)) => (bal.encode_to_vec(), block),
                                Ok(None) => {
                                    match bal_bytes_for_block(ctx.blockchain.clone(), block).await {
                                        Ok(v) => v,
                                        Err(resp) => return resp,
                                    }
                                }
                                Err(e) => {
                                    return ProblemJson::internal(&format!("storage: {e}"))
                                        .into_response();
                                }
                            };
                        match amsterdam_body_from_internal(block.body, bal_bytes) {
                            Ok(body) => BodyEntryAmsterdam::available(body),
                            Err(p) => return p.into_response(),
                        }
                    }
                    _ => BodyEntryAmsterdam::unavailable(),
                };
                entries.push(entry);
            }
            match TryInto::<BodiesResponseAmsterdam>::try_into(entries) {
                Ok(resp) => SszBody(resp).into_response(),
                Err(_) => bodies_overflow().into_response(),
            }
        }
        // Unreachable: ForkPath restricts to the 6 spec forks before the handler runs.
        _ => unreachable!("ForkPath restricts to spec forks"),
    }
}

fn bodies_overflow() -> ProblemJson {
    ProblemJson::internal("bodies response exceeds MAX_BODIES_PER_REQUEST")
}

/// Get the RLP-encoded BAL bytes for a block, preferring the copy stored at
/// import time and falling back to re-execution only when it isn't persisted
/// (see `Blockchain::generate_bal_for_block`). The work is offloaded to a
/// blocking thread because the re-execution fallback is CPU-bound and would
/// otherwise stall the async runtime. Returns the (bytes, block) pair so the
/// caller can reuse the moved-in block. Empty bytes for a pre-Amsterdam block.
async fn bal_bytes_for_block(
    blockchain: Arc<Blockchain>,
    block: Block,
) -> Result<(Vec<u8>, Block), Response> {
    let (bal_result, block) = tokio::task::spawn_blocking(move || {
        let bal = blockchain.generate_bal_for_block(&block);
        (bal, block)
    })
    .await
    .map_err(|e| ProblemJson::internal(&format!("bal task join failed: {e}")).into_response())?;

    match bal_result {
        Ok(bal) => Ok((bal.map(|b| b.encode_to_vec()).unwrap_or_default(), block)),
        Err(e) => Err(ProblemJson::internal(&format!("bal: {e}")).into_response()),
    }
}

// ── internal → SSZ body conversions ───────────────────────────────────────────

fn paris_body_from_internal(body: BlockBody) -> Result<BodyParis, ProblemJson> {
    let sszed_txs = body
        .transactions
        .iter()
        .map(|tx| {
            tx.encode_canonical_to_vec()
                .try_into()
                .map_err(|_| ProblemJson::internal("transaction exceeds MAX_BYTES_PER_TRANSACTION"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let transactions = sszed_txs
        .try_into()
        .map_err(|_| ProblemJson::internal("transactions exceed MAX_TRANSACTIONS_PER_PAYLOAD"))?;
    Ok(BodyParis { transactions })
}

fn shanghai_body_from_internal(body: BlockBody) -> Result<BodyShanghai, ProblemJson> {
    let sszed_txs = body
        .transactions
        .iter()
        .map(|tx| {
            tx.encode_canonical_to_vec()
                .try_into()
                .map_err(|_| ProblemJson::internal("transaction exceeds MAX_BYTES_PER_TRANSACTION"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let transactions = sszed_txs
        .try_into()
        .map_err(|_| ProblemJson::internal("transactions exceed MAX_TRANSACTIONS_PER_PAYLOAD"))?;
    let withdrawals_vec: Vec<SszWithdrawal> = body
        .withdrawals
        .unwrap_or_default()
        .into_iter()
        .map(|w| SszWithdrawal {
            index: w.index,
            validator_index: w.validator_index,
            address: Bytes20(w.address.0),
            amount: w.amount,
        })
        .collect();
    let withdrawals = withdrawals_vec
        .try_into()
        .map_err(|_| ProblemJson::internal("withdrawals exceed MAX_WITHDRAWALS_PER_PAYLOAD"))?;
    Ok(BodyShanghai {
        transactions,
        withdrawals,
    })
}

fn amsterdam_body_from_internal(
    body: BlockBody,
    bal_bytes: Vec<u8>,
) -> Result<BodyAmsterdam, ProblemJson> {
    let shanghai = shanghai_body_from_internal(body)?;
    let block_access_list = bal_bytes
        .try_into()
        .map_err(|_| ProblemJson::internal("BAL bytes exceed MAX_BLOCK_ACCESS_LIST_BYTES"))?;
    Ok(BodyAmsterdam {
        transactions: shanghai.transactions,
        withdrawals: shanghai.withdrawals,
        block_access_list,
    })
}
