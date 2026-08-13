use std::collections::BTreeMap;

use ethrex_common::H32;
use ethrex_common::H160;
use ethrex_common::H256;
use ethrex_common::serde_utils;
use ethrex_common::types::BlockNumber;
use ethrex_common::types::Fork;
use ethrex_common::types::ForkBlobSchedule;
use ethrex_common::types::ForkId;
use ethrex_storage::Store;
use ethrex_vm::{precompiles_for_fork, system_contracts::system_contracts_for_fork};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tracing::debug;

use crate::{
    rpc::{RpcApiContext, RpcHandler},
    utils::RpcErr,
};

pub struct ChainId;
impl RpcHandler for ChainId {
    fn parse(_params: &Option<Vec<Value>>) -> Result<Self, RpcErr> {
        Ok(Self {})
    }

    async fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        debug!("Requested chain id");
        let chain_spec = context.storage.get_chain_config();
        serde_json::to_value(format!("{:#x}", chain_spec.chain_id))
            .map_err(|error| RpcErr::Internal(error.to_string()))
    }
}

pub struct Syncing;

/// How many blocks the executed head may trail the forkchoice target while still
/// being reported as synced. A node following head sits within a block or two of the
/// forkchoice head, so this absorbs normal per-slot lag and brief batch imports
/// without flapping, while a genuine fall-behind (hundreds of blocks) is reported as
/// syncing even though the `is_synced()` latch is still set.
const SYNCED_HEAD_TOLERANCE: u64 = 8;

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SyncingStatusRpc {
    #[serde(with = "serde_utils::u64::hex_str")]
    starting_block: u64,
    #[serde(with = "serde_utils::u64::hex_str")]
    current_block: u64,
    #[serde(with = "serde_utils::u64::hex_str")]
    highest_block: u64,
}

/// Whether the canonical head's post-state is one this node actually holds.
///
/// `eth_syncing` reports the canonical head as `current_block` only when this
/// is true; otherwise it falls back to the executed head, so a node whose
/// canonical pointer has run ahead of its state does not advertise itself as
/// near-synced.
///
/// Extracted so it can be asserted on directly: the check must be made *per
/// header*, since past `binaryTreeTime` the head's `state_root` is a
/// binary-trie root that resolves against no MPT node. Inline and root-only, it
/// silently under-reported the head on every scheduled chain.
pub async fn canonical_head_is_stateful(
    storage: &Store,
    canonical_head: BlockNumber,
) -> Result<bool, RpcErr> {
    let Some(header) = storage.get_block_header(canonical_head)? else {
        return Ok(false);
    };
    Ok(storage.has_state_for_header(header.hash(), &header)?)
}

/// The block number `eth_syncing` reports as `highestBlock`: how far the
/// consensus client says the chain has got.
///
/// `head_hash` is the last forkchoiceUpdated head. Resolving it locally is the
/// best answer, and a pending block is the next best. `recorded_target` is the
/// sync cycle's own view of the target, which is the answer that matters when
/// neither lookup succeeds — that is exactly the state of a node too far behind
/// to have downloaded the target yet.
///
/// **Falling straight back to `canonical_head` is what made a wedged node
/// invisible.** On the 2026-08-07 devnet a node stuck 128 blocks behind, failing
/// every sync cycle, answered
/// `{"currentBlock":"0x36","highestBlock":"0x36"}` — the target collapsed onto
/// the local head, so the one number that would have shown the node was behind
/// instead agreed that it had arrived. Its logs knew better
/// (`Sync target from consensus forkchoice: block 144`); nothing carried that
/// into the RPC. `canonical_head` survives only as a last resort, for a node
/// that has genuinely never been told a target.
pub async fn resolve_highest_block(
    storage: &Store,
    head_hash: H256,
    recorded_target: Option<BlockNumber>,
    canonical_head: BlockNumber,
) -> Result<SyncTarget, RpcErr> {
    if let Some(number) = storage.get_block_number(head_hash).await? {
        return Ok(SyncTarget::Known(number));
    }
    if let Some(block) = storage.get_pending_block(head_hash).await? {
        return Ok(SyncTarget::Known(block.header.number));
    }
    if let Some(target) = recorded_target {
        // Never report a target *below* the canonical head: a stale one would
        // otherwise make a node that has caught up look ahead of itself.
        return Ok(SyncTarget::Known(target.max(canonical_head)));
    }
    Ok(SyncTarget::Unknown(canonical_head))
}

/// How far the consensus client says the chain has got — and crucially, whether
/// this node actually knows.
///
/// The distinction is the whole point. A node that cannot resolve the forkchoice
/// head and has never been told a target does not know where the chain is; the
/// only number it can put in `highestBlock` is its own head, which is a stand-in,
/// not a measurement. Collapsing those two cases into a bare `BlockNumber` is
/// what let a node 130 blocks behind report `false` from `eth_syncing`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncTarget {
    /// The consensus client's head, as far as this node can tell.
    Known(BlockNumber),
    /// No target known. Carries the local head purely as the number to report.
    Unknown(BlockNumber),
}

impl SyncTarget {
    /// The number to report as `highestBlock`.
    pub fn number(&self) -> BlockNumber {
        match self {
            Self::Known(n) | Self::Unknown(n) => *n,
        }
    }

    pub fn is_known(&self) -> bool {
        matches!(self, Self::Known(_))
    }
}

/// Whether `eth_syncing` should report the node as synced (i.e. return `false`).
///
/// Requires all three: the `is_synced()` latch, a target this node actually
/// knows, and an executed head that has reached it within
/// [`SYNCED_HEAD_TOLERANCE`].
///
/// **The middle condition is the one that was missing.** With `highest_block`
/// derived from `canonical_head` it equalled `current_block` by construction, so
/// the distance test was trivially true and the latch alone decided the answer.
/// A node 130 blocks behind reported `false` the moment the latch flipped.
pub fn is_reported_synced(latched: bool, current_block: BlockNumber, target: &SyncTarget) -> bool {
    latched && target.is_known() && current_block + SYNCED_HEAD_TOLERANCE >= target.number()
}

impl RpcHandler for Syncing {
    /// Ref: https://ethereum.org/en/developers/docs/apis/json-rpc/#eth_syncing
    fn parse(_params: &Option<Vec<Value>>) -> Result<Self, RpcErr> {
        Ok(Self {})
    }

    async fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        let Some(syncer) = &context.syncer else {
            return Err(RpcErr::Internal(
                "Syncing status requested but syncer is not initialized".to_string(),
            ));
        };

        // `current_block` must reflect the executed/state head, not the canonical
        // pointer. An FCU can canonicalize blocks before their state is computed
        // (full-sync wedge), and snap may not have healed up to the head, so the
        // canonical head can be stateless. Reporting it would make the node look
        // near-synced while it has no state up to the tip. Use the canonical head
        // only when its post-state is on disk; otherwise report the executed head
        // recorded by the sync cycle.
        let canonical_head = context.storage.get_latest_block_number()?;
        // Deliberately `canonical_head_is_stateful`, not an inlined
        // `has_state_root`: that predicate is MPT-only, and asking it about a
        // binary-committed header is what halted a devnet permanently at the
        // activation block. The helper routes through `has_state_for_header`,
        // which answers for whichever commitment the header actually carries.
        let current_block = if canonical_head_is_stateful(&context.storage, canonical_head).await? {
            canonical_head
        } else {
            syncer
                .diagnostics()
                .read()
                .await
                .executed_head
                .min(canonical_head)
        };

        // `get_last_fcu_head` returns the head *hash* from the last forkchoiceUpdated.
        // See `resolve_highest_block` for the fallback order and why the
        // recorded sync target has to come before the canonical head.
        let head_hash = syncer
            .get_last_fcu_head()
            .map_err(|error| RpcErr::Internal(error.to_string()))?;
        let recorded_target = syncer.diagnostics().read().await.sync_target;
        let target =
            resolve_highest_block(&context.storage, head_hash, recorded_target, canonical_head)
                .await?;
        let highest_block = target.number();

        // `is_synced()` is a latch: it flips true on the first successful FCU and is
        // never reset on L1 (`set_not_synced` has no L1 caller), so it does not reflect
        // a node that synced once and later fell behind its consensus client. Trusting
        // it alone makes `eth_syncing` report `false` for a wedged/lagging node. Treat
        // the node as synced only if the latch is set AND the executed head has reached
        // (within a small following tolerance) the forkchoice target.
        if is_reported_synced(context.blockchain.is_synced(), current_block, &target) {
            return Ok(Value::Bool(false));
        }

        let syncing_status = SyncingStatusRpc {
            starting_block: context.storage.get_earliest_block_number().await?,
            current_block,
            highest_block,
        };
        serde_json::to_value(syncing_status).map_err(|error| RpcErr::Internal(error.to_string()))
    }
}

pub struct Config;

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct EthConfigObject {
    activation_time: Option<u64>,
    blob_schedule: Option<ForkBlobSchedule>,
    #[serde(with = "serde_utils::u64::hex_str")]
    chain_id: u64,
    fork_id: H32,
    precompiles: BTreeMap<String, H160>,
    system_contracts: BTreeMap<String, H160>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EthConfigResponse {
    current: EthConfigObject,
    next: Option<EthConfigObject>,
    last: Option<EthConfigObject>,
}

impl RpcHandler for Config {
    fn parse(_params: &Option<Vec<Value>>) -> Result<Self, RpcErr> {
        Ok(Self {})
    }

    async fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        let chain_config = context.storage.get_chain_config();
        let Some(latest_block) = context
            .storage
            .get_block_by_number(context.storage.get_latest_block_number()?)
            .await?
        else {
            return Err(RpcErr::Internal("Failed to fetch latest block".to_string()));
        };

        let latest_block_timestamp = latest_block.header.timestamp;
        let current_fork = chain_config.get_fork(latest_block_timestamp);

        if current_fork < Fork::Paris {
            return Err(RpcErr::UnsupportedFork(
                "eth-config is not supported for forks prior to Paris".to_string(),
            ));
        }

        let current = get_config_for_fork(current_fork, &context).await?;
        let next = if let Some(next_fork) = chain_config.next_fork(latest_block_timestamp) {
            Some(get_config_for_fork(next_fork, &context).await?)
        } else {
            None
        };
        let last_fork = chain_config.get_last_scheduled_fork();
        let last = if last_fork > current_fork {
            Some(get_config_for_fork(last_fork, &context).await?)
        } else {
            None
        };
        let response = EthConfigResponse {
            current,
            next,
            last,
        };

        serde_json::to_value(response).map_err(|error| RpcErr::Internal(error.to_string()))
    }
}

async fn get_config_for_fork(
    fork: Fork,
    context: &RpcApiContext,
) -> Result<EthConfigObject, RpcErr> {
    let chain_config = context.storage.get_chain_config();
    let activation_time = chain_config.get_activation_timestamp_for_fork(fork);
    let genesis_header = context
        .storage
        .get_block_by_number(0)
        .await?
        .expect("Failed to get genesis block. This should not happen.")
        .header;
    let block_number = context.storage.get_latest_block_number()?;
    let fork_id = if let Some(timestamp) = activation_time {
        ForkId::new(chain_config, genesis_header, timestamp, block_number).fork_hash
    } else {
        H32::zero()
    };
    let mut system_contracts = BTreeMap::new();
    for contract in system_contracts_for_fork(fork) {
        system_contracts.insert(contract.name.to_string(), contract.address);
    }

    let mut precompiles = BTreeMap::new();

    for precompile in precompiles_for_fork(fork) {
        precompiles.insert(precompile.name.to_string(), precompile.address);
    }

    Ok(EthConfigObject {
        activation_time,
        blob_schedule: chain_config.get_blob_schedule_for_fork(fork),
        chain_id: chain_config.chain_id,
        fork_id,
        precompiles,
        system_contracts,
    })
}
