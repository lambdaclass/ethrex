//! Block Builder GenServer implementation.
//!
//! Receives transactions and builds blocks either on-demand (default) or at
//! specified intervals.

use std::{
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use bytes::Bytes;
use ethereum_types::H256;
use ethrex_blockchain::{
    Blockchain, BlockchainOptions,
    fork_choice::apply_fork_choice,
    payload::{BuildPayloadArgs, create_payload},
};
use ethrex_common::{
    Address,
    types::{BlobsBundle, Block, DEFAULT_BUILDER_GAS_CEIL, ELASTICITY_MULTIPLIER, Transaction},
};
use ethrex_config::networks::Network;
use ethrex_storage::{EngineType, Store};
use spawned_concurrency::tasks::{CallResponse, CastResponse, GenServer, GenServerHandle};
use tokio::time::interval;
use tracing::{error, info};

use crate::error::BlockBuilderError;

#[derive(Clone)]
pub struct BlockBuilderConfig {
    pub coinbase: Address,
    /// `None` means on-demand mode (build immediately per transaction).
    pub block_time_ms: Option<u64>,
    pub gas_ceil: u64,
}

impl Default for BlockBuilderConfig {
    fn default() -> Self {
        Self {
            coinbase: Address::zero(),
            block_time_ms: None,
            gas_ceil: DEFAULT_BUILDER_GAS_CEIL,
        }
    }
}

pub struct BlockBuilder {
    store: Store,
    blockchain: Arc<Blockchain>,
    config: BlockBuilderConfig,
    /// Queued transactions for interval mode.
    pending_txs: Vec<(Transaction, Option<BlobsBundle>)>,
    last_timestamp: u64,
}

/// Synchronous (request/response) messages.
#[derive(Clone, Debug)]
pub enum CallMsg {
    GetBlockNumber,
    GetHeadBlockHash,
    GetPendingTxCount,
}

/// Asynchronous (fire-and-forget) messages.
#[derive(Clone, Debug)]
pub enum CastMsg {
    SubmitTransaction {
        tx: Box<Transaction>,
        blobs_bundle: Option<BlobsBundle>,
    },
    /// Timer tick for interval mode.
    BuildBlock,
}

/// Responses from the block builder GenServer.
#[derive(Clone, Debug)]
pub enum OutMsg {
    BlockNumber(u64),
    HeadBlockHash(H256),
    PendingTxCount(usize),
    Error(String),
}

impl BlockBuilder {
    pub async fn new(config: BlockBuilderConfig) -> Result<Self, BlockBuilderError> {
        let network = Network::LocalDevnet;
        let genesis = network
            .get_genesis()
            .map_err(|e| BlockBuilderError::Genesis(e.to_string()))?;

        let mut store =
            Store::new("memory", EngineType::InMemory).map_err(BlockBuilderError::Store)?;

        store
            .add_initial_state(genesis.clone())
            .await
            .map_err(BlockBuilderError::Store)?;

        let blockchain = Arc::new(Blockchain::new(store.clone(), BlockchainOptions::default()));

        let last_timestamp = genesis.timestamp;

        Ok(Self {
            store,
            blockchain,
            config,
            pending_txs: Vec::new(),
            last_timestamp,
        })
    }

    pub fn store(&self) -> Store {
        self.store.clone()
    }

    pub fn blockchain(&self) -> Arc<Blockchain> {
        self.blockchain.clone()
    }

    /// Spawn the block builder and optionally start the interval timer.
    /// Returns the handle, store, and blockchain for use by RPC context.
    pub async fn spawn(
        config: BlockBuilderConfig,
    ) -> Result<(GenServerHandle<BlockBuilder>, Store, Arc<Blockchain>), BlockBuilderError> {
        let block_time_ms = config.block_time_ms;
        let builder = Self::new(config).await?;
        let store = builder.store();
        let blockchain = builder.blockchain();
        let handle = builder.start();

        if let Some(ms) = block_time_ms {
            let mut timer_handle = handle.clone();
            tokio::spawn(async move {
                let mut timer = interval(Duration::from_millis(ms));
                loop {
                    timer.tick().await;
                    if let Err(e) = timer_handle.cast(CastMsg::BuildBlock).await {
                        error!("Block builder timer error: {e}");
                        break;
                    }
                }
            });
        }

        Ok((handle, store, blockchain))
    }

    /// Returns a strictly increasing timestamp (ensures unique block timestamps).
    fn next_timestamp(&mut self) -> u64 {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        // Ensure timestamp is always increasing
        let timestamp = std::cmp::max(now, self.last_timestamp + 1);
        self.last_timestamp = timestamp;
        timestamp
    }

    async fn build_block_with_txs(
        &mut self,
        txs: Vec<(Transaction, Option<BlobsBundle>)>,
    ) -> Result<Block, BlockBuilderError> {
        if txs.is_empty() {
            return Err(BlockBuilderError::Internal(
                "No transactions to build block".to_string(),
            ));
        }

        let head_block_header = {
            let current_block_number = self.store.get_latest_block_number().await?;
            self.store
                .get_block_header(current_block_number)?
                .ok_or_else(|| BlockBuilderError::Internal("Head block not found".to_string()))?
        };

        let timestamp = self.next_timestamp();

        let build_payload_args = BuildPayloadArgs {
            parent: head_block_header.hash(),
            timestamp,
            fee_recipient: self.config.coinbase,
            random: H256::zero(),
            withdrawals: Some(Vec::new()),
            beacon_root: Some(H256::zero()),
            slot_number: None,
            version: 3,
            elasticity_multiplier: ELASTICITY_MULTIPLIER,
            gas_ceil: self.config.gas_ceil,
        };

        let payload_id = build_payload_args
            .id()
            .map_err(|e| BlockBuilderError::Internal(e.to_string()))?;

        let payload = create_payload(&build_payload_args, &self.store, Bytes::new())?;

        // Note: Transactions are already in the mempool (added by RPC handler before sending to builder)
        // Build the payload (this will pick up transactions from mempool)
        self.blockchain
            .clone()
            .initiate_payload_build(payload.clone(), payload_id)
            .await;

        let result = self
            .blockchain
            .get_payload(payload_id)
            .await
            .map_err(BlockBuilderError::Chain)?;

        let block = result.payload;

        // Skip if the block has no transactions (tx was already included in a previous block)
        if block.body.transactions.is_empty() {
            return Err(BlockBuilderError::Internal(
                "Transaction already included in a previous block".to_string(),
            ));
        }

        // Add the block to the chain
        self.blockchain
            .add_block(block.clone())
            .map_err(BlockBuilderError::Chain)?;

        // Update fork choice
        let block_hash = block.hash();
        apply_fork_choice(&self.store, block_hash, block_hash, block_hash)
            .await
            .map_err(|e| BlockBuilderError::Internal(format!("Fork choice failed: {}", e)))?;

        // Remove transactions from mempool (they're now in a block)
        self.blockchain
            .remove_block_transactions_from_pool(&block)
            .map_err(BlockBuilderError::Store)?;

        info!(
            number = block.header.number,
            txs = block.body.transactions.len(),
            hash = %format!("{block_hash:#x}"),
            "Block mined"
        );

        Ok(block)
    }

    /// Build a block immediately with a single transaction (on-demand mode).
    async fn handle_transaction_on_demand(
        &mut self,
        tx: Transaction,
        blobs_bundle: Option<BlobsBundle>,
    ) -> Result<(), BlockBuilderError> {
        // Build a block immediately with just this transaction
        self.build_block_with_txs(vec![(tx, blobs_bundle)]).await?;
        Ok(())
    }

    /// Queue a transaction for the next interval-mode block.
    fn handle_transaction_interval(&mut self, tx: Transaction, blobs_bundle: Option<BlobsBundle>) {
        self.pending_txs.push((tx, blobs_bundle));
        info!(pending = self.pending_txs.len(), "Transaction queued");
    }

    async fn build_pending_block(&mut self) -> Result<(), BlockBuilderError> {
        if self.pending_txs.is_empty() {
            return Ok(());
        }

        let txs = std::mem::take(&mut self.pending_txs);
        self.build_block_with_txs(txs).await?;
        Ok(())
    }
}

impl GenServer for BlockBuilder {
    type CallMsg = CallMsg;
    type CastMsg = CastMsg;
    type OutMsg = OutMsg;
    type Error = BlockBuilderError;

    async fn handle_call(
        &mut self,
        message: Self::CallMsg,
        _handle: &GenServerHandle<Self>,
    ) -> CallResponse<Self> {
        let response = match message {
            CallMsg::GetBlockNumber => match self.store.get_latest_block_number().await {
                Ok(number) => OutMsg::BlockNumber(number),
                Err(e) => OutMsg::Error(e.to_string()),
            },
            CallMsg::GetHeadBlockHash => match self.store.get_latest_block_number().await {
                Ok(number) => match self.store.get_block_header(number) {
                    Ok(Some(header)) => OutMsg::HeadBlockHash(header.hash()),
                    Ok(None) => OutMsg::Error("Head block not found".to_string()),
                    Err(e) => OutMsg::Error(e.to_string()),
                },
                Err(e) => OutMsg::Error(e.to_string()),
            },
            CallMsg::GetPendingTxCount => OutMsg::PendingTxCount(self.pending_txs.len()),
        };
        CallResponse::Reply(response)
    }

    async fn handle_cast(
        &mut self,
        message: Self::CastMsg,
        _handle: &GenServerHandle<Self>,
    ) -> CastResponse {
        match message {
            CastMsg::SubmitTransaction { tx, blobs_bundle } => {
                let tx = *tx;
                let tx_hash = tx.hash();

                if self.config.block_time_ms.is_some() {
                    self.handle_transaction_interval(tx, blobs_bundle);
                } else if let Err(e) = self.handle_transaction_on_demand(tx, blobs_bundle).await {
                    error!(tx = %format!("{tx_hash:#x}"), "Failed to build block for tx: {e}");
                }
            }
            CastMsg::BuildBlock => {
                if let Err(e) = self.build_pending_block().await {
                    error!("Failed to build pending block: {e}");
                }
            }
        }
        CastResponse::NoReply
    }
}

