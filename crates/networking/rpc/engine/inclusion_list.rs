//! EIP-7805 (FOCIL) machinery shared by `engine_newPayloadV6` and
//! `engine_forkchoiceUpdatedV5`: the inclusion lists retained per payload, and
//! the satisfaction verdict reported back as
//! `PayloadStatusV2.inclusionListSatisfied`.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Arc, Mutex};

use bytes::Bytes;
use ethrex_blockchain::inclusion_list_validator::{
    InclusionListSatisfactionValidator, StoreIlStateProvider,
};
use ethrex_common::H256;
use ethrex_common::types::Transaction;
use ethrex_crypto::NativeCrypto;
use tracing::debug;

use crate::{rpc::RpcApiContext, utils::RpcErr};

/// RLP-decode the EIP-2718 byte strings a consensus layer supplied as an
/// inclusion list, dropping the entries that do not decode.
///
/// Dropping rather than rejecting is deliberate. EIP-7805 has the consensus
/// layer aggregate inclusion lists produced by other nodes, so the bytes are
/// untrusted, and the engine API offers no way to reject one entry without
/// failing the whole call. An entry that cannot be decoded is also harmless:
/// it names no transaction, so it can impose no obligation on the block.
pub fn decode_inclusion_list(raw: &[Bytes], method: &str) -> Vec<Transaction> {
    let mut decoded = Vec::with_capacity(raw.len());
    for (index, bytes) in raw.iter().enumerate() {
        match Transaction::decode_canonical(bytes.as_ref()) {
            Ok(tx) => decoded.push(tx),
            Err(error) => debug!(
                index,
                %error,
                "{method}: skipping inclusion-list entry that is not a decodable transaction"
            ),
        }
    }
    decoded
}

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
/// refreshed from the block's post-state (with same-block withdrawal credits
/// discounted, since the check point precedes withdrawal processing), and
/// consulted once — no transaction is re-executed.
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
        .refresh_all_from(&post_state)
        .map_err(|e| RpcErr::Internal(format!("IL validator refresh failed: {e}")))?;

    let body = context
        .storage
        .get_block_body_by_hash(block_hash)
        .await
        .map_err(|e| RpcErr::Internal(e.to_string()))?
        .ok_or_else(|| {
            RpcErr::Internal("block body missing for IL satisfaction check".to_string())
        })?;
    // The satisfaction check evaluates senders at the post-transactions,
    // PRE-withdrawals point (EELS `apply_body` order), so the withdrawals'
    // credits must be discounted from the post-state balances.
    validator.discount_withdrawals(body.withdrawals.as_deref().unwrap_or_default());
    let block_tx_hashes: HashSet<H256> = body
        .transactions
        .iter()
        .map(|tx| tx.hash(&crypto))
        .collect();
    // `header.gas_used` is the maximum of EIP-8037's two gas dimensions, so this
    // is `min(execution_available, state_available)`. See the
    // `check_block_gas_capacity` note in `InclusionListSatisfactionValidator` for
    // what that costs and why the second dimension is not reachable from here.
    let gas_left = header.gas_limit.saturating_sub(header.gas_used);
    let chain_config = context.storage.get_chain_config();

    Ok(validator
        .check(
            inclusion_list,
            &block_tx_hashes,
            gas_left,
            &header,
            &chain_config,
            &crypto,
        )
        .is_ok())
}
