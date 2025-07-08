pub mod constants;
pub mod error;
pub mod fork_choice;
pub mod mempool;
pub mod payload;
mod smoke_test;
pub mod tracing;
pub mod vm;

use ::tracing::{info, warn};
use constants::{MAX_INITCODE_SIZE, MAX_TRANSACTION_DATA_SIZE};
use error::MempoolError;
use error::{ChainError, InvalidBlockError};
use ethrex_common::constants::{GAS_PER_BLOB, MIN_BASE_FEE_PER_BLOB_GAS};
use ethrex_common::types::MempoolTransaction;
use ethrex_common::types::block_execution_witness::ExecutionWitnessResult;
use ethrex_common::types::requests::{EncodedRequests, Requests, compute_requests_hash};
use ethrex_common::types::{
    AccountUpdate, Block, BlockHash, BlockHeader, BlockNumber, ChainConfig, EIP4844Transaction,
    Receipt, Transaction, WrappedEIP4844Transaction, compute_receipts_root, validate_block_header,
    validate_cancun_header_fields, validate_prague_header_fields,
    validate_pre_cancun_header_fields,
};
use ethrex_common::types::{ELASTICITY_MULTIPLIER, P2PTransaction};
use ethrex_common::{Address, H256, TrieLogger};
use ethrex_metrics::metrics;
use ethrex_storage::{Store, UpdateBatch, error::StoreError, hash_address, hash_key};
use ethrex_vm::backends::levm::db::DatabaseLogger;
use ethrex_vm::{BlockExecutionResult, DynVmDatabase, Evm, EvmEngine, EvmError};
use mempool::Mempool;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tokio_util::sync::CancellationToken;

use vm::StoreVmDatabase;

#[cfg(feature = "metrics")]
use ethrex_metrics::metrics_blocks::METRICS_BLOCKS;

#[cfg(feature = "c-kzg")]
use ethrex_common::types::BlobsBundle;

//TODO: Implement a struct Chain or BlockChain to encapsulate
//functionality and canonical chain state and config

#[derive(Debug, Clone, Default)]
pub enum BlockchainType {
    #[default]
    L1,
    L2,
}

#[derive(Debug)]
pub struct Blockchain {
    pub evm_engine: EvmEngine,
    storage: Store,
    pub mempool: Mempool,
    /// Whether the node's chain is in or out of sync with the current chain
    /// This will be set to true once the initial sync has taken place and wont be set to false after
    /// This does not reflect whether there is an ongoing sync process
    is_synced: AtomicBool,
    pub r#type: BlockchainType,
}

#[derive(Debug, Clone)]
pub struct BatchBlockProcessingFailure {
    pub last_valid_hash: H256,
    pub failed_block_hash: H256,
}

fn log_batch_progress(batch_size: usize, current_block: usize) {
    let progress_needed = batch_size > 10;
    const PERCENT_MARKS: [usize; 4] = [20, 40, 60, 80];
    if progress_needed {
        PERCENT_MARKS.iter().for_each(|mark| {
            if (batch_size * mark) / 100 == current_block {
                info!("[SYNCING] {mark}% of batch processed");
            }
        });
    }
}

impl Blockchain {
    pub fn new(evm_engine: EvmEngine, store: Store, blockchain_type: BlockchainType) -> Self {
        Self {
            evm_engine,
            storage: store,
            mempool: Mempool::new(),
            is_synced: AtomicBool::new(false),
            r#type: blockchain_type,
        }
    }

    pub fn default_with_store(store: Store) -> Self {
        Self {
            evm_engine: EvmEngine::default(),
            storage: store,
            mempool: Mempool::new(),
            is_synced: AtomicBool::new(false),
            r#type: BlockchainType::default(),
        }
    }

    /// Executes a block withing a new vm instance and state
    async fn execute_block(
        &self,
        block: &Block,
    ) -> Result<(BlockExecutionResult, Vec<AccountUpdate>), ChainError> {
        // Validate if it can be the new head and find the parent
        let Ok(parent_header) = find_parent_header(&block.header, &self.storage) else {
            // If the parent is not present, we store it as pending.
            self.storage.add_pending_block(block.clone()).await?;
            return Err(ChainError::ParentNotFound);
        };

        let chain_config = self.storage.get_chain_config()?;

        // Validate the block pre-execution
        validate_block(block, &parent_header, &chain_config, ELASTICITY_MULTIPLIER)?;

        let vm_db = StoreVmDatabase::new(self.storage.clone(), block.header.parent_hash);
        let mut vm = self.new_evm(vm_db)?;
        let execution_result = vm.execute_block(block)?;
        let account_updates = vm.get_state_transitions()?;

        // Validate execution went alright
        validate_gas_used(&execution_result.receipts, &block.header)?;
        validate_receipts_root(&block.header, &execution_result.receipts)?;
        validate_requests_hash(&block.header, &chain_config, &execution_result.requests)?;

        Ok((execution_result, account_updates))
    }

    /// Executes a block from a given vm instance an does not clear its state
    fn execute_block_from_state(
        &self,
        parent_header: &BlockHeader,
        block: &Block,
        chain_config: &ChainConfig,
        vm: &mut Evm,
    ) -> Result<BlockExecutionResult, ChainError> {
        // Validate the block pre-execution
        validate_block(block, parent_header, chain_config, ELASTICITY_MULTIPLIER)?;

        let execution_result = vm.execute_block(block)?;

        // Validate execution went alright
        validate_gas_used(&execution_result.receipts, &block.header)?;
        validate_receipts_root(&block.header, &execution_result.receipts)?;
        validate_requests_hash(&block.header, chain_config, &execution_result.requests)?;

        Ok(execution_result)
    }

    pub async fn generate_witness_for_blocks(
        &self,
        blocks: &[Block],
    ) -> Result<ExecutionWitnessResult, ChainError> {
        let first_block_header = blocks
            .first()
            .ok_or(ChainError::WitnessGeneration(
                "Empty block batch".to_string(),
            ))?
            .header
            .clone();

        let parent_block_header = self
            .storage
            .get_block_header_by_hash(first_block_header.parent_hash)?
            .ok_or(ChainError::ParentNotFound)?;

        // Get state at previous block
        let trie = self
            .storage
            .state_trie(first_block_header.parent_hash)
            .map_err(|_| ChainError::ParentStateNotFound)?
            .ok_or(ChainError::ParentStateNotFound)?;
        let (state_trie_witness, mut trie) = TrieLogger::open_trie(trie);

        // Store the root node in case the block is empty and the witness does not record any nodes
        let root_node = trie.root_node().map_err(|_| {
            ChainError::WitnessGeneration("Failed to get root state node".to_string())
        })?;

        let mut encoded_storage_tries: HashMap<ethrex_common::H160, Vec<Vec<u8>>> = HashMap::new();
        let mut block_hashes = HashMap::new();
        let mut codes = HashMap::new();

        for block in blocks {
            let parent_hash = block.header.parent_hash;
            let vm_db: DynVmDatabase =
                Box::new(StoreVmDatabase::new(self.storage.clone(), parent_hash));
            let logger = Arc::new(DatabaseLogger::new(Arc::new(Mutex::new(Box::new(vm_db)))));
            let mut vm = match self.r#type {
                BlockchainType::L1 => Evm::new_from_db_for_l1(logger.clone()),
                BlockchainType::L2 => Evm::new_from_db_for_l2(logger.clone()),
            };

            // Re-execute block with logger
            vm.execute_block(block)?;
            // Gather account updates
            let account_updates = vm.get_state_transitions()?;

            let mut used_storage_tries = HashMap::new();
            // Get the used block hashes from the logger
            let logger_block_hashes = logger
                .block_hashes_accessed
                .lock()
                .map_err(|_e| {
                    ChainError::WitnessGeneration("Failed to get block hashes".to_string())
                })?
                .clone();
            block_hashes.extend(logger_block_hashes);
            // Access all the accounts needed for withdrawals
            if let Some(withdrawals) = block.body.withdrawals.as_ref() {
                for withdrawal in withdrawals {
                    trie.get(&hash_address(&withdrawal.address)).map_err(|_e| {
                        ChainError::Custom("Failed to access account from trie".to_string())
                    })?;
                }
            }

            // Access all the accounts from the initial trie
            // Record all the storage nodes for the initial state
            for (account, keys) in logger
                .state_accessed
                .lock()
                .map_err(|_e| {
                    ChainError::WitnessGeneration("Failed to execute with witness".to_string())
                })?
                .iter()
            {
                // Access the account from the state trie to record the nodes used to access it
                trie.get(&hash_address(account)).map_err(|_e| {
                    ChainError::WitnessGeneration("Failed to access account from trie".to_string())
                })?;
                // Get storage trie at before updates
                if !keys.is_empty() {
                    if let Ok(Some(storage_trie)) = self.storage.storage_trie(parent_hash, *account)
                    {
                        let (storage_trie_witness, storage_trie) =
                            TrieLogger::open_trie(storage_trie);
                        // Access all the keys
                        for storage_key in keys {
                            let hashed_key = hash_key(storage_key);
                            storage_trie.get(&hashed_key).map_err(|_e| {
                                ChainError::WitnessGeneration(
                                    "Failed to access storage key".to_string(),
                                )
                            })?;
                        }
                        // Store the tries to reuse when applying account updates
                        used_storage_tries.insert(*account, (storage_trie_witness, storage_trie));
                    }
                }
            }
            // Store all the accessed evm bytecodes
            for code_hash in logger
                .code_accessed
                .lock()
                .map_err(|_e| {
                    ChainError::WitnessGeneration("Failed to gather used bytecodes".to_string())
                })?
                .iter()
            {
                let code = self
                    .storage
                    .get_account_code(*code_hash)
                    .map_err(|_e| {
                        ChainError::WitnessGeneration("Failed to get account code".to_string())
                    })?
                    .ok_or(ChainError::WitnessGeneration(
                        "Failed to get account code".to_string(),
                    ))?;
                codes.insert(*code_hash, code);
            }

            // Apply account updates to the trie recording all the necessary nodes to do so
            let (updated_trie, storage_tries_after_update) = self
                .storage
                .apply_account_updates_from_trie_with_witness(
                    trie,
                    &account_updates,
                    used_storage_tries,
                )
                .await?;
            for (address, (witness, _storage_trie)) in storage_tries_after_update {
                let mut witness = witness.lock().map_err(|_| {
                    ChainError::WitnessGeneration("Failed to lock storage trie witness".to_string())
                })?;
                let witness = std::mem::take(&mut *witness);
                let witness = witness.into_iter().collect::<Vec<_>>();
                match encoded_storage_tries.entry(address) {
                    std::collections::hash_map::Entry::Occupied(mut entry) => {
                        entry.get_mut().extend(witness);
                    }
                    std::collections::hash_map::Entry::Vacant(vacant) => {
                        vacant.insert(witness);
                    }
                }
            }
            trie = updated_trie;
        }

        // Get the witness for the state trie
        let mut state_trie_witness = state_trie_witness.lock().map_err(|_| {
            ChainError::WitnessGeneration("Failed to lock state trie witness".to_string())
        })?;
        let state_trie_witness = std::mem::take(&mut *state_trie_witness);
        let mut used_trie_nodes = Vec::from_iter(state_trie_witness.into_iter());
        // If the witness is empty at least try to store the root
        if used_trie_nodes.is_empty() {
            if let Some(root) = root_node {
                used_trie_nodes.push(root.encode_raw());
            }
        }

        let mut needed_block_numbers = block_hashes.keys().collect::<Vec<_>>();
        needed_block_numbers.sort();
        // The last block number we need is the parent of the last block we execute
        let last_needed_block_number = blocks
            .last()
            .ok_or(ChainError::WitnessGeneration("Empty batch".to_string()))?
            .header
            .number
            .saturating_sub(1);
        // The first block number we need is either the parent of the first block number or the earliest block number used by BLOCKHASH
        let mut first_needed_block_number = first_block_header.number.saturating_sub(1);
        if let Some(block_number_from_logger) = needed_block_numbers.first() {
            if **block_number_from_logger < first_needed_block_number {
                first_needed_block_number = **block_number_from_logger;
            }
        }
        let mut block_headers = HashMap::new();
        for block_number in first_needed_block_number..=last_needed_block_number {
            let header = self.storage.get_block_header(block_number)?.ok_or(
                ChainError::WitnessGeneration("Failed to get block header".to_string()),
            )?;
            block_headers.insert(block_number, header);
        }

        let chain_config = self.storage.get_chain_config().map_err(ChainError::from)?;

        Ok(ExecutionWitnessResult {
            state_trie_nodes: Some(used_trie_nodes),
            storage_trie_nodes: Some(encoded_storage_tries),
            codes,
            state_trie: None,
            storage_tries: None,
            block_headers,
            parent_block_header,
            chain_config,
        })
    }

    pub async fn store_block(
        &self,
        block: &Block,
        execution_result: BlockExecutionResult,
        account_updates: &[AccountUpdate],
    ) -> Result<(), ChainError> {
        // Apply the account updates over the last block's state and compute the new state root
        let apply_updates_list = self
            .storage
            .apply_account_updates_batch(block.header.parent_hash, account_updates)
            .await?
            .ok_or(ChainError::ParentStateNotFound)?;

        let new_state_root = apply_updates_list.state_trie_hash;
        let state_updates = apply_updates_list.state_updates;
        let accounts_updates = apply_updates_list.storage_updates;
        let code_updates = apply_updates_list.code_updates;

        // Check state root matches the one in block header
        validate_state_root(&block.header, new_state_root)?;

        let update_batch = UpdateBatch {
            account_updates: state_updates,
            storage_updates: accounts_updates,
            blocks: vec![block.clone()],
            receipts: vec![(block.hash(), execution_result.receipts)],
            code_updates,
        };

        self.storage
            .clone()
            .store_block_updates(update_batch)
            .await
            .map_err(|e| e.into())
    }

    pub async fn add_block(&self, block: &Block) -> Result<(), ChainError> {
        let since = Instant::now();
        let (res, updates) = self.execute_block(block).await?;
        let executed = Instant::now();
        let result = self.store_block(block, res, &updates).await;
        let stored = Instant::now();
        Self::print_add_block_logs(block, since, executed, stored);
        result
    }

    fn print_add_block_logs(block: &Block, since: Instant, executed: Instant, stored: Instant) {
        let interval = stored.duration_since(since).as_millis() as f64;
        if interval != 0f64 {
            let as_gigas = block.header.gas_used as f64 / 10_f64.powf(9_f64);
            let throughput = as_gigas / interval * 1000_f64;
            let execution_time = executed.duration_since(since).as_millis() as f64;
            let storage_time = stored.duration_since(executed).as_millis() as f64;
            let execution_fraction = (execution_time * 100_f64 / interval).round() as u64;
            let storage_fraction = (storage_time * 100_f64 / interval).round() as u64;
            let execution_time_per_gigagas = (execution_time / as_gigas).round() as u64;
            let storage_time_per_gigagas = (storage_time / as_gigas).round() as u64;
            metrics!(
                let _ = METRICS_BLOCKS.set_block_number(block.header.number);
                METRICS_BLOCKS.set_latest_gas_used(block.header.gas_used as f64);
                METRICS_BLOCKS.set_latest_block_gas_limit(block.header.gas_limit as f64);
                METRICS_BLOCKS.set_latest_gigagas(throughput);
            );
            let base_log = format!(
                "[METRIC] BLOCK EXECUTION THROUGHPUT ({}): {:.2} Ggas/s TIME SPENT: {:.0} ms. Gas Used: {:.0}%, #Txs: {}.",
                block.header.number,
                throughput,
                interval,
                (block.header.gas_used as f64 / block.header.gas_limit as f64) * 100.0,
                block.body.transactions.len()
            );
            let extra_log = if as_gigas > 0.0 {
                format!(
                    " exec/Ggas: {execution_time_per_gigagas} ms ({execution_fraction}%), st/Ggas: {storage_time_per_gigagas} ms ({storage_fraction}%)",
                )
            } else {
                "".to_string()
            };
            info!("{}{}", base_log, extra_log);
        }
    }

    /// Adds multiple blocks in a batch.
    ///
    /// If an error occurs, returns a tuple containing:
    /// - The error type ([`ChainError`]).
    /// - [`BatchProcessingFailure`] (if the error was caused by block processing).
    ///
    /// Note: only the last block's state trie is stored in the db
    pub async fn add_blocks_in_batch(
        &self,
        blocks: Vec<Block>,
        cancellation_token: CancellationToken,
    ) -> Result<(), (ChainError, Option<BatchBlockProcessingFailure>)> {
        let mut last_valid_hash = H256::default();

        let Some(first_block_header) = blocks.first().map(|e| e.header.clone()) else {
            return Err((ChainError::Custom("First block not found".into()), None));
        };

        let chain_config: ChainConfig = self
            .storage
            .get_chain_config()
            .map_err(|e| (e.into(), None))?;

        // Cache block hashes for the full batch so we can access them during execution without having to store the blocks beforehand
        let block_hash_cache = blocks.iter().map(|b| (b.header.number, b.hash())).collect();

        let vm_db = StoreVmDatabase::new_with_block_hash_cache(
            self.storage.clone(),
            first_block_header.parent_hash,
            block_hash_cache,
        );
        let mut vm = self.new_evm(vm_db).map_err(|e| (e.into(), None))?;

        let blocks_len = blocks.len();
        let mut all_receipts: Vec<(BlockHash, Vec<Receipt>)> = Vec::with_capacity(blocks_len);
        let mut total_gas_used = 0;
        let mut transactions_count = 0;

        let interval = Instant::now();
        for (i, block) in blocks.iter().enumerate() {
            if cancellation_token.is_cancelled() {
                info!("Received shutdown signal, aborting");
                return Err((ChainError::Custom(String::from("shutdown signal")), None));
            }
            // for the first block, we need to query the store
            let parent_header = if i == 0 {
                find_parent_header(&block.header, &self.storage).map_err(|err| {
                    (
                        err,
                        Some(BatchBlockProcessingFailure {
                            failed_block_hash: block.hash(),
                            last_valid_hash,
                        }),
                    )
                })?
            } else {
                // for the subsequent ones, the parent is the previous block
                blocks[i - 1].header.clone()
            };

            let BlockExecutionResult { receipts, .. } = self
                .execute_block_from_state(&parent_header, block, &chain_config, &mut vm)
                .map_err(|err| {
                    warn!("Failed to execute block: {}", block.header.number);
                    (
                        err,
                        Some(BatchBlockProcessingFailure {
                            failed_block_hash: block.hash(),
                            last_valid_hash,
                        }),
                    )
                })?;

            last_valid_hash = block.hash();
            total_gas_used += block.header.gas_used;
            transactions_count += block.body.transactions.len();
            all_receipts.push((block.hash(), receipts));

            log_batch_progress(blocks_len, i);
        }

        let account_updates = vm
            .get_state_transitions()
            .map_err(|err| (ChainError::EvmError(err), None))?;

        let last_block = blocks
            .last()
            .ok_or_else(|| (ChainError::Custom("Last block not found".into()), None))?;

        let last_block_number = last_block.header.number;
        let last_block_gas_limit = last_block.header.gas_limit;

        // Apply the account updates over all blocks and compute the new state root
        let account_updates_list = self
            .storage
            .apply_account_updates_batch(first_block_header.parent_hash, &account_updates)
            .await
            .map_err(|e| (e.into(), None))?
            .ok_or((ChainError::ParentStateNotFound, None))?;

        let new_state_root = account_updates_list.state_trie_hash;
        let state_updates = account_updates_list.state_updates;
        let accounts_updates = account_updates_list.storage_updates;
        let code_updates = account_updates_list.code_updates;

        // Check state root matches the one in block header
        validate_state_root(&last_block.header, new_state_root).map_err(|e| (e, None))?;

        let update_batch = UpdateBatch {
            account_updates: state_updates,
            storage_updates: accounts_updates,
            blocks,
            receipts: all_receipts,
            code_updates,
        };

        self.storage
            .store_block_updates(update_batch)
            .await
            .map_err(|e| (e.into(), None))?;

        let elapsed_seconds = interval.elapsed().as_secs_f64();
        let mut throughput = 0.0;
        if elapsed_seconds > 0.0 && total_gas_used != 0 {
            let as_gigas = (total_gas_used as f64) / 1e9;
            throughput = as_gigas / elapsed_seconds;
        }

        metrics!(
            let _ = METRICS_BLOCKS.set_block_number(last_block_number);
            METRICS_BLOCKS.set_latest_block_gas_limit(last_block_gas_limit as f64);
            // Set the latest gas used as the average gas used per block in the batch
            METRICS_BLOCKS.set_latest_gas_used(total_gas_used as f64 / blocks_len as f64);
            METRICS_BLOCKS.set_latest_gigagas(throughput);
        );

        info!(
            "[METRICS] Executed and stored: Range: {}, Last block num: {}, Last block gas limit: {}, Total transactions: {}, Total Gas: {}, Throughput: {} Gigagas/s",
            blocks_len,
            last_block_number,
            last_block_gas_limit,
            transactions_count,
            total_gas_used,
            throughput
        );

        Ok(())
    }

    /// Add a blob transaction and its blobs bundle to the mempool checking that the transaction is valid
    #[cfg(feature = "c-kzg")]
    pub async fn add_blob_transaction_to_pool(
        &self,
        transaction: EIP4844Transaction,
        blobs_bundle: BlobsBundle,
    ) -> Result<H256, MempoolError> {
        // Validate blobs bundle

        blobs_bundle.validate(&transaction)?;

        let transaction = Transaction::EIP4844Transaction(transaction);
        let sender = transaction.sender()?;

        // Validate transaction
        if let Some(tx_to_replace) = self.validate_transaction(&transaction, sender).await? {
            self.remove_transaction_from_pool(&tx_to_replace)?;
        }

        // Add transaction and blobs bundle to storage
        let hash = transaction.compute_hash();
        self.mempool
            .add_transaction(hash, MempoolTransaction::new(transaction, sender))?;
        self.mempool.add_blobs_bundle(hash, blobs_bundle)?;
        Ok(hash)
    }

    /// Add a transaction to the mempool checking that the transaction is valid
    pub async fn add_transaction_to_pool(
        &self,
        transaction: Transaction,
    ) -> Result<H256, MempoolError> {
        // Blob transactions should be submitted via add_blob_transaction along with the corresponding blobs bundle
        if matches!(transaction, Transaction::EIP4844Transaction(_)) {
            return Err(MempoolError::BlobTxNoBlobsBundle);
        }
        let sender = transaction.sender()?;
        // Validate transaction
        if let Some(tx_to_replace) = self.validate_transaction(&transaction, sender).await? {
            self.remove_transaction_from_pool(&tx_to_replace)?;
        }

        let hash = transaction.compute_hash();

        // Add transaction to storage
        self.mempool
            .add_transaction(hash, MempoolTransaction::new(transaction, sender))?;

        Ok(hash)
    }

    /// Remove a transaction from the mempool
    pub fn remove_transaction_from_pool(&self, hash: &H256) -> Result<(), StoreError> {
        self.mempool.remove_transaction(hash)
    }

    /*

    SOME VALIDATIONS THAT WE COULD INCLUDE
    Stateless validations
    1. This transaction is valid on current mempool
        -> Depends on mempool transaction filtering logic
    2. Ensure the maxPriorityFeePerGas is high enough to cover the requirement of the calling pool (the minimum to be included in)
        -> Depends on mempool transaction filtering logic
    3. Transaction's encoded size is smaller than maximum allowed
        -> I think that this is not in the spec, but it may be a good idea
    4. Make sure the transaction is signed properly
    5. Ensure a Blob Transaction comes with its sidecar (Done! - All blob validations have been moved to `common/types/blobs_bundle.rs`):
      1. Validate number of BlobHashes is positive (Done!)
      2. Validate number of BlobHashes is less than the maximum allowed per block,
         which may be computed as `maxBlobGasPerBlock / blobTxBlobGasPerBlob`
      3. Ensure number of BlobHashes is equal to:
        - The number of blobs (Done!)
        - The number of commitments (Done!)
        - The number of proofs (Done!)
      4. Validate that the hashes matches with the commitments, performing a `kzg4844` hash. (Done!)
      5. Verify the blob proofs with the `kzg4844` (Done!)
    Stateful validations
    1. Ensure transaction nonce is higher than the `from` address stored nonce
    2. Certain pools do not allow for nonce gaps. Ensure a gap is not produced (that is, the transaction nonce is exactly the following of the stored one)
    3. Ensure the transactor has enough funds to cover transaction cost:
        - Transaction cost is calculated as `(gas * gasPrice) + (blobGas * blobGasPrice) + value`
    4. In case of transaction reorg, ensure the transactor has enough funds to cover for transaction replacements without overdrafts.
    - This is done by comparing the total spent gas of the transactor from all pooled transactions, and accounting for the necessary gas spenditure if any of those transactions is replaced.
    5. Ensure the transactor is able to add a new transaction. The number of transactions sent by an account may be limited by a certain configured value

    */
    /// Returns the hash of the transaction to replace in case the nonce already exists
    pub async fn validate_transaction(
        &self,
        tx: &Transaction,
        sender: Address,
    ) -> Result<Option<H256>, MempoolError> {
        let nonce = tx.nonce();

        if matches!(tx, &Transaction::PrivilegedL2Transaction(_)) {
            return Ok(None);
        }

        let header_no = self.storage.get_latest_block_number().await?;
        let header = self
            .storage
            .get_block_header(header_no)?
            .ok_or(MempoolError::NoBlockHeaderError)?;
        let config = self.storage.get_chain_config()?;

        // NOTE: We could add a tx size limit here, but it's not in the actual spec

        // Check init code size
        if config.is_shanghai_activated(header.timestamp)
            && tx.is_contract_creation()
            && tx.data().len() > MAX_INITCODE_SIZE
        {
            return Err(MempoolError::TxMaxInitCodeSizeError);
        }

        if !tx.is_contract_creation() && tx.data().len() >= MAX_TRANSACTION_DATA_SIZE {
            return Err(MempoolError::TxMaxDataSizeError);
        }

        // Check gas limit is less than header's gas limit
        if header.gas_limit < tx.gas_limit() {
            return Err(MempoolError::TxGasLimitExceededError);
        }

        // Check priority fee is less or equal than gas fee gap
        if tx.max_priority_fee().unwrap_or(0) > tx.max_fee_per_gas().unwrap_or(0) {
            return Err(MempoolError::TxTipAboveFeeCapError);
        }

        // Check that the gas limit is covers the gas needs for transaction metadata.
        if tx.gas_limit() < mempool::transaction_intrinsic_gas(tx, &header, &config)? {
            return Err(MempoolError::TxIntrinsicGasCostAboveLimitError);
        }

        // Check that the specified blob gas fee is above the minimum value
        if let Some(fee) = tx.max_fee_per_blob_gas() {
            // Blob tx fee checks
            if fee < MIN_BASE_FEE_PER_BLOB_GAS.into() {
                return Err(MempoolError::TxBlobBaseFeeTooLowError);
            }
        };

        let maybe_sender_acc_info = self.storage.get_account_info(header_no, sender).await?;

        if let Some(sender_acc_info) = maybe_sender_acc_info {
            if nonce < sender_acc_info.nonce || nonce == u64::MAX {
                return Err(MempoolError::NonceTooLow);
            }

            let tx_cost = tx
                .cost_without_base_fee()
                .ok_or(MempoolError::InvalidTxGasvalues)?;

            if tx_cost > sender_acc_info.balance {
                return Err(MempoolError::NotEnoughBalance);
            }
        } else {
            // An account that is not in the database cannot possibly have enough balance to cover the transaction cost
            return Err(MempoolError::NotEnoughBalance);
        }

        // Check the nonce of pendings TXs in the mempool from the same sender
        // If it exists check if the new tx has higher fees
        let tx_to_replace_hash = self.mempool.find_tx_to_replace(sender, nonce, tx)?;

        if let Some(chain_id) = tx.chain_id() {
            if chain_id != config.chain_id {
                return Err(MempoolError::InvalidChainId(config.chain_id));
            }
        }

        Ok(tx_to_replace_hash)
    }

    /// Marks the node's chain as up to date with the current chain
    /// Once the initial sync has taken place, the node will be consireded as sync
    pub fn set_synced(&self) {
        self.is_synced.store(true, Ordering::Relaxed);
    }

    /// Returns whether the node's chain is up to date with the current chain
    /// This will be true if the initial sync has already taken place and does not reflect whether there is an ongoing sync process
    /// The node should accept incoming p2p transactions if this method returns true
    pub fn is_synced(&self) -> bool {
        self.is_synced.load(Ordering::Relaxed)
    }

    pub fn get_p2p_transaction_by_hash(&self, hash: &H256) -> Result<P2PTransaction, StoreError> {
        let Some(tx) = self.mempool.get_transaction_by_hash(*hash)? else {
            return Err(StoreError::Custom(format!(
                "Hash {hash} not found in the mempool",
            )));
        };
        let result = match tx {
            Transaction::LegacyTransaction(itx) => P2PTransaction::LegacyTransaction(itx),
            Transaction::EIP2930Transaction(itx) => P2PTransaction::EIP2930Transaction(itx),
            Transaction::EIP1559Transaction(itx) => P2PTransaction::EIP1559Transaction(itx),
            Transaction::EIP4844Transaction(itx) => {
                let Some(bundle) = self.mempool.get_blobs_bundle(*hash)? else {
                    return Err(StoreError::Custom(format!(
                        "Blob transaction present without its bundle: hash {hash}",
                    )));
                };

                P2PTransaction::EIP4844TransactionWithBlobs(WrappedEIP4844Transaction {
                    tx: itx,
                    blobs_bundle: bundle,
                })
            }
            Transaction::EIP7702Transaction(itx) => P2PTransaction::EIP7702Transaction(itx),
            Transaction::PrivilegedL2Transaction(itx) => {
                P2PTransaction::PrivilegedL2Transaction(itx)
            }
        };

        Ok(result)
    }

    pub fn new_evm(&self, vm_db: StoreVmDatabase) -> Result<Evm, EvmError> {
        let evm = match self.r#type {
            BlockchainType::L1 => Evm::new_for_l1(self.evm_engine, vm_db),
            BlockchainType::L2 => Evm::new_for_l2(self.evm_engine, vm_db)?,
        };
        Ok(evm)
    }
}

pub fn validate_requests_hash(
    header: &BlockHeader,
    chain_config: &ChainConfig,
    requests: &[Requests],
) -> Result<(), ChainError> {
    if !chain_config.is_prague_activated(header.timestamp) {
        return Ok(());
    }

    let encoded_requests: Vec<EncodedRequests> = requests.iter().map(|r| r.encode()).collect();
    let computed_requests_hash = compute_requests_hash(&encoded_requests);
    let valid = header
        .requests_hash
        .map(|requests_hash| requests_hash == computed_requests_hash)
        .unwrap_or(false);

    if !valid {
        return Err(ChainError::InvalidBlock(
            InvalidBlockError::RequestsHashMismatch,
        ));
    }

    Ok(())
}

/// Performs post-execution checks
pub fn validate_state_root(
    block_header: &BlockHeader,
    new_state_root: H256,
) -> Result<(), ChainError> {
    // Compare state root
    if new_state_root == block_header.state_root {
        Ok(())
    } else {
        Err(ChainError::InvalidBlock(
            InvalidBlockError::StateRootMismatch,
        ))
    }
}

pub fn validate_receipts_root(
    block_header: &BlockHeader,
    receipts: &[Receipt],
) -> Result<(), ChainError> {
    let receipts_root = compute_receipts_root(receipts);

    if receipts_root == block_header.receipts_root {
        Ok(())
    } else {
        Err(ChainError::InvalidBlock(
            InvalidBlockError::ReceiptsRootMismatch,
        ))
    }
}

// Returns the hash of the head of the canonical chain (the latest valid hash).
pub async fn latest_canonical_block_hash(storage: &Store) -> Result<H256, ChainError> {
    let latest_block_number = storage.get_latest_block_number().await?;
    if let Some(latest_valid_header) = storage.get_block_header(latest_block_number)? {
        let latest_valid_hash = latest_valid_header.hash();
        return Ok(latest_valid_hash);
    }
    Err(ChainError::StoreError(StoreError::Custom(
        "Could not find latest valid hash".to_string(),
    )))
}

/// Searchs the header of the parent block header. If the parent header is missing,
/// Returns a ChainError::ParentNotFound. If the storage has an error it propagates it
pub fn find_parent_header(
    block_header: &BlockHeader,
    storage: &Store,
) -> Result<BlockHeader, ChainError> {
    match storage.get_block_header_by_hash(block_header.parent_hash)? {
        Some(parent_header) => Ok(parent_header),
        None => Err(ChainError::ParentNotFound),
    }
}

/// Performs pre-execution validation of the block's header values in reference to the parent_header
/// Verifies that blob gas fields in the header are correct in reference to the block's body.
/// If a block passes this check, execution will still fail with execute_block when a transaction runs out of gas
pub fn validate_block(
    block: &Block,
    parent_header: &BlockHeader,
    chain_config: &ChainConfig,
    elasticity_multiplier: u64,
) -> Result<(), ChainError> {
    // Verify initial header validity against parent
    validate_block_header(&block.header, parent_header, elasticity_multiplier)
        .map_err(InvalidBlockError::from)?;

    if chain_config.is_prague_activated(block.header.timestamp) {
        validate_prague_header_fields(&block.header, parent_header, chain_config)
            .map_err(InvalidBlockError::from)?;
        verify_blob_gas_usage(block, chain_config)?;
    } else if chain_config.is_cancun_activated(block.header.timestamp) {
        validate_cancun_header_fields(&block.header, parent_header, chain_config)
            .map_err(InvalidBlockError::from)?;
        verify_blob_gas_usage(block, chain_config)?;
    } else {
        validate_pre_cancun_header_fields(&block.header).map_err(InvalidBlockError::from)?
    }

    Ok(())
}

pub async fn is_canonical(
    store: &Store,
    block_number: BlockNumber,
    block_hash: BlockHash,
) -> Result<bool, StoreError> {
    match store.get_canonical_block_hash(block_number).await? {
        Some(hash) if hash == block_hash => Ok(true),
        _ => Ok(false),
    }
}

pub fn validate_gas_used(
    receipts: &[Receipt],
    block_header: &BlockHeader,
) -> Result<(), ChainError> {
    if let Some(last) = receipts.last() {
        // Note: This is commented because it is still being used in development.
        // dbg!(last.cumulative_gas_used);
        // dbg!(block_header.gas_used);
        if last.cumulative_gas_used != block_header.gas_used {
            return Err(ChainError::InvalidBlock(InvalidBlockError::GasUsedMismatch));
        }
    }
    Ok(())
}

// Perform validations over the block's blob gas usage.
// Must be called only if the block has cancun activated
fn verify_blob_gas_usage(block: &Block, config: &ChainConfig) -> Result<(), ChainError> {
    let mut blob_gas_used = 0_u64;
    let mut blobs_in_block = 0_u64;
    let max_blob_number_per_block = config
        .get_fork_blob_schedule(block.header.timestamp)
        .map(|schedule| schedule.max)
        .ok_or(ChainError::Custom("Provided block fork is invalid".into()))?;
    let max_blob_gas_per_block = max_blob_number_per_block * GAS_PER_BLOB;

    for transaction in block.body.transactions.iter() {
        if let Transaction::EIP4844Transaction(tx) = transaction {
            blob_gas_used += get_total_blob_gas(tx);
            blobs_in_block += tx.blob_versioned_hashes.len() as u64;
        }
    }
    if blob_gas_used > max_blob_gas_per_block {
        return Err(ChainError::InvalidBlock(
            InvalidBlockError::ExceededMaxBlobGasPerBlock,
        ));
    }
    if blobs_in_block > max_blob_number_per_block {
        return Err(ChainError::InvalidBlock(
            InvalidBlockError::ExceededMaxBlobNumberPerBlock,
        ));
    }
    if block
        .header
        .blob_gas_used
        .is_some_and(|header_blob_gas_used| header_blob_gas_used != blob_gas_used)
    {
        return Err(ChainError::InvalidBlock(
            InvalidBlockError::BlobGasUsedMismatch,
        ));
    }
    Ok(())
}

/// Calculates the blob gas required by a transaction
fn get_total_blob_gas(tx: &EIP4844Transaction) -> u64 {
    GAS_PER_BLOB * tx.blob_versioned_hashes.len() as u64
}

#[cfg(test)]
mod tests {}
