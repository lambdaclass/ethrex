//! Native Rollup L2 PoC — parallel L2 mode where blocks are produced and
//! committed via the EXECUTE precompile on L1.
//!
//! This module provides the actors (GenServers) that implement the native
//! rollup L2 lifecycle:
//!
//! - **NativeL1Watcher**: polls L1 for `L1MessageRecorded` events
//! - **NativeBlockProducer**: produces L2 blocks compatible with EXECUTE
//! - **NativeL1Committer**: submits produced blocks to L1 via advance()
//!
//! All communication between actors happens through shared thread-safe queues.

pub mod block_producer;
pub mod l1_committer;
pub mod l1_watcher;
pub mod types;

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use ethrex_common::Address;
use ethrex_l2_rpc::signer::Signer;
use ethrex_rpc::clients::eth::EthClient;
use reqwest::Url;
use spawned_concurrency::tasks::{GenServer, GenServerHandle};
use tracing::info;

use block_producer::{NativeBlockProducer, NativeBlockProducerConfig};
use l1_committer::NativeL1Committer;
use l1_watcher::NativeL1Watcher;
use types::{PendingL1Messages, ProducedBlocks};

use ethrex_storage::Store;

/// Configuration for the native rollup L2.
#[derive(Clone, Debug)]
pub struct NativeRollupConfig {
    /// L1 RPC URL(s) for watching events and submitting transactions.
    pub l1_rpc_urls: Vec<Url>,
    /// Address of the NativeRollup.sol contract on L1.
    pub contract_address: Address,
    /// Block production interval in milliseconds.
    pub block_time_ms: u64,
    /// L1 watcher polling interval in milliseconds.
    pub watch_interval_ms: u64,
    /// L1 committer interval in milliseconds.
    pub commit_interval_ms: u64,
    /// Maximum number of L1 blocks to scan per poll.
    pub max_block_step: u64,
    /// Coinbase address for produced L2 blocks.
    pub coinbase: Address,
    /// Block gas limit for L2 blocks.
    pub block_gas_limit: u64,
    /// L2 chain ID.
    pub chain_id: u64,
    /// Relayer private key (32 bytes) for signing L2Bridge.processL1Message txs.
    pub relayer_key: [u8; 32],
    /// Signer for L1 transactions (advance() calls).
    pub l1_signer: Signer,
    /// Runtime bytecode of the L2Bridge contract.
    pub bridge_runtime: Vec<u8>,
    /// Runtime bytecode of the L1Anchor contract.
    pub anchor_runtime: Vec<u8>,
}

/// Start the native rollup L2 actors.
///
/// Spawns three GenServers:
/// 1. NativeL1Watcher — polls L1 for L1MessageRecorded events
/// 2. NativeBlockProducer — produces L2 blocks
/// 3. NativeL1Committer — submits blocks to L1 via advance()
///
/// Returns handles to the spawned actors.
#[allow(clippy::type_complexity)]
pub fn start_native_rollup_l2(
    store: Store,
    config: NativeRollupConfig,
) -> Result<
    (
        GenServerHandle<NativeL1Watcher>,
        GenServerHandle<NativeBlockProducer>,
        GenServerHandle<NativeL1Committer>,
    ),
    Box<dyn std::error::Error>,
> {
    info!("Starting Native Rollup L2");
    info!("  Contract: {:?}", config.contract_address);
    info!("  Coinbase: {:?}", config.coinbase);
    info!("  Chain ID: {}", config.chain_id);

    // Shared queues
    let pending_l1_messages: PendingL1Messages = Arc::new(Mutex::new(VecDeque::new()));
    let produced_blocks: ProducedBlocks = Arc::new(Mutex::new(VecDeque::new()));

    // Create EthClient for L1
    let eth_client = EthClient::new_with_multiple_urls(config.l1_rpc_urls.clone())?;

    // 1. Spawn NativeL1Watcher
    let watcher = NativeL1Watcher::new(
        eth_client.clone(),
        config.contract_address,
        pending_l1_messages.clone(),
        config.watch_interval_ms,
        config.max_block_step,
    );
    let watcher_handle = watcher.start();
    info!("  NativeL1Watcher started");

    // 2. Spawn NativeBlockProducer
    let producer_config = NativeBlockProducerConfig {
        block_time_ms: config.block_time_ms,
        coinbase: config.coinbase,
        block_gas_limit: config.block_gas_limit,
        chain_id: config.chain_id,
        relayer_key: config.relayer_key,
    };
    let producer = NativeBlockProducer::new(
        store,
        producer_config,
        pending_l1_messages,
        produced_blocks.clone(),
        config.bridge_runtime,
        config.anchor_runtime,
    );
    let producer_handle = producer.start();
    info!("  NativeBlockProducer started");

    // 3. Spawn NativeL1Committer
    let committer = NativeL1Committer::new(
        eth_client,
        config.contract_address,
        config.l1_signer,
        produced_blocks,
        config.commit_interval_ms,
    );
    let committer_handle = committer.start();
    info!("  NativeL1Committer started");

    Ok((watcher_handle, producer_handle, committer_handle))
}
