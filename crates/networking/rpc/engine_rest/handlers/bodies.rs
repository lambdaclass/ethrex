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
    BodiesByHashRequest, BodiesByHashResponseAmsterdam, BodiesByHashResponseParis,
    BodiesByHashResponseShanghai, BodyAmsterdam, BodyParis, BodyShanghai, OptBodyAmsterdam,
    OptBodyParis, OptBodyShanghai,
};
use crate::engine_rest::types::common::Bytes20;
use crate::engine_rest::types::shanghai::Withdrawal as SszWithdrawal;
use crate::rpc::RpcApiContext;

pub async fn bodies_by_hash(
    ForkPath(fork): ForkPath,
    State(ctx): State<RpcApiContext>,
    Ssz(req): Ssz<BodiesByHashRequest>,
) -> Response {
    if req.hashes.len() > BODIES_MAX_COUNT as usize {
        return ProblemJson::payload_too_large(&format!(
            "request exceeds BODIES_MAX_COUNT ({BODIES_MAX_COUNT})"
        ))
        .into_response();
    }

    let hashes: Vec<H256> = req.hashes.iter().map(|h| H256::from(*h)).collect();

    match fork {
        Fork::Paris => fetch_paris_bodies(hashes, ctx).await,
        Fork::Shanghai | Fork::Cancun | Fork::Prague | Fork::Osaka => {
            fetch_shanghai_bodies(hashes, ctx).await
        }
        Fork::Amsterdam => fetch_amsterdam_bodies(hashes, ctx).await,
        _ => unreachable!("ForkPath restricts to spec forks"),
    }
}

async fn fetch_paris_bodies(hashes: Vec<H256>, ctx: RpcApiContext) -> Response {
    let mut bodies: Vec<OptBodyParis> = Vec::with_capacity(hashes.len());
    for h in hashes {
        let body_opt = match ctx.storage.get_block_body_by_hash(h).await {
            Ok(b) => b,
            Err(e) => {
                return ProblemJson::internal(&format!("storage: {e}")).into_response();
            }
        };
        match body_opt {
            Some(b) => match paris_body_from_internal(b) {
                Ok(p) => bodies.push(OptBodyParis(Some(p))),
                Err(p) => return p.into_response(),
            },
            None => bodies.push(OptBodyParis(None)),
        }
    }
    SszBody(BodiesByHashResponseParis { bodies }).into_response()
}

async fn fetch_shanghai_bodies(hashes: Vec<H256>, ctx: RpcApiContext) -> Response {
    let mut bodies: Vec<OptBodyShanghai> = Vec::with_capacity(hashes.len());
    for h in hashes {
        let body_opt = match ctx.storage.get_block_body_by_hash(h).await {
            Ok(b) => b,
            Err(e) => {
                return ProblemJson::internal(&format!("storage: {e}")).into_response();
            }
        };
        match body_opt {
            Some(b) => match shanghai_body_from_internal(b) {
                Ok(p) => bodies.push(OptBodyShanghai(Some(p))),
                Err(p) => return p.into_response(),
            },
            None => bodies.push(OptBodyShanghai(None)),
        }
    }
    SszBody(BodiesByHashResponseShanghai { bodies }).into_response()
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

async fn fetch_amsterdam_bodies(hashes: Vec<H256>, ctx: RpcApiContext) -> Response {
    let mut bodies: Vec<OptBodyAmsterdam> = Vec::with_capacity(hashes.len());
    for h in hashes {
        let block_opt = match ctx.storage.get_block_by_hash(h).await {
            Ok(b) => b,
            Err(e) => {
                return ProblemJson::internal(&format!("storage: {e}")).into_response();
            }
        };
        match block_opt {
            Some(block) => {
                let (bal_bytes, block) =
                    match bal_bytes_for_block(ctx.blockchain.clone(), block).await {
                        Ok(v) => v,
                        Err(resp) => return resp,
                    };
                match amsterdam_body_from_internal(block.body, bal_bytes) {
                    Ok(p) => bodies.push(OptBodyAmsterdam(Some(p))),
                    Err(p) => return p.into_response(),
                }
            }
            None => bodies.push(OptBodyAmsterdam(None)),
        }
    }
    SszBody(BodiesByHashResponseAmsterdam { bodies }).into_response()
}

// ── internal → SSZ conversions ────────────────────────────────────────────────

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

// ── bodies_by_range ───────────────────────────────────────────────────────────

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
    // Spec: MUST NOT return trailing nulls past the chain head.
    let last = latest.min(from + count - 1);

    if last < from {
        // No blocks in range — return empty per-fork response.
        return match fork {
            Fork::Paris => SszBody(BodiesByHashResponseParis {
                bodies: Vec::<OptBodyParis>::new(),
            })
            .into_response(),
            Fork::Shanghai | Fork::Cancun | Fork::Prague | Fork::Osaka => {
                SszBody(BodiesByHashResponseShanghai {
                    bodies: Vec::<OptBodyShanghai>::new(),
                })
                .into_response()
            }
            Fork::Amsterdam => SszBody(BodiesByHashResponseAmsterdam {
                bodies: Vec::<OptBodyAmsterdam>::new(),
            })
            .into_response(),
            _ => unreachable!("ForkPath restricts to spec forks"),
        };
    }

    match fork {
        Fork::Paris => {
            let bodies = match ctx.storage.get_block_bodies(from, last).await {
                Ok(v) => v,
                Err(e) => return ProblemJson::internal(&format!("storage: {e}")).into_response(),
            };
            let mut converted: Vec<OptBodyParis> = Vec::with_capacity(bodies.len());
            for b in bodies {
                converted.push(match b {
                    Some(body) => match paris_body_from_internal(body) {
                        Ok(p) => OptBodyParis(Some(p)),
                        Err(p) => return p.into_response(),
                    },
                    None => OptBodyParis(None),
                });
            }
            SszBody(BodiesByHashResponseParis { bodies: converted }).into_response()
        }
        Fork::Shanghai | Fork::Cancun | Fork::Prague | Fork::Osaka => {
            let bodies = match ctx.storage.get_block_bodies(from, last).await {
                Ok(v) => v,
                Err(e) => return ProblemJson::internal(&format!("storage: {e}")).into_response(),
            };
            let mut converted: Vec<OptBodyShanghai> = Vec::with_capacity(bodies.len());
            for b in bodies {
                converted.push(match b {
                    Some(body) => match shanghai_body_from_internal(body) {
                        Ok(p) => OptBodyShanghai(Some(p)),
                        Err(p) => return p.into_response(),
                    },
                    None => OptBodyShanghai(None),
                });
            }
            SszBody(BodiesByHashResponseShanghai { bodies: converted }).into_response()
        }
        Fork::Amsterdam => {
            let mut converted: Vec<OptBodyAmsterdam> =
                Vec::with_capacity((last - from + 1) as usize);
            for n in from..=last {
                let block_opt = match ctx.storage.get_block_by_number(n).await {
                    Ok(b) => b,
                    Err(e) => {
                        return ProblemJson::internal(&format!("storage: {e}")).into_response();
                    }
                };
                match block_opt {
                    Some(block) => {
                        let (bal_bytes, block) =
                            match bal_bytes_for_block(ctx.blockchain.clone(), block).await {
                                Ok(v) => v,
                                Err(resp) => return resp,
                            };
                        match amsterdam_body_from_internal(block.body, bal_bytes) {
                            Ok(p) => converted.push(OptBodyAmsterdam(Some(p))),
                            Err(p) => return p.into_response(),
                        }
                    }
                    None => converted.push(OptBodyAmsterdam(None)),
                }
            }
            SszBody(BodiesByHashResponseAmsterdam { bodies: converted }).into_response()
        }
        _ => unreachable!("ForkPath restricts to spec forks"),
    }
}
