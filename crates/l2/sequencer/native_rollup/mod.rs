//! Native Rollup L2 PoC — parallel L2 mode where blocks are produced and
//! committed via the EXECUTE precompile on L1.
//!
//! This module provides the actors that implement the native rollup L2 lifecycle:
//!
//! - **NativeL1Watcher**: polls L1 for `L1MessageRecorded` events and forwards
//!   them to the block producer via `EnqueueL1Messages` messages
//! - **NativeBlockProducer**: owns the pending-L1-messages queue, drains it to
//!   build relayer txs, and uses the standard payload builder flow to produce blocks
//! - **NativeL1Advancer**: reads produced blocks from the Store, generates an
//!   execution witness, and submits via advance()

pub mod block_producer;
pub mod l1_advancer;
pub mod l1_watcher;
pub mod types;

use std::sync::Arc;

use ethrex_blockchain::Blockchain;
use ethrex_common::Address;
use ethrex_l2_rpc::signer::Signer;
use ethrex_rpc::clients::eth::EthClient;
use reqwest::Url;
use spawned_concurrency::tasks::ActorRef;
use tracing::info;

use block_producer::{NativeBlockProducer, NativeBlockProducerConfig};
use l1_advancer::NativeL1Advancer;
use l1_watcher::NativeL1Watcher;

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
    /// L1 advancer interval in milliseconds.
    pub advance_interval_ms: u64,
    /// Maximum number of L1 blocks to scan per poll.
    pub max_block_step: u64,
    /// Coinbase address for produced L2 blocks.
    pub coinbase: Address,
    /// Block gas limit for L2 blocks.
    pub block_gas_limit: u64,
    /// L2 chain ID.
    pub chain_id: u64,
    /// Signer for the relayer that signs L2Bridge.processL1Message txs.
    pub relayer_signer: Signer,
    /// Signer for L1 transactions (advance() calls).
    pub l1_signer: Signer,
}

/// Start the native rollup L2 actors.
///
/// Spawns three actors:
/// 1. NativeL1Watcher — polls L1 for L1MessageRecorded events
/// 2. NativeBlockProducer — drains L1 messages, builds relayer txs, produces blocks
/// 3. NativeL1Advancer — reads blocks from Store, generates witness, submits to L1
///
/// Returns refs to the spawned actors.
#[allow(clippy::type_complexity)]
pub fn start_native_rollup_l2(
    store: Store,
    blockchain: Arc<Blockchain>,
    config: NativeRollupConfig,
) -> Result<
    (
        ActorRef<NativeL1Watcher>,
        ActorRef<NativeBlockProducer>,
        ActorRef<NativeL1Advancer>,
    ),
    Box<dyn std::error::Error>,
> {
    info!("Starting Native Rollup L2");
    info!("  Contract: {:?}", config.contract_address);
    info!("  Coinbase: {:?}", config.coinbase);
    info!("  Chain ID: {}", config.chain_id);

    // Create EthClient for L1
    let eth_client = EthClient::new_with_multiple_urls(config.l1_rpc_urls.clone())?;

    // 1. Spawn NativeBlockProducer first so the watcher can hold its ActorRef.
    let relayer_address = config.relayer_signer.address();
    let producer_config = NativeBlockProducerConfig {
        block_time_ms: config.block_time_ms,
        coinbase: config.coinbase,
        block_gas_limit: config.block_gas_limit,
        chain_id: config.chain_id,
        relayer_signer: config.relayer_signer,
    };
    let producer_ref =
        NativeBlockProducer::spawn(store.clone(), producer_config, blockchain.clone());
    info!("  NativeBlockProducer started");

    // 2. Spawn NativeL1Watcher with a handle to the block producer.
    let watcher_ref = NativeL1Watcher::spawn(
        eth_client.clone(),
        config.contract_address,
        producer_ref.clone(),
        config.watch_interval_ms,
        config.max_block_step,
    );
    info!("  NativeL1Watcher started");

    // 3. Spawn NativeL1Advancer
    let advancer_ref = NativeL1Advancer::spawn(
        eth_client,
        config.contract_address,
        config.l1_signer,
        store,
        blockchain,
        relayer_address,
        config.advance_interval_ms,
    );
    info!("  NativeL1Advancer started");

    Ok((watcher_ref, producer_ref, advancer_ref))
}
