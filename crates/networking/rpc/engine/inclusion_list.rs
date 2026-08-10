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
use ethrex_blockchain::focil_profile2::BlockchainProfile2Evaluator;
use ethrex_blockchain::inclusion_list_builder::InclusionListBuilder;
use ethrex_blockchain::inclusion_list_validator::{
    InclusionListSatisfactionValidator, StoreIlStateProvider,
};
use ethrex_common::H256;
use ethrex_common::types::{MAX_BYTES_PER_INCLUSION_LIST, Transaction};
use ethrex_crypto::NativeCrypto;
use serde_json::Value;
use tracing::{debug, info};

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
    by_block: HashMap<H256, Vec<Transaction>>,
    order: VecDeque<H256>,
}

impl RetainedInclusionLists {
    pub fn insert(&mut self, block_hash: H256, transactions: Vec<Transaction>) {
        if self.by_block.insert(block_hash, transactions).is_none() {
            self.order.push_back(block_hash);
        }
        while self.order.len() > MAX_RETAINED_INCLUSION_LISTS {
            if let Some(evicted) = self.order.pop_front() {
                self.by_block.remove(&evicted);
            }
        }
    }

    pub fn get(&self, block_hash: &H256) -> Option<&[Transaction]> {
        self.by_block.get(block_hash).map(Vec::as_slice)
    }
}

/// Shared handle to [`RetainedInclusionLists`]. The critical section only ever
/// clones a transaction list out or moves one in, so it never spans an `await`.
pub type RetainedInclusionListsHandle = Arc<Mutex<RetainedInclusionLists>>;

/// Runs the EIP-7805 (FOCIL) satisfaction algorithm for `block_hash` against
/// `inclusion_list`, reporting whether the block satisfies it.
///
/// An empty inclusion list is trivially satisfied. The algorithm is a pure
/// state-comparison pass: the validator is seeded from the parent's pre-state,
/// refreshed from the block's post-state, and consulted once — no transaction
/// is re-executed.
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
    let parent_header = context
        .storage
        .get_block_header_by_hash(header.parent_hash)
        .map_err(|e| RpcErr::Internal(e.to_string()))?
        .ok_or_else(|| RpcErr::Internal("parent missing for IL satisfaction check".to_string()))?;

    let pre_state = StoreIlStateProvider {
        store: &context.storage,
        state_root: parent_header.state_root,
    };
    let post_state = StoreIlStateProvider {
        store: &context.storage,
        state_root: header.state_root,
    };
    let crypto = NativeCrypto;
    let mut validator =
        InclusionListSatisfactionValidator::new(inclusion_list, &pre_state, &crypto)
            .map_err(|e| RpcErr::Internal(format!("IL validator init failed: {e}")))?;
    validator
        .refresh_all_from(&post_state, &crypto)
        .map_err(|e| RpcErr::Internal(format!("IL validator refresh failed: {e}")))?;

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
        .map(|tx| tx.hash(&crypto))
        .collect();
    let gas_left = header.gas_limit.saturating_sub(header.gas_used);
    let chain_config = context.storage.get_chain_config();

    let profile_2 = BlockchainProfile2Evaluator::new(
        &context.blockchain,
        &header,
        parent_header.state_root,
        gas_left,
    );
    let report = validator.check_with_profile_2(
        inclusion_list,
        &block_tx_hashes,
        gas_left,
        &header,
        &chain_config,
        &crypto,
        Some(&profile_2),
    );

    // Log every Profile 2 outcome, including the ones that reached no
    // verdict, so an operator can see what the replay decided rather than
    // inferring it from the single `unsatisfied` value.
    for tx_hash in &report.profile_2_unjustified {
        info!(
            target: "focil::profile2",
            %tx_hash,
            %block_hash,
            "profile_2_unjustified: omitted frame tx would have passed EIP-8369 Profile 2 eligibility replay"
        );
    }
    for tx_hash in &report.profile_2_undecided {
        info!(
            target: "focil::profile2",
            %tx_hash,
            %block_hash,
            "profile_2_undecided: omitted frame tx's EIP-8369 Profile 2 eligibility could not be decided"
        );
    }

    Ok(report.unsatisfied.is_none())
}

#[derive(Debug)]
pub struct GetInclusionListV1Request;

impl RpcHandler for GetInclusionListV1Request {
    fn parse(params: &Option<Vec<Value>>) -> Result<Self, RpcErr> {
        // The engine API defines `engine_getInclusionListV1` with `params: []`.
        //
        // A single parameter is also accepted and ignored: every consensus client
        // that implements FOCIL today sends a `parentHash` here, because they
        // were written against the pre-spec design and the engine-api FOCIL
        // methods only landed on 2026-08-03. Ignoring it is safe rather than
        // merely lenient, because the list is built against this node's canonical
        // head, which is exactly the block that hash identifies.
        //
        // Remove this once a client ships the specified signature.
        match params {
            None => Ok(Self),
            Some(params) if params.len() <= 1 => Ok(Self),
            Some(_) => Err(RpcErr::BadParams("Expected no params".to_owned())),
        }
    }

    async fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        debug!("engine_getInclusionListV1");

        // Build the IL inside `tokio::time::timeout`. The builder is sync but
        // wrapping it in spawn_blocking would be heavier than necessary; the
        // 1-second deadline is generous for the in-memory work the builder
        // performs.
        let result = tokio::time::timeout(GET_INCLUSION_LIST_V1_TIMEOUT, async move {
            // The list constrains the next block, so it is built against the
            // node's canonical head — the parent of that block.
            let head_number = context
                .storage
                .get_latest_block_number()
                .await
                .map_err(|e| RpcErr::Internal(e.to_string()))?;
            let head_header = context
                .storage
                .get_block_header(head_number)
                .map_err(|e| RpcErr::Internal(e.to_string()))?
                .ok_or_else(|| RpcErr::Internal("canonical head header missing".to_string()))?;

            let state = StoreIlStateProvider {
                store: &context.storage,
                state_root: head_header.state_root,
            };
            let base_fee = head_header.base_fee_per_gas.unwrap_or(0);

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
