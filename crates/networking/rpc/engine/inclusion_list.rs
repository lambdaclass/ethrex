//! `engine_getInclusionListV1` plus the EIP-7805 (FOCIL) machinery shared by
//! `engine_newPayloadV6` and `engine_forkchoiceUpdatedV5`.
//!
//! Per EIP-7805 and execution-apis:
//!
//! - `engine_getInclusionListV1` takes no parameters and builds the list from
//!   the node's own view of the mempool against its canonical head.
//! - Returns: array of EIP-2718 RLP-encoded transactions, hex-encoded.
//! - Total RLP byte length ≤ `MAX_BYTES_PER_INCLUSION_LIST` (8192).
//! - Excludes blob (EIP-4844) transactions.
//! - 1-second timeout.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bytes::Bytes;
use ethrex_blockchain::inclusion_list_builder::InclusionListBuilder;
use ethrex_blockchain::inclusion_list_validator::StoreIlStateProvider;
use ethrex_common::H256;
use ethrex_common::types::{MAX_BYTES_PER_INCLUSION_LIST, Transaction};
use ethrex_crypto::NativeCrypto;
use serde_json::Value;
use tracing::debug;

use crate::{
    rpc::{RpcApiContext, RpcHandler},
    utils::RpcErr,
};

/// 1-second deadline mandated by the FOCIL execution-apis spec.
const GET_INCLUSION_LIST_V1_TIMEOUT: Duration = Duration::from_secs(1);

/// Upper bound on retained inclusion lists. `engine_newPayloadV6` retains the
/// list for every payload it accepts so `engine_forkchoiceUpdatedV5` can report
/// `inclusionListSatisfied` for the head it is told to adopt; the spec allows
/// discarding a list once its payload is no longer the tip of a branch, so a
/// small FIFO window is enough to cover the branches still in play.
const MAX_RETAINED_INCLUSION_LISTS: usize = 64;

/// Inclusion lists retained from `engine_newPayloadV6`, keyed by block hash.
///
/// execution-apis requires retaining `inclusionListTransactions` for a payload
/// with `ACCEPTED` status and permits discarding them once the payload is no
/// longer a branch tip. Lists are kept for accepted *and* valid payloads, since
/// `engine_forkchoiceUpdatedV5` must report the verdict for whichever of them
/// the consensus layer later names as head. Eviction is FIFO once the window is
/// full, which is why this is not a correctness-critical cache: a miss simply
/// leaves `inclusionListSatisfied` unreported.
#[derive(Debug, Default)]
pub struct RetainedInclusionLists {
    by_block: HashMap<H256, RetainedInclusionList>,
    order: VecDeque<H256>,
}

/// One retained list and, once the payload has been imported and judged, its
/// verdict. The verdict is kept rather than recomputed on later calls:
/// recomputation reads both of the block's states, and a state pruned between
/// two calls would turn a decided replay into an excused one, so the two calls
/// would disagree about the same payload.
#[derive(Debug, Clone)]
pub struct RetainedInclusionList {
    pub transactions: Vec<Transaction>,
    pub satisfied: Option<bool>,
}

impl RetainedInclusionLists {
    /// Retains `transactions` for `block_hash`. A list already retained for the
    /// block keeps its verdict when the transactions are the same, and loses it
    /// when they differ, since the verdict was about the earlier list.
    pub fn insert(&mut self, block_hash: H256, transactions: Vec<Transaction>) {
        match self.by_block.get_mut(&block_hash) {
            Some(existing) => {
                if existing.transactions != transactions {
                    existing.transactions = transactions;
                    existing.satisfied = None;
                }
            }
            None => {
                self.by_block.insert(
                    block_hash,
                    RetainedInclusionList {
                        transactions,
                        satisfied: None,
                    },
                );
                self.order.push_back(block_hash);
            }
        }
        while self.order.len() > MAX_RETAINED_INCLUSION_LISTS {
            if let Some(evicted) = self.order.pop_front() {
                self.by_block.remove(&evicted);
            }
        }
    }

    pub fn get(&self, block_hash: &H256) -> Option<&RetainedInclusionList> {
        self.by_block.get(block_hash)
    }

    /// Records the verdict computed for `block_hash`, if its list is retained.
    pub fn record_verdict(&mut self, block_hash: &H256, satisfied: bool) {
        if let Some(retained) = self.by_block.get_mut(block_hash) {
            retained.satisfied = Some(satisfied);
        }
    }
}

/// Shared handle to [`RetainedInclusionLists`]. The critical section only ever
/// clones a transaction list out or moves one in, so it never spans an `await`.
pub type RetainedInclusionListsHandle = Arc<Mutex<RetainedInclusionLists>>;

/// Runs the inclusion-list satisfaction check for `block_hash` against
/// `inclusion_list`, reporting whether the block satisfies it.
///
/// An empty inclusion list is trivially satisfied. The verdict itself is
/// `Blockchain::inclusion_list_satisfaction`: the EIP-7805 nonce-and-balance
/// check for ordinary transactions and the FOCIL frame-transaction extension
/// for EIP-8141 frame transactions, over the imported block's committed states.
/// Because both are fixed by the block, `engine_newPayloadV6` and a later
/// `engine_forkchoiceUpdatedV5` naming the same payload report the same field.
pub async fn block_satisfies_inclusion_list(
    context: &RpcApiContext,
    block_hash: H256,
    inclusion_list: &[Transaction],
) -> Result<bool, RpcErr> {
    if inclusion_list.is_empty() {
        return Ok(true);
    }

    let header = context
        .storage
        .get_block_header_by_hash(block_hash)
        .map_err(|e| RpcErr::Internal(e.to_string()))?
        .ok_or_else(|| RpcErr::Internal("block missing for IL satisfaction check".to_string()))?;
    let body = context
        .storage
        .get_block_body_by_hash(block_hash)
        .await
        .map_err(|e| RpcErr::Internal(e.to_string()))?
        .ok_or_else(|| {
            RpcErr::Internal("block body missing for IL satisfaction check".to_string())
        })?;
    let block_tx_hashes: HashSet<H256> = body
        .transactions
        .iter()
        .map(|tx| tx.hash(&NativeCrypto))
        .collect();

    let satisfaction = context
        .blockchain
        .inclusion_list_satisfaction(
            &header,
            &block_tx_hashes,
            body.withdrawals.as_deref().unwrap_or_default(),
            inclusion_list,
        )
        .map_err(|e| RpcErr::Internal(format!("IL satisfaction check failed: {e}")))?;
    Ok(satisfaction.is_satisfied())
}

#[derive(Debug)]
pub struct GetInclusionListV1Request {
    /// Parent block the list is built against, when the caller names one.
    ///
    /// The merged execution-apis spec (bogota.md) gives this method no
    /// parameters, and `None` reproduces that: the list is built against this
    /// node's canonical head, which is the parent of the block the list
    /// constrains.
    ///
    /// An earlier revision of execution-apis#609 passed the parent hash, and
    /// consensus clients built against it still send one — teku sends
    /// `params: ["0x…"]` and treats a rejection as "failed to produce
    /// inclusion_list", which takes FOCIL out of service for that validator.
    /// Accepting the argument costs nothing and is strictly better information
    /// than assuming the head: it is the parent the caller will actually build
    /// on, which differs from our head whenever the CL is a block ahead.
    pub parent_hash: Option<H256>,
}

impl RpcHandler for GetInclusionListV1Request {
    fn parse(params: &Option<Vec<Value>>) -> Result<Self, RpcErr> {
        match params.as_deref() {
            None | Some([]) => Ok(Self { parent_hash: None }),
            Some([parent]) => {
                let parent_hash = serde_json::from_value(parent.clone()).map_err(|_| {
                    RpcErr::WrongParam("parentHash: expected a 32-byte hash".to_owned())
                })?;
                Ok(Self {
                    parent_hash: Some(parent_hash),
                })
            }
            Some(_) => Err(RpcErr::BadParams(
                "Expected no params, or a single parent hash".to_owned(),
            )),
        }
    }

    async fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        debug!("engine_getInclusionListV1");

        // Build the IL inside `tokio::time::timeout`. The builder is sync but
        // wrapping it in spawn_blocking would be heavier than necessary; the
        // 1-second deadline is generous for the in-memory work the builder
        // performs.
        let parent_hash = self.parent_hash;
        let result = tokio::time::timeout(GET_INCLUSION_LIST_V1_TIMEOUT, async move {
            // The list constrains the next block, so it is built against that
            // block's parent: the hash the caller named, or this node's
            // canonical head when the caller named none. An unknown parent
            // falls back to the head rather than failing — a list built against
            // a slightly stale parent is still useful, and refusing would take
            // the caller's validator out of FOCIL service entirely.
            let named_header = match parent_hash {
                Some(hash) => context
                    .storage
                    .get_block_header_by_hash(hash)
                    .map_err(|e| RpcErr::Internal(e.to_string()))?,
                None => None,
            };
            let parent_header = match named_header {
                Some(header) => header,
                None => {
                    if let Some(hash) = parent_hash {
                        debug!(
                            "engine_getInclusionListV1: unknown parent {hash:#x}, building against canonical head"
                        );
                    }
                    let head_number = context
                        .storage
                        .get_latest_block_number()
                        .await
                        .map_err(|e| RpcErr::Internal(e.to_string()))?;
                    context
                        .storage
                        .get_block_header(head_number)
                        .map_err(|e| RpcErr::Internal(e.to_string()))?
                        .ok_or_else(|| {
                            RpcErr::Internal("canonical head header missing".to_string())
                        })?
                }
            };

            let state = StoreIlStateProvider {
                store: &context.storage,
                state_root: parent_header.state_root,
            };
            let base_fee = parent_header.base_fee_per_gas.unwrap_or(0);

            // Read CLI-driven config from context. Hard-cap at 8192 in
            // non-test builds per spec — operators cannot raise this above
            // the protocol limit.
            let cfg = &context.il_config;
            let max_bytes = if cfg!(test) {
                cfg.max_bytes
            } else {
                cfg.max_bytes.min(MAX_BYTES_PER_INCLUSION_LIST)
            };
            let builder = InclusionListBuilder::new(cfg.policy, cfg.per_sender_cap, max_bytes);
            let txs = builder.build(&context.blockchain.mempool, base_fee, &state);

            // Serialize each tx to its EIP-2718 canonical encoding (RLP for
            // legacy, type-prefixed for typed) wrapped as JSON hex strings.
            let encoded: Vec<Bytes> = txs
                .iter()
                .map(|tx| Bytes::from(tx.encode_canonical_to_vec()))
                .collect();

            // Defense in depth: re-check the spec byte cap (8192). Operator
            // overrides via `--il-max-bytes` are clamped above to the spec
            // limit; this catches any future refactor that bypasses that.
            let total: usize = encoded.iter().map(|b| b.len()).sum();
            if total > MAX_BYTES_PER_INCLUSION_LIST {
                return Err(RpcErr::Internal(format!(
                    "inclusion list builder produced {total} bytes, exceeding 8192-byte cap"
                )));
            }

            Ok::<_, RpcErr>(encoded)
        })
        .await;

        let encoded = match result {
            Ok(Ok(v)) => v,
            Ok(Err(e)) => return Err(e),
            Err(_) => {
                return Err(RpcErr::Internal(
                    "engine_getInclusionListV1 timed out".to_string(),
                ));
            }
        };

        // Serialize as `["0x...", "0x..."]`.
        let hex: Vec<String> = encoded
            .iter()
            .map(|b| format!("0x{}", hex::encode(b)))
            .collect();
        serde_json::to_value(hex).map_err(|e| RpcErr::Internal(e.to_string()))
    }
}
