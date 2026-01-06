//! PEVM (Parallel EVM) backend for ethrex
//!
//! This module provides an alternative execution backend using pevm for parallel
//! transaction execution. It can be enabled via the `pevm` feature flag.
//!
//! Note: This backend only supports L1 execution. L2 transactions are not supported.

pub mod storage;
pub mod types;

use crate::backends::levm::{extract_all_requests_levm, LEVM};
use crate::errors::EvmError;
use crate::BlockExecutionResult;

use ethrex_common::types::{AccountUpdate, Block, Fork, Receipt};
use ethrex_levm::db::gen_db::GeneralizedDatabase;
use ethrex_levm::vm::VMType;

use hashbrown::HashMap;
use pevm::{BuildSuffixHasher, EvmAccount, Pevm, PevmTxExecutionResult};
use revm::primitives::SpecId;
use std::num::NonZeroUsize;

use self::storage::PevmStorageAdapter;
use self::types::{
    alloy_addr_to_ethrex, alloy_b256_to_ethrex, alloy_u256_to_ethrex, convert_block_env,
    convert_logs_to_ethrex, convert_tx_env, create_receipt, ethrex_addr_to_alloy,
};

/// PEVM backend implementation
pub struct PEVM;

impl PEVM {
    /// Execute a block using PEVM's parallel execution engine
    ///
    /// This function:
    /// 1. Calls LEVM::prepare_block for system contract calls (beacon root, block hash history)
    /// 2. Converts the ethrex Block to revm TxEnv/BlockEnv format
    /// 3. Creates a storage adapter wrapping the GeneralizedDatabase
    /// 4. Executes via PEVM
    /// 5. Converts results back to ethrex format
    /// 6. Applies state changes to the GeneralizedDatabase
    /// 7. Processes withdrawals via LEVM
    /// 8. Extracts requests for Prague+ via LEVM
    pub fn execute_block(
        block: &Block,
        db: &mut GeneralizedDatabase,
    ) -> Result<BlockExecutionResult, EvmError> {
        let vm_type = VMType::L1;

        // Step 1: Prepare block - handles system contract calls (beacon root, block hash history)
        // This uses LEVM for the system contract calls
        LEVM::prepare_block(block, db, vm_type)?;

        // Get chain configuration
        let chain_config = db.store.get_chain_config()?;

        // Create PEVM chain (Ethereum mainnet-style)
        let chain = pevm::chain::PevmEthereum::mainnet();

        // Create storage adapter wrapping the database
        // Note: We need to use the store directly, as prepare_block may have modified the cache
        let storage = PevmStorageAdapter::new(db.store.clone());

        // Convert block header to revm BlockEnv
        let block_env = convert_block_env(&block.header);

        // Determine the spec ID based on fork
        let fork = chain_config.fork(block.header.timestamp);
        let spec_id = fork_to_spec_id(fork);

        // Convert transactions to revm TxEnv format
        let mut txs = Vec::with_capacity(block.body.transactions.len());
        for tx in &block.body.transactions {
            let sender = tx
                .sender()
                .map_err(|e| EvmError::Custom(format!("Failed to recover sender: {:?}", e)))?;
            let tx_env = convert_tx_env(tx, ethrex_addr_to_alloy(&sender))
                .map_err(|e| EvmError::Custom(e))?;
            txs.push(tx_env);
        }

        // Determine concurrency level
        let concurrency = std::thread::available_parallelism()
            .unwrap_or(NonZeroUsize::new(1).expect("1 is non-zero"));

        // Execute with PEVM using execute_revm_parallel
        let mut pevm = Pevm::default();
        let results = pevm
            .execute_revm_parallel(&chain, &storage, spec_id, block_env, txs, concurrency)
            .map_err(|e| EvmError::Custom(format!("PEVM execution error: {:?}", e)))?;

        // Apply state changes to the GeneralizedDatabase and create receipts
        let receipts = Self::apply_results_to_db(block, &results, db)?;

        // Step 7: Process withdrawals
        if let Some(withdrawals) = &block.body.withdrawals {
            LEVM::process_withdrawals(db, withdrawals)?;
        }

        // Step 8: Extract requests (Prague+)
        let requests = extract_all_requests_levm(&receipts, db, &block.header, vm_type)?;

        Ok(BlockExecutionResult { receipts, requests })
    }

    /// Apply PEVM execution results to the GeneralizedDatabase and create receipts
    fn apply_results_to_db(
        block: &Block,
        results: &[PevmTxExecutionResult],
        db: &mut GeneralizedDatabase,
    ) -> Result<Vec<Receipt>, EvmError> {
        let mut receipts = Vec::with_capacity(results.len());
        let mut cumulative_gas_used = 0u64;

        for (i, tx_result) in results.iter().enumerate() {
            let tx = block
                .body
                .transactions
                .get(i)
                .ok_or_else(|| EvmError::Custom("Transaction index out of bounds".to_string()))?;

            // Update cumulative gas
            cumulative_gas_used =
                cumulative_gas_used.saturating_add(tx_result.receipt.cumulative_gas_used);

            // Convert logs
            let logs = convert_logs_to_ethrex(&tx_result.receipt.logs);

            // Determine success from receipt status
            let succeeded = tx_result.receipt.status.coerce_status();

            // Create receipt
            let receipt = create_receipt(tx.tx_type(), succeeded, cumulative_gas_used, logs);
            receipts.push(receipt);

            // Apply state changes to the database
            Self::apply_state_changes(db, &tx_result.state)?;
        }

        Ok(receipts)
    }

    /// Apply state changes from a single transaction to the GeneralizedDatabase
    fn apply_state_changes(
        db: &mut GeneralizedDatabase,
        state: &HashMap<alloy_primitives::Address, Option<EvmAccount>, BuildSuffixHasher>,
    ) -> Result<(), EvmError> {
        // First pass: collect codes to insert (to avoid borrow issues)
        let mut codes_to_insert = Vec::new();

        // EvmStateTransitions is HashMap<Address, Option<EvmAccount>>
        // None means the account was deleted (self-destruct)
        for (address, opt_account) in state.iter() {
            let ethrex_addr = alloy_addr_to_ethrex(address);

            // Get or create account in the cache using public API
            let cached_account = db
                .get_account_mut(ethrex_addr)
                .map_err(|e| EvmError::Custom(format!("Failed to get account: {:?}", e)))?;

            match opt_account {
                Some(account) => {
                    // Update account fields
                    cached_account.info.balance = alloy_u256_to_ethrex(&account.balance);
                    cached_account.info.nonce = account.nonce;

                    // Update code if changed
                    if let Some(code_hash) = &account.code_hash {
                        let ethrex_code_hash = alloy_b256_to_ethrex(code_hash);
                        cached_account.info.code_hash = ethrex_code_hash;

                        // If code is provided, collect it for insertion later
                        if let Some(evm_code) = &account.code {
                            let code_bytes = evm_code_to_bytes(evm_code);
                            let ethrex_code = ethrex_common::types::Code::from_bytecode(
                                bytes::Bytes::from(code_bytes),
                            );
                            codes_to_insert.push((ethrex_code_hash, ethrex_code));
                        }
                    }

                    // Update storage
                    for (key, value) in &account.storage {
                        let ethrex_key = ethrex_common::H256::from_slice(&key.to_be_bytes::<32>());
                        let ethrex_value = alloy_u256_to_ethrex(value);
                        cached_account.storage.insert(ethrex_key, ethrex_value);
                    }
                }
                None => {
                    // Account was deleted (self-destruct)
                    cached_account.info.balance = ethrex_common::U256::zero();
                    cached_account.info.nonce = 0;
                    cached_account.storage.clear();
                }
            }
        }

        // Second pass: insert collected codes
        for (code_hash, code) in codes_to_insert {
            db.codes.insert(code_hash, code);
        }

        Ok(())
    }

    /// Get state transitions from the GeneralizedDatabase
    /// This delegates to the same method used by LEVM
    pub fn get_state_transitions(
        db: &mut GeneralizedDatabase,
    ) -> Result<Vec<AccountUpdate>, EvmError> {
        db.get_state_transitions()
            .map_err(|e| EvmError::Custom(format!("Failed to get state transitions: {:?}", e)))
    }
}

/// Convert ethrex Fork to revm SpecId
fn fork_to_spec_id(fork: Fork) -> SpecId {
    match fork {
        Fork::Frontier => SpecId::FRONTIER,
        Fork::FrontierThawing => SpecId::FRONTIER_THAWING,
        Fork::Homestead => SpecId::HOMESTEAD,
        Fork::DaoFork => SpecId::DAO_FORK,
        Fork::Tangerine => SpecId::TANGERINE,
        Fork::SpuriousDragon => SpecId::SPURIOUS_DRAGON,
        Fork::Byzantium => SpecId::BYZANTIUM,
        Fork::Constantinople => SpecId::CONSTANTINOPLE,
        Fork::Petersburg => SpecId::PETERSBURG,
        Fork::Istanbul => SpecId::ISTANBUL,
        Fork::MuirGlacier => SpecId::MUIR_GLACIER,
        Fork::Berlin => SpecId::BERLIN,
        Fork::London => SpecId::LONDON,
        Fork::ArrowGlacier => SpecId::ARROW_GLACIER,
        Fork::GrayGlacier => SpecId::GRAY_GLACIER,
        Fork::Paris => SpecId::MERGE,
        Fork::Shanghai => SpecId::SHANGHAI,
        Fork::Cancun => SpecId::CANCUN,
        Fork::Prague => SpecId::PRAGUE,
        Fork::Osaka => SpecId::OSAKA,
        // L2-specific forks - map to latest supported spec
        Fork::BPO1 | Fork::BPO2 | Fork::BPO3 | Fork::BPO4 | Fork::BPO5 => SpecId::OSAKA,
    }
}

/// Convert pevm EvmCode to raw bytes
fn evm_code_to_bytes(code: &pevm::EvmCode) -> Vec<u8> {
    // Try to convert EvmCode to revm Bytecode, then extract raw bytes
    match revm::primitives::Bytecode::try_from(code.clone()) {
        Ok(bytecode) => bytecode.original_bytes().to_vec(),
        Err(_) => vec![], // Fallback to empty if conversion fails
    }
}

#[cfg(test)]
mod tests {
    // TODO: Add integration tests comparing LEVM and PEVM results
}
