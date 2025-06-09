use std::{cmp::min, collections::HashMap, sync::Arc, time::Duration};

use ethrex_blockchain::{fork_choice::apply_fork_choice, vm::StoreVmDatabase, Blockchain};
use ethrex_common::{
    types::{
        batch::Batch, AccountUpdate, Block, BlockNumber, PrivilegedL2Transaction, Transaction,
    },
    Address, H160, H256, U256,
};
use ethrex_l2_common::state_diff::prepare_state_diff;
use ethrex_rlp::decode::RLPDecode;
use ethrex_rpc::{types::receipt::RpcLog, utils::get_withdrawal_hash, EthClient};
use ethrex_storage::Store;
use ethrex_storage_rollup::StoreRollup;
use ethrex_vm::{Evm, EvmEngine};
use keccak_hash::keccak;
use tokio::{sync::Mutex, time::sleep};
use tracing::{debug, error, info};

use crate::{
    based::sequencer_state::SequencerState,
    sequencer::{errors::SequencerError, l1_committer::generate_blobs_bundle},
    utils::helpers::is_withdrawal_l2,
    SequencerConfig,
};

#[derive(Debug, thiserror::Error)]
pub enum BlockFetcherError {
    #[error("Block Fetcher failed due to an EthClient error: {0}")]
    EthClientError(#[from] ethrex_rpc::clients::EthClientError),
    #[error("Block Fetcher failed due to a Store error: {0}")]
    StoreError(#[from] ethrex_storage::error::StoreError),
    #[error("Internal Error: {0}")]
    InternalError(String),
    #[error("Failed to store fetched block: {0}")]
    ChainError(#[from] ethrex_blockchain::error::ChainError),
    #[error("Failed to apply fork choice for fetched block: {0}")]
    InvalidForkChoice(#[from] ethrex_blockchain::error::InvalidForkChoice),
    #[error("Failed to push fetched block to execution cache: {0}")]
    ExecutionCacheError(#[from] crate::sequencer::errors::ExecutionCacheError),
    #[error("Failed to RLP decode fetched block: {0}")]
    RLPDecodeError(#[from] ethrex_rlp::error::RLPDecodeError),
    #[error("Block Fetcher failed in a helper function: {0}")]
    UtilsError(#[from] crate::utils::error::UtilsError),
    #[error("Missing bytes from calldata: {0}")]
    WrongBatchCalldata(String),
    #[error("Failed due to an EVM error: {0}")]
    EvmError(#[from] ethrex_vm::EvmError),
    #[error("Failed to produce the blob bundle")]
    BlobBundleError,
}

pub struct BlockFetcher {
    eth_client: EthClient,
    on_chain_proposer_address: Address,
    store: Store,
    rollup_store: StoreRollup,
    blockchain: Arc<Blockchain>,
    sequencer_state: Arc<Mutex<SequencerState>>,
    fetch_interval_ms: u64,
    last_l1_block_fetched: U256,
    fetch_block_step: U256,
}

pub async fn start_block_fetcher(
    store: Store,
    blockchain: Arc<Blockchain>,
    sequencer_state: Arc<Mutex<SequencerState>>,
    rollup_store: StoreRollup,
    cfg: SequencerConfig,
) -> Result<(), SequencerError> {
    let mut block_fetcher = BlockFetcher::new(
        &cfg,
        store.clone(),
        rollup_store,
        blockchain,
        sequencer_state,
    )
    .await?;
    block_fetcher.run().await;
    Ok(())
}

impl BlockFetcher {
    pub async fn new(
        cfg: &SequencerConfig,
        store: Store,
        rollup_store: StoreRollup,
        blockchain: Arc<Blockchain>,
        sequencer_state: Arc<Mutex<SequencerState>>,
    ) -> Result<Self, BlockFetcherError> {
        let eth_client = EthClient::new_with_multiple_urls(cfg.eth.rpc_url.clone())?;
        let last_l1_block_fetched = eth_client
            .get_last_fetched_l1_block(cfg.l1_watcher.bridge_address)
            .await?
            .into();
        Ok(Self {
            eth_client: EthClient::new_with_multiple_urls(cfg.eth.rpc_url.clone())?,
            on_chain_proposer_address: cfg.l1_committer.on_chain_proposer_address,
            store,
            rollup_store,
            blockchain,
            sequencer_state,
            fetch_interval_ms: cfg.based.block_fetcher.fetch_interval_ms,
            last_l1_block_fetched,
            fetch_block_step: cfg.based.block_fetcher.fetch_block_step.into(),
        })
    }

    pub async fn run(&mut self) {
        loop {
            let _ = self.main_logic().await.inspect_err(|err| {
                error!("Block Fetcher Error: {err}");
            });

            sleep(Duration::from_millis(self.fetch_interval_ms)).await;
        }
    }

    pub async fn main_logic(&mut self) -> Result<(), BlockFetcherError> {
        if let SequencerState::Sequencing = *self.sequencer_state.clone().lock().await {
            return Ok(());
        }
        self.fetch().await
    }

    async fn fetch(&mut self) -> Result<(), BlockFetcherError> {
        while !self.node_is_up_to_date().await? {
            info!("Node is not up to date. Syncing via L1");

            let last_l2_block_number_known = self.store.get_latest_block_number().await?;

            let last_l2_batch_number_known = self
                .rollup_store
                .get_batch_number_by_block(last_l2_block_number_known)
                .await?
                .ok_or(BlockFetcherError::InternalError(format!(
                    "Failed to get last batch number known for block {last_l2_block_number_known}"
                )))?;

            let last_l2_committed_batch_number = self
                .eth_client
                .get_last_committed_batch(self.on_chain_proposer_address)
                .await?;

            let l2_batches_behind = last_l2_committed_batch_number.checked_sub(last_l2_batch_number_known).ok_or(
                BlockFetcherError::InternalError(
                    "Failed to calculate batches behind. Last batch number known is greater than last committed batch number.".to_string(),
                ),
            )?;

            info!("Node is {l2_batches_behind} batches behind. Last batch number known: {last_l2_batch_number_known}, last committed batch number: {last_l2_committed_batch_number}");

            let batch_committed_logs = self.get_logs().await?;

            let mut missing_batches_logs = self
                .filter_logs(&batch_committed_logs, last_l2_batch_number_known)
                .await?;

            missing_batches_logs.sort_by_key(|(_log, batch_number)| *batch_number);

            for (batch_committed_log, batch_number) in missing_batches_logs {
                let batch_commit_tx_calldata = self
                    .eth_client
                    .get_transaction_by_hash(batch_committed_log.transaction_hash)
                    .await?
                    .ok_or(BlockFetcherError::InternalError(format!(
                        "Failed to get the receipt for transaction {:x}",
                        batch_committed_log.transaction_hash
                    )))?
                    .data;

                let batch = Self::decode_batch_from_calldata(&batch_commit_tx_calldata)?;

                self.store_batch(&batch).await?;

                self.seal_batch(&batch, batch_number).await?;
            }

            sleep(Duration::from_millis(self.fetch_interval_ms)).await;
        }

        info!("Node is up to date");

        Ok(())
    }

    async fn node_is_up_to_date(&self) -> Result<bool, BlockFetcherError> {
        let last_committed_batch_number = self
            .eth_client
            .get_last_committed_batch(self.on_chain_proposer_address)
            .await?;

        self.rollup_store
            .contains_batch(&last_committed_batch_number)
            .await
            .map_err(BlockFetcherError::StoreError)
    }

    /// Fetch logs from the L1 chain for the BatchCommitted event.
    /// This function fetches logs, starting from the last fetched block number (aka the last block that was processed)
    /// and going up to the current block number.
    async fn get_logs(&mut self) -> Result<Vec<RpcLog>, BlockFetcherError> {
        let last_l1_block_number = self.eth_client.get_block_number().await?;

        let mut batch_committed_logs = Vec::new();
        while self.last_l1_block_fetched < last_l1_block_number {
            let new_last_l1_fetched_block = min(
                self.last_l1_block_fetched + self.fetch_block_step,
                last_l1_block_number,
            );

            debug!(
                "Fetching logs from block {} to {}",
                self.last_l1_block_fetched + 1,
                new_last_l1_fetched_block
            );

            // Fetch logs from the L1 chain for the BatchCommitted event.
            let logs = self
                .eth_client
                .get_logs(
                    self.last_l1_block_fetched + 1,
                    new_last_l1_fetched_block,
                    self.on_chain_proposer_address,
                    keccak(b"BatchCommitted(uint256,bytes32)"),
                )
                .await?;

            // Update the last L1 block fetched.
            self.last_l1_block_fetched = new_last_l1_fetched_block;

            batch_committed_logs.extend_from_slice(&logs);
        }

        Ok(batch_committed_logs)
    }

    /// Given the logs from the event `BatchCommitted`,
    /// this function gets the committed batches that are missing in the local store.
    /// It does that by comparing if the batch number is greater than the last known batch number.
    async fn filter_logs(
        &self,
        logs: &[RpcLog],
        last_batch_number_known: u64,
    ) -> Result<Vec<(RpcLog, U256)>, BlockFetcherError> {
        let mut filtered_logs = Vec::new();

        // Filter missing batches logs
        for batch_committed_log in logs.iter().cloned() {
            let committed_batch_number = U256::from_big_endian(
                batch_committed_log
                    .log
                    .topics
                    .get(1)
                    .ok_or(BlockFetcherError::InternalError(
                        "Failed to get committed batch number from BatchCommitted log".to_string(),
                    ))?
                    .as_bytes(),
            );

            if committed_batch_number > last_batch_number_known.into() {
                filtered_logs.push((batch_committed_log, committed_batch_number));
            }
        }

        Ok(filtered_logs)
    }

    // TODO: Move to calldata module (SDK)
    fn decode_batch_from_calldata(calldata: &[u8]) -> Result<Vec<Block>, BlockFetcherError> {
        // function commitBatch(
        //     uint256 batchNumber,
        //     bytes32 newStateRoot,
        //     bytes32 stateDiffKZGVersionedHash,
        //     bytes32 withdrawalsLogsMerkleRoot,
        //     bytes32 processedDepositLogsRollingHash,
        //     bytes[] calldata _hexEncodedBlocks
        // ) external;

        // data =   4 bytes (function selector) 0..4
        //          || 8 bytes (batch number)   4..36
        //          || 32 bytes (new state root) 36..68
        //          || 32 bytes (state diff KZG versioned hash) 68..100
        //          || 32 bytes (withdrawals logs merkle root) 100..132
        //          || 32 bytes (processed deposit logs rolling hash) 132..164

        let batch_length_in_blocks = U256::from_big_endian(calldata.get(196..228).ok_or(
            BlockFetcherError::WrongBatchCalldata("Couldn't get batch length bytes".to_owned()),
        )?)
        .as_usize();

        let base = 228;

        let mut batch = Vec::new();

        for block_i in 0..batch_length_in_blocks {
            let block_length_offset = base + block_i * 32;

            let dynamic_offset = U256::from_big_endian(
                calldata
                    .get(block_length_offset..block_length_offset + 32)
                    .ok_or(BlockFetcherError::WrongBatchCalldata(
                        "Couldn't get dynamic offset bytes".to_owned(),
                    ))?,
            )
            .as_usize();

            let block_length_in_bytes = U256::from_big_endian(
                calldata
                    .get(base + dynamic_offset..base + dynamic_offset + 32)
                    .ok_or(BlockFetcherError::WrongBatchCalldata(
                        "Couldn't get block length bytes".to_owned(),
                    ))?,
            )
            .as_usize();

            let block_offset = base + dynamic_offset + 32;

            let block = Block::decode(
                calldata
                    .get(block_offset..block_offset + block_length_in_bytes)
                    .ok_or(BlockFetcherError::WrongBatchCalldata(
                        "Couldn't get block bytes".to_owned(),
                    ))?,
            )?;

            batch.push(block);
        }

        Ok(batch)
    }

    async fn store_batch(&self, batch: &[Block]) -> Result<(), BlockFetcherError> {
        for block in batch.iter() {
            self.blockchain.add_block(block).await?;

            let block_hash = block.hash();

            apply_fork_choice(&self.store, block_hash, block_hash, block_hash).await?;

            info!(
                "Added fetched block {} with hash {block_hash:#x}",
                block.header.number,
            );
        }

        Ok(())
    }

    async fn seal_batch(
        &self,
        batch: &[Block],
        batch_number: U256,
    ) -> Result<(), BlockFetcherError> {
        let batch = self.get_batch(batch, batch_number).await?;

        self.rollup_store.seal_batch(batch).await?;

        info!("Sealed batch {batch_number}.");

        Ok(())
    }

    async fn get_batch_withdrawal_hashes(
        &self,
        batch: &[Block],
    ) -> Result<Vec<H256>, BlockFetcherError> {
        let mut withdrawal_hashes = Vec::new();

        for block in batch {
            let block_withdrawals = self.get_block_withdrawals(block.header.number).await?;

            for tx in &block_withdrawals {
                let hash = get_withdrawal_hash(tx).ok_or(BlockFetcherError::InternalError(
                    "Invalid withdraw transaction".to_owned(),
                ))?;
                withdrawal_hashes.push(hash);
            }
        }

        Ok(withdrawal_hashes)
    }

    async fn get_block_withdrawals(
        &self,
        block_number: BlockNumber,
    ) -> Result<Vec<Transaction>, BlockFetcherError> {
        let Some(block_body) = self.store.get_block_body(block_number).await? else {
            return Err(BlockFetcherError::InternalError(format!(
                "Block {block_number} is supposed to be in store at this point"
            )));
        };

        let mut txs_and_receipts = vec![];
        for (index, tx) in block_body.transactions.iter().enumerate() {
            let receipt = self
                .store
                .get_receipt(
                    block_number,
                    index.try_into().map_err(|_| {
                        BlockFetcherError::InternalError(
                            "Failed to convert index to u64".to_owned(),
                        )
                    })?,
                )
                .await?
                .ok_or(BlockFetcherError::InternalError(
                    "Transactions in a block should have a receipt".to_owned(),
                ))?;
            txs_and_receipts.push((tx.clone(), receipt));
        }

        let mut ret = vec![];

        for (tx, receipt) in txs_and_receipts {
            if is_withdrawal_l2(&tx, &receipt) {
                ret.push(tx.clone())
            }
        }
        Ok(ret)
    }

    async fn get_batch(
        &self,
        batch: &[Block],
        batch_number: U256,
    ) -> Result<Batch, BlockFetcherError> {
        let deposits: Vec<PrivilegedL2Transaction> = batch
            .iter()
            .flat_map(|block| {
                block.body.transactions.iter().filter_map(|tx| {
                    if let Transaction::PrivilegedL2Transaction(tx) = tx {
                        Some(tx.clone())
                        // tx.get_deposit_hash()
                    } else {
                        None
                    }
                })
            })
            .collect();
        let deposit_hashes = deposits
            .iter()
            .filter_map(|tx| tx.get_deposit_hash())
            .collect();
        let mut withdrawals = Vec::new();
        for block in batch {
            let block_withdrawals = self.get_block_withdrawals(block.header.number).await?;
            withdrawals.extend(block_withdrawals);
        }
        let deposit_logs_hash = get_deposit_logs_hash(deposit_hashes)?;

        let first_block = batch.first().ok_or(BlockFetcherError::InternalError(
            "Batch is empty. This shouldn't happen.".to_owned(),
        ))?;

        let last_block = batch.last().ok_or(BlockFetcherError::InternalError(
            "Batch is empty. This shouldn't happen.".to_owned(),
        ))?;

        let new_state_root = self
            .store
            .state_trie(last_block.hash())?
            .ok_or(BlockFetcherError::InternalError(
                "This block should be in the store".to_owned(),
            ))?
            .hash_no_commit();

        // This is copied from the L1Committer, this should be reviewed.
        let mut acc_account_updates: HashMap<H160, AccountUpdate> = HashMap::new();
        for block in batch {
            let vm_db = StoreVmDatabase::new(self.store.clone(), block.header.parent_hash);
            let mut vm = Evm::new(EvmEngine::default(), vm_db);
            vm.execute_block(block)
                .map_err(BlockFetcherError::EvmError)?;
            let account_updates = vm
                .get_state_transitions()
                .map_err(BlockFetcherError::EvmError)?;

            for account in account_updates {
                let address = account.address;
                if let Some(existing) = acc_account_updates.get_mut(&address) {
                    existing.merge(account);
                } else {
                    acc_account_updates.insert(address, account);
                }
            }
        }

        let parent_block_hash = first_block.header.parent_hash;

        let parent_db = StoreVmDatabase::new(self.store.clone(), parent_block_hash);

        let state_diff = prepare_state_diff(
            last_block.header.clone(),
            &parent_db,
            &withdrawals,
            &deposits,
            acc_account_updates.into_values().collect(),
        )
        .map_err(|_| BlockFetcherError::BlobBundleError)?;

        let (blobs_bundle, _) =
            generate_blobs_bundle(&state_diff).map_err(|_| BlockFetcherError::BlobBundleError)?;

        Ok(Batch {
            number: batch_number.as_u64(),
            first_block: first_block.header.number,
            last_block: last_block.header.number,
            state_root: new_state_root,
            deposit_logs_hash,
            withdrawal_hashes: self.get_batch_withdrawal_hashes(batch).await?,
            blobs_bundle,
        })
    }
}

fn get_deposit_logs_hash(deposit_hashes: Vec<H256>) -> Result<H256, BlockFetcherError> {
    if deposit_hashes.is_empty() {
        return Ok(H256::zero());
    }
    let deposit_hashes_len: u16 = deposit_hashes.len().try_into().map_err(|e| {
        BlockFetcherError::InternalError(format!("Failed to convert usize to u16: {e}"))
    })?;
    Ok(H256::from_slice(
        [
            &deposit_hashes_len.to_be_bytes(),
            keccak(
                deposit_hashes
                    .iter()
                    .map(H256::as_bytes)
                    .collect::<Vec<&[u8]>>()
                    .concat(),
            )
            .as_bytes()
            .get(2..32)
            .ok_or(BlockFetcherError::WrongBatchCalldata(
                "Failed to decode deposit hashes".to_string(),
            ))?,
        ]
        .concat()
        .as_slice(),
    ))
}
