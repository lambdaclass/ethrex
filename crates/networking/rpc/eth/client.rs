use std::collections::BTreeMap;

use ethrex_blockchain::Blockchain;
use ethrex_common::H32;
use ethrex_common::H160;
use ethrex_common::serde_utils;
use ethrex_common::types::Fork;
use ethrex_common::types::ForkBlobSchedule;
use ethrex_common::types::ForkId;
use ethrex_p2p::sync_manager::SyncManager;
use ethrex_storage::Store;
use ethrex_vm::{precompiles_for_fork, system_contracts::system_contracts_for_fork};
use serde::{Deserialize, Serialize};
use serde_json::{Value, value::RawValue};
use tracing::debug;

use crate::{
    rpc::{RpcApiContext, RpcHandler},
    utils::RpcErr,
};

pub struct ChainId;
impl RpcHandler for ChainId {
    fn parse(_params: &Option<Vec<Box<RawValue>>>) -> Result<Self, RpcErr> {
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
pub(crate) const SYNCED_HEAD_TOLERANCE: u64 = 8;

/// How far the executed head may trail before a node already reported as synced
/// is reported as syncing again. Wider than [`SYNCED_HEAD_TOLERANCE`] so the
/// `syncing` subscription does not flap for a node lagging by about that much.
/// A new subscriber gets the status for the feed's current band, so with 9 to 32
/// blocks of lag it can be told `false` while `eth_syncing` reports syncing.
pub(crate) const SYNCING_HEAD_TOLERANCE: u64 = 32;

/// A syncing node's progress, as block numbers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SyncProgress {
    pub starting_block: u64,
    pub current_block: u64,
    pub highest_block: u64,
}

/// What `eth_syncing` and the `syncing` subscription report.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncStatus {
    Synced,
    Syncing(SyncProgress),
}

impl SyncStatus {
    /// `eth_syncing`'s result: `false`, or `{startingBlock, currentBlock, highestBlock}`.
    pub fn to_rpc_value(self) -> Value {
        match self {
            Self::Synced => Value::Bool(false),
            Self::Syncing(progress) => serde_json::json!({
                "startingBlock": format!("{:#x}", progress.starting_block),
                "currentBlock": format!("{:#x}", progress.current_block),
                "highestBlock": format!("{:#x}", progress.highest_block),
            }),
        }
    }

    /// A `syncing` subscription's result, in reth's format: `eth_syncing`'s
    /// with `syncing: true` added to the object.
    pub fn to_subscription_value(self) -> Value {
        let mut value = self.to_rpc_value();
        if let Value::Object(fields) = &mut value {
            fields.insert("syncing".to_string(), Value::Bool(true));
        }
        value
    }
}

impl RpcHandler for Syncing {
    /// Ref: https://ethereum.org/en/developers/docs/apis/json-rpc/#eth_syncing
    fn parse(_params: &Option<Vec<Box<RawValue>>>) -> Result<Self, RpcErr> {
        Ok(Self {})
    }

    async fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        let Some(syncer) = &context.syncer else {
            return Err(RpcErr::Internal(
                "Syncing status requested but syncer is not initialized".to_string(),
            ));
        };
        let status = sync_status(
            &context.storage,
            &context.blockchain,
            syncer,
            SYNCED_HEAD_TOLERANCE,
        )
        .await?;
        Ok(status.to_rpc_value())
    }
}

/// The node counts as synced while its executed head trails the forkchoice
/// target by at most `head_tolerance` blocks.
pub(crate) async fn sync_status(
    storage: &Store,
    blockchain: &Blockchain,
    syncer: &SyncManager,
    head_tolerance: u64,
) -> Result<SyncStatus, RpcErr> {
    // `current_block` must reflect the executed/state head, not the canonical
    // pointer. An FCU can canonicalize blocks before their state is computed
    // (full-sync wedge), and snap may not have healed up to the head, so the
    // canonical head can be stateless. Reporting it would make the node look
    // near-synced while it has no state up to the tip. Use the canonical head
    // only when its post-state is on disk; otherwise report the executed head
    // recorded by the sync cycle.
    let canonical_head = storage.get_latest_block_number()?;
    let current_block = match storage.get_block_header(canonical_head)? {
        Some(header) if storage.has_state_root(header.state_root)? => canonical_head,
        _ => syncer
            .diagnostics()
            .read()
            .await
            .executed_head
            .min(canonical_head),
    };

    // `get_last_fcu_head` returns the head *hash* from the last forkchoiceUpdated.
    // Resolve it to a block number (the consensus-provided sync target). If the
    // header isn't canonical yet it may still be a pending block whose number we
    // can read; only when neither is available (e.g. mid snap-sync, target not
    // downloaded) fall back to the canonical head instead of reporting garbage.
    let head_hash = syncer
        .get_last_fcu_head()
        .map_err(|error| RpcErr::Internal(error.to_string()))?;
    let highest_block = match storage.get_block_number(head_hash).await? {
        Some(number) => number,
        None => match storage.get_pending_block(head_hash).await? {
            Some(block) => block.header.number,
            None => canonical_head,
        },
    };

    // `is_synced()` is a latch: it flips true on the first successful FCU and is
    // never reset on L1 (`set_not_synced` has no L1 caller), so it does not reflect
    // a node that synced once and later fell behind its consensus client. Trusting
    // it alone makes `eth_syncing` report `false` for a wedged/lagging node. Treat
    // the node as synced only if the latch is set AND the executed head has reached
    // (within a small following tolerance) the forkchoice target.
    let synced = blockchain.is_synced() && current_block + head_tolerance >= highest_block;

    if synced {
        return Ok(SyncStatus::Synced);
    }

    Ok(SyncStatus::Syncing(SyncProgress {
        // `starting_block` reuses `earliest_block_number` as a proxy. Note
        // this is the backfill frontier on a node with `--history.chain`
        // enabled, so it descends as historical backfill progresses rather
        // than marking where the current sync attempt started.
        starting_block: storage.get_earliest_block_number().await?,
        current_block,
        highest_block,
    }))
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
    fn parse(_params: &Option<Vec<Box<RawValue>>>) -> Result<Self, RpcErr> {
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
