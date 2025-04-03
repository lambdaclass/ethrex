pub mod constants;
pub mod error;
pub mod fork_choice;
pub mod mempool;
pub mod payload;
mod smoke_test;

use constants::MAX_INITCODE_SIZE;
use error::MempoolError;
use error::{ChainError, InvalidBlockError};
use ethrex_common::constants::{GAS_PER_BLOB, MIN_BASE_FEE_PER_BLOB_GAS};
use ethrex_common::types::requests::{compute_requests_hash, EncodedRequests, Requests};
use ethrex_common::types::BlobsBundle;
use ethrex_common::types::MempoolTransaction;
use ethrex_common::types::{
    compute_receipts_root, validate_block_header, validate_cancun_header_fields,
    validate_prague_header_fields, validate_pre_cancun_header_fields, Block, BlockHash,
    BlockHeader, BlockNumber, ChainConfig, EIP4844Transaction, Receipt, Transaction,
};

use ethrex_common::{Address, H160, H256};
use mempool::Mempool;
use std::collections::HashMap;
use std::{ops::Div, time::Instant};

use ethrex_storage::error::StoreError;
use ethrex_storage::{AccountUpdate, Store};
use ethrex_vm::{BlockExecutionResult, Evm, EvmEngine};
use fork_choice::apply_fork_choice;
use tracing::{error, info, warn};

//TODO: Implement a struct Chain or BlockChain to encapsulate
//functionality and canonical chain state and config

#[derive(Debug)]
pub struct Blockchain {
    pub evm_engine: EvmEngine,
    storage: Store,
    pub mempool: Mempool,
}

#[derive(Debug, Clone)]
pub struct BatchBlockProcessingFailure {
    pub last_valid_hash: H256,
    pub failed_block_hash: H256,
}

impl Blockchain {
    pub fn new(evm_engine: EvmEngine, store: Store) -> Self {
        Self {
            evm_engine,
            storage: store,
            mempool: Mempool::new(),
        }
    }

    pub fn default_with_store(store: Store) -> Self {
        Self {
            evm_engine: EvmEngine::default(),
            storage: store,
            mempool: Mempool::new(),
        }
    }

    /// Executes a block withing a new vm instance and state
    fn execute_block(&self, block: &Block) -> Result<BlockExecutionResult, ChainError> {
        // Validate if it can be the new head and find the parent
        let Ok(parent_header) = find_parent_header(&block.header, &self.storage) else {
            // If the parent is not present, we store it as pending.
            self.storage.add_pending_block(block.clone())?;
            return Err(ChainError::ParentNotFound);
        };
        let chain_config = self.storage.get_chain_config()?;

        // Validate the block pre-execution
        validate_block(block, &parent_header, &chain_config)?;

        let mut vm = Evm::new(
            self.evm_engine,
            self.storage.clone(),
            block.header.parent_hash,
        );
        let execution_result = vm.execute_block(block)?;

        // Validate execution went alright
        validate_gas_used(&execution_result.receipts, &block.header)?;
        validate_receipts_root(&block.header, &execution_result.receipts)?;
        validate_requests_hash(&block.header, &chain_config, &execution_result.requests)?;

        Ok(execution_result)
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
        validate_block(block, parent_header, chain_config)?;

        let execution_result = vm.execute_block(block)?;

        // Validate execution went alright
        validate_gas_used(&execution_result.receipts, &block.header)?;
        validate_receipts_root(&block.header, &execution_result.receipts)?;
        validate_requests_hash(&block.header, chain_config, &execution_result.requests)?;

        Ok(execution_result)
    }

    pub fn store_block(
        &self,
        block: &Block,
        execution_result: BlockExecutionResult,
    ) -> Result<(), ChainError> {
        // Apply the account updates over the last block's state and compute the new state root
        let new_state_root = self
            .storage
            .apply_account_updates(block.header.parent_hash, &execution_result.account_updates)?
            .ok_or(ChainError::ParentStateNotFound)?;

        // Check state root matches the one in block header
        validate_state_root(&block.header, new_state_root)?;

        self.storage
            .add_block(block.clone())
            .map_err(ChainError::StoreError)?;
        self.storage
            .add_receipts(block.hash(), execution_result.receipts)
            .map_err(ChainError::StoreError)
    }

    pub fn add_block(&self, block: &Block) -> Result<(), ChainError> {
        let since = Instant::now();

        let result = self
            .execute_block(block)
            .and_then(|res| self.store_block(block, res));

        let interval = Instant::now().duration_since(since).as_millis();
        if interval != 0 {
            let as_gigas = (block.header.gas_used as f64).div(10_f64.powf(9_f64));
            let throughput = (as_gigas) / (interval as f64) * 1000_f64;
            info!("[METRIC] BLOCK EXECUTION THROUGHPUT: {throughput} Gigagas/s TIME SPENT: {interval} msecs");
        }

        result
    }

    /// Adds multiple blocks in a batch.
    ///
    /// If an error occurs, returns a tuple containing:
    /// - The error type ([`ChainError`]).
    /// - [`BatchProcessingFailure`] (if the error was caused by block processing).
    ///
    /// Note: only the last block's state trie is stored in the db
    pub fn add_blocks_in_batch(
        &self,
        blocks: &[Block],
    ) -> Result<(), (ChainError, Option<BatchBlockProcessingFailure>)> {
        let mut last_valid_hash = H256::default();

        let Some(first_block_header) = blocks.first().map(|e| e.header.clone()) else {
            return Err((ChainError::Custom("First block not found".into()), None));
        };

        let chain_config: ChainConfig = self
            .storage
            .get_chain_config()
            .map_err(|e| (e.into(), None))?;
        let mut vm = Evm::new(
            self.evm_engine,
            self.storage.clone(),
            first_block_header.parent_hash,
        );

        let blocks_len = blocks.len();
        let mut all_receipts: HashMap<BlockHash, Vec<Receipt>> = HashMap::new();
        let mut all_account_updates: HashMap<H160, AccountUpdate> = HashMap::new();
        let mut total_gas_used = 0;
        let mut transactions_count = 0;

        let interval = Instant::now();
        for (i, block) in blocks.iter().enumerate() {
            // for the first block, we need to query the store
            let parent_header = if i == 0 {
                let Ok(parent_header) = find_parent_header(&block.header, &self.storage) else {
                    return Err((
                        ChainError::ParentNotFound,
                        Some(BatchBlockProcessingFailure {
                            failed_block_hash: block.hash(),
                            last_valid_hash,
                        }),
                    ));
                };
                parent_header
            } else {
                // for the subsequent ones, the parent is the previous block
                blocks[i - 1].header.clone()
            };

            let BlockExecutionResult {
                receipts,
                account_updates,
                ..
            } = match self.execute_block_from_state(&parent_header, block, &chain_config, &mut vm) {
                Ok(result) => result,
                Err(err) => {
                    return Err((
                        err,
                        Some(BatchBlockProcessingFailure {
                            failed_block_hash: block.hash(),
                            last_valid_hash,
                        }),
                    ))
                }
            };

            // Merge account updates
            for account_update in account_updates {
                let Some(cache) = all_account_updates.get_mut(&account_update.address) else {
                    all_account_updates.insert(account_update.address, account_update);
                    continue;
                };

                cache.removed = account_update.removed;
                if let Some(code) = account_update.code {
                    cache.code = Some(code);
                };

                if let Some(info) = account_update.info {
                    cache.info = Some(info);
                }

                for (k, v) in account_update.added_storage.into_iter() {
                    cache.added_storage.insert(k, v);
                }
            }

            last_valid_hash = block.hash();
            total_gas_used += block.header.gas_used;
            transactions_count += block.body.transactions.len();
            all_receipts.insert(block.hash(), receipts);
        }

        let Some(last_block) = blocks.last() else {
            return Err((ChainError::Custom("Last block not found".into()), None));
        };

        // Apply the account updates over all blocks and compute the new state root
        let new_state_root = self
            .storage
            .apply_account_updates(
                first_block_header.parent_hash,
                &all_account_updates.into_values().collect::<Vec<_>>(),
            )
            .map_err(|e| (e.into(), None))?
            .ok_or((ChainError::ParentStateNotFound, None))?;

        // Check state root matches the one in block header
        validate_state_root(&last_block.header, new_state_root).map_err(|e| (e, None))?;

        self.storage
            .add_blocks(blocks)
            .map_err(|e| (e.into(), None))?;
        self.storage
            .add_receipts_for_blocks(all_receipts)
            .map_err(|e| (e.into(), None))?;

        let elapsed_total = interval.elapsed().as_millis();
        let mut throughput = 0.0;
        if elapsed_total != 0 && total_gas_used != 0 {
            let as_gigas = (total_gas_used as f64).div(10_f64.powf(9_f64));
            throughput = (as_gigas) / (elapsed_total as f64) * 1000_f64;
        }

        info!(
            "[METRICS] Executed and stored: Range: {}, Total transactions: {}, Throughput: {} Gigagas/s",
            blocks_len, transactions_count, throughput
        );

        Ok(())
    }

    //TODO: Forkchoice Update shouldn't be part of this function
    pub fn import_blocks(&self, blocks: &Vec<Block>) {
        let size = blocks.len();
        for block in blocks {
            let hash = block.hash();
            info!(
                "Adding block {} with hash {:#x}.",
                block.header.number, hash
            );
            if let Err(error) = self.add_block(block) {
                warn!(
                    "Failed to add block {} with hash {:#x}: {}.",
                    block.header.number, hash, error
                );
            }
            if self
                .storage
                .update_latest_block_number(block.header.number)
                .is_err()
            {
                error!("Fatal: added block {} but could not update the block number -- aborting block import", block.header.number);
                break;
            };
            if self
                .storage
                .set_canonical_block(block.header.number, hash)
                .is_err()
            {
                error!(
                    "Fatal: added block {} but could not set it as canonical -- aborting block import",
                    block.header.number
                );
                break;
            };
        }
        if let Some(last_block) = blocks.last() {
            let hash = last_block.hash();
            match self.evm_engine {
                EvmEngine::LEVM => {
                    // We are allowing this not to unwrap so that tests can run even if block execution results in the wrong root hash with LEVM.
                    let _ = apply_fork_choice(&self.storage, hash, hash, hash);
                }
                EvmEngine::REVM => {
                    apply_fork_choice(&self.storage, hash, hash, hash).unwrap();
                }
            }
        }
        info!("Added {size} blocks to blockchain");
    }

    /// Add a blob transaction and its blobs bundle to the mempool checking that the transaction is valid
    #[cfg(feature = "c-kzg")]
    pub fn add_blob_transaction_to_pool(
        &self,
        transaction: EIP4844Transaction,
        blobs_bundle: BlobsBundle,
    ) -> Result<H256, MempoolError> {
        // Validate blobs bundle

        blobs_bundle.validate(&transaction)?;

        let transaction = Transaction::EIP4844Transaction(transaction);
        let sender = transaction.sender();

        // Validate transaction
        self.validate_transaction(&transaction, sender)?;

        // Add transaction and blobs bundle to storage
        let hash = transaction.compute_hash();
        self.mempool
            .add_transaction(hash, MempoolTransaction::new(transaction, sender))?;
        self.mempool.add_blobs_bundle(hash, blobs_bundle)?;
        Ok(hash)
    }

    /// Add a transaction to the mempool checking that the transaction is valid
    pub fn add_transaction_to_pool(&self, transaction: Transaction) -> Result<H256, MempoolError> {
        // Blob transactions should be submitted via add_blob_transaction along with the corresponding blobs bundle
        if matches!(transaction, Transaction::EIP4844Transaction(_)) {
            return Err(MempoolError::BlobTxNoBlobsBundle);
        }
        let sender = transaction.sender();
        // Validate transaction
        self.validate_transaction(&transaction, sender)?;

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

    pub fn validate_transaction(
        &self,
        tx: &Transaction,
        sender: Address,
    ) -> Result<(), MempoolError> {
        // TODO: Add validations here

        let header_no = self.storage.get_latest_block_number()?;
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

        let maybe_sender_acc_info = self.storage.get_account_info(header_no, sender)?;

        if let Some(sender_acc_info) = maybe_sender_acc_info {
            if tx.nonce() < sender_acc_info.nonce {
                return Err(MempoolError::InvalidNonce);
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

        if let Some(chain_id) = tx.chain_id() {
            if chain_id != config.chain_id {
                return Err(MempoolError::InvalidChainId(config.chain_id));
            }
        }

        Ok(())
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
pub fn latest_canonical_block_hash(storage: &Store) -> Result<H256, ChainError> {
    let latest_block_number = storage.get_latest_block_number()?;
    if let Some(latest_valid_header) = storage.get_block_header(latest_block_number)? {
        let latest_valid_hash = latest_valid_header.compute_block_hash();
        return Ok(latest_valid_hash);
    }
    Err(ChainError::StoreError(StoreError::Custom(
        "Could not find latest valid hash".to_string(),
    )))
}

/// Validates if the provided block could be the new head of the chain, and returns the
/// parent_header in that case. If not found, the new block is saved as pending.
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
) -> Result<(), ChainError> {
    // Verify initial header validity against parent
    validate_block_header(&block.header, parent_header).map_err(InvalidBlockError::from)?;

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

pub fn is_canonical(
    store: &Store,
    block_number: BlockNumber,
    block_hash: BlockHash,
) -> Result<bool, StoreError> {
    match store.get_canonical_block_hash(block_number)? {
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
