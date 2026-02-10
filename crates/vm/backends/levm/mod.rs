pub mod db;
mod tracing;

use super::BlockExecutionResult;
use crate::system_contracts::{
    BEACON_ROOTS_ADDRESS, CONSOLIDATION_REQUEST_PREDEPLOY_ADDRESS, HISTORY_STORAGE_ADDRESS,
    PRAGUE_SYSTEM_CONTRACTS, SYSTEM_ADDRESS, WITHDRAWAL_REQUEST_PREDEPLOY_ADDRESS,
};
use crate::{EvmError, ExecutionResult};
use bytes::Bytes;
use ethrex_common::types::block_access_list::BlockAccessList;
use ethrex_common::types::fee_config::FeeConfig;
use ethrex_common::types::{AuthorizationTuple, EIP7702Transaction};
use ethrex_common::{
    Address, U256,
    types::{
        AccessList, AccountUpdate, Block, BlockHeader, EIP1559Transaction, Fork, GWEI_TO_WEI,
        GenericTransaction, INITIAL_BASE_FEE, Receipt, Transaction, TxKind, Withdrawal,
        requests::Requests,
    },
};
use ethrex_levm::EVMConfig;
use ethrex_levm::call_frame::Stack;
use ethrex_levm::constants::{
    POST_OSAKA_GAS_LIMIT_CAP, STACK_LIMIT, SYS_CALL_GAS_LIMIT, TX_BASE_COST,
};
use ethrex_levm::db::Database;
use ethrex_levm::db::gen_db::GeneralizedDatabase;
use ethrex_levm::errors::{InternalError, TxValidationError};
#[cfg(feature = "perf_opcode_timings")]
use ethrex_levm::timings::{OPCODE_TIMINGS, PRECOMPILES_TIMINGS};
use ethrex_levm::tracing::LevmCallTracer;
use ethrex_levm::utils::get_base_fee_per_blob_gas;
use ethrex_levm::vm::VMType;
use ethrex_levm::{
    Environment,
    errors::{ExecutionReport, TxResult, VMError},
    vm::VM,
};
use rayon::iter::{IntoParallelIterator, ParallelIterator};
use rustc_hash::FxHashMap;
use std::cmp::min;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::Sender;

/// The struct implements the following functions:
/// [LEVM::execute_block]
/// [LEVM::execute_tx]
/// [LEVM::get_state_transitions]
/// [LEVM::process_withdrawals]
#[derive(Debug)]
pub struct LEVM;

impl LEVM {
    /// Execute a block and return the execution result.
    ///
    /// Also records and returns the Block Access List (EIP-7928) for Amsterdam+ forks.
    /// The BAL will be `None` for pre-Amsterdam forks.
    pub fn execute_block(
        block: &Block,
        db: &mut GeneralizedDatabase,
        vm_type: VMType,
    ) -> Result<(BlockExecutionResult, Option<BlockAccessList>), EvmError> {
        let chain_config = db.store.get_chain_config()?;
        let record_bal = chain_config.is_amsterdam_activated(block.header.timestamp);

        // Enable BAL recording for Amsterdam+ forks
        if record_bal {
            db.enable_bal_recording();
            // Set index 0 for pre-execution phase (system contracts)
            db.set_bal_index(0);
        }

        Self::prepare_block(block, db, vm_type)?;

        let mut receipts = Vec::new();
        // Cumulative gas for receipts (POST-REFUND per EIP-7778)
        let mut cumulative_gas_used = 0_u64;
        // Block gas accounting (PRE-REFUND for Amsterdam+ per EIP-7778)
        let mut block_gas_used = 0_u64;

        let transactions_with_sender =
            block.body.get_transactions_with_sender().map_err(|error| {
                EvmError::Transaction(format!("Couldn't recover addresses with error: {error}"))
            })?;

        for (tx_idx, (tx, tx_sender)) in transactions_with_sender.into_iter().enumerate() {
            // Use block_gas_used for limit check (pre-refund for Amsterdam+)
            if block_gas_used + tx.gas_limit() > block.header.gas_limit {
                return Err(EvmError::Transaction(format!(
                    "Gas allowance exceeded. Block gas limit {} can be surpassed by executing transaction with gas limit {}",
                    block.header.gas_limit,
                    tx.gas_limit()
                )));
            }

            // Set BAL index for this transaction (1-indexed per EIP-7928, uint16)
            if record_bal {
                #[allow(clippy::cast_possible_truncation)]
                db.set_bal_index((tx_idx + 1) as u16);

                // Record tx sender and recipient for BAL
                if let Some(recorder) = db.bal_recorder_mut() {
                    recorder.record_touched_address(tx_sender);
                    if let TxKind::Call(to) = tx.to() {
                        recorder.record_touched_address(to);
                    }
                }
            }

            let report = Self::execute_tx(tx, tx_sender, &block.header, db, vm_type)?;

            // EIP-7778: Separate gas tracking
            // - gas_spent (POST-REFUND) for receipt cumulative_gas_used
            // - gas_used (PRE-REFUND for Amsterdam+) for block accounting
            cumulative_gas_used += report.gas_spent;
            block_gas_used += report.gas_used;

            let receipt = Receipt::new(
                tx.tx_type(),
                matches!(report.result, TxResult::Success),
                cumulative_gas_used,
                report.logs,
            );

            receipts.push(receipt);
        }

        // Set BAL index for post-execution phase (withdrawals, uint16)
        if record_bal {
            #[allow(clippy::cast_possible_truncation)]
            let withdrawal_index = (block.body.transactions.len() + 1) as u16;
            db.set_bal_index(withdrawal_index);
        }

        if let Some(withdrawals) = &block.body.withdrawals {
            // Record ALL withdrawal recipients for BAL per EIP-7928:
            // "Withdrawal recipients regardless of amount"
            // The amount filter only applies to balance_changes, not touched_addresses
            if record_bal && let Some(recorder) = db.bal_recorder_mut() {
                recorder.extend_touched_addresses(withdrawals.iter().map(|w| w.address));
            }
            Self::process_withdrawals(db, withdrawals)?;
        }

        // TODO: I don't like deciding the behavior based on the VMType here.
        // TODO2: Revise this, apparently extract_all_requests_levm is not called
        // in L2 execution, but its implementation behaves differently based on this.
        let requests = match vm_type {
            VMType::L1 => extract_all_requests_levm(&receipts, db, &block.header, vm_type)?,
            VMType::L2(_) => Default::default(),
        };

        // Extract BAL if recording was enabled
        let bal = db.take_bal();

        Ok((
            BlockExecutionResult {
                receipts,
                requests,
                block_gas_used,
            },
            bal,
        ))
    }

    pub fn execute_block_pipeline(
        block: &Block,
        db: &mut GeneralizedDatabase,
        vm_type: VMType,
        merkleizer: Sender<Vec<AccountUpdate>>,
        queue_length: &AtomicUsize,
    ) -> Result<(BlockExecutionResult, Option<BlockAccessList>), EvmError> {
        let chain_config = db.store.get_chain_config()?;
        let record_bal = chain_config.is_amsterdam_activated(block.header.timestamp);

        // Enable BAL recording for Amsterdam+ forks
        if record_bal {
            db.enable_bal_recording();
            // Set index 0 for pre-execution phase (system contracts)
            db.set_bal_index(0);
        }

        Self::prepare_block(block, db, vm_type)?;

        let mut shared_stack_pool = Vec::with_capacity(STACK_LIMIT);

        let mut receipts = Vec::new();
        // Cumulative gas for receipts (POST-REFUND per EIP-7778)
        let mut cumulative_gas_used = 0_u64;
        // Block gas accounting (PRE-REFUND for Amsterdam+ per EIP-7778)
        let mut block_gas_used = 0_u64;

        // Starts at 2 to account for the two precompile calls done in `Self::prepare_block`.
        // The value itself can be safely changed.
        let mut tx_since_last_flush = 2;

        let transactions_with_sender =
            block.body.get_transactions_with_sender().map_err(|error| {
                EvmError::Transaction(format!("Couldn't recover addresses with error: {error}"))
            })?;

        for (tx_idx, (tx, tx_sender)) in transactions_with_sender.into_iter().enumerate() {
            // Use block_gas_used for limit check (pre-refund for Amsterdam+)
            if block_gas_used + tx.gas_limit() > block.header.gas_limit {
                return Err(EvmError::Transaction(format!(
                    "Gas allowance exceeded. Block gas limit {} can be surpassed by executing transaction with gas limit {}",
                    block.header.gas_limit,
                    tx.gas_limit()
                )));
            }

            // Set BAL index for this transaction (1-indexed per EIP-7928, uint16)
            if record_bal {
                #[allow(clippy::cast_possible_truncation)]
                db.set_bal_index((tx_idx + 1) as u16);

                // Record tx sender and recipient for BAL
                if let Some(recorder) = db.bal_recorder_mut() {
                    recorder.record_touched_address(tx_sender);
                    if let TxKind::Call(to) = tx.to() {
                        recorder.record_touched_address(to);
                    }
                }
            }

            let report = Self::execute_tx_in_block(
                tx,
                tx_sender,
                &block.header,
                db,
                vm_type,
                &mut shared_stack_pool,
                false, // NOT warming - full validation
            )?;
            if queue_length.load(Ordering::Relaxed) == 0 && tx_since_last_flush > 5 {
                LEVM::send_state_transitions_tx(&merkleizer, db, queue_length)?;
                tx_since_last_flush = 0;
            } else {
                tx_since_last_flush += 1;
            }

            // EIP-7778: Separate gas tracking
            // - gas_spent (POST-REFUND) for receipt cumulative_gas_used
            // - gas_used (PRE-REFUND for Amsterdam+) for block accounting
            cumulative_gas_used += report.gas_spent;
            block_gas_used += report.gas_used;

            let receipt = Receipt::new(
                tx.tx_type(),
                matches!(report.result, TxResult::Success),
                cumulative_gas_used,
                report.logs,
            );

            receipts.push(receipt);
        }

        #[cfg(feature = "perf_opcode_timings")]
        {
            let mut timings = OPCODE_TIMINGS.lock().expect("poison");
            timings.inc_tx_count(receipts.len());
            timings.inc_block_count();
            ::tracing::info!("{}", timings.info_pretty());
            let precompiles_timings = PRECOMPILES_TIMINGS.lock().expect("poison");
            ::tracing::info!("{}", precompiles_timings.info_pretty());
        }

        if queue_length.load(Ordering::Relaxed) == 0 {
            LEVM::send_state_transitions_tx(&merkleizer, db, queue_length)?;
        }

        // Set BAL index for post-execution phase (withdrawals, uint16)
        if record_bal {
            #[allow(clippy::cast_possible_truncation)]
            let withdrawal_index = (block.body.transactions.len() + 1) as u16;
            db.set_bal_index(withdrawal_index);
        }

        if let Some(withdrawals) = &block.body.withdrawals {
            // Record ALL withdrawal recipients for BAL per EIP-7928
            if record_bal && let Some(recorder) = db.bal_recorder_mut() {
                recorder.extend_touched_addresses(withdrawals.iter().map(|w| w.address));
            }
            Self::process_withdrawals(db, withdrawals)?;
        }

        // TODO: I don't like deciding the behavior based on the VMType here.
        // TODO2: Revise this, apparently extract_all_requests_levm is not called
        // in L2 execution, but its implementation behaves differently based on this.
        let requests = match vm_type {
            VMType::L1 => extract_all_requests_levm(&receipts, db, &block.header, vm_type)?,
            VMType::L2(_) => Default::default(),
        };
        LEVM::send_state_transitions_tx(&merkleizer, db, queue_length)?;

        // Extract BAL if recording was enabled
        let bal = db.take_bal();

        Ok((
            BlockExecutionResult {
                receipts,
                requests,
                block_gas_used,
            },
            bal,
        ))
    }

    /// Pre-warms state by executing all transactions in parallel, grouped by sender.
    ///
    /// Transactions from the same sender are executed sequentially within their group
    /// to ensure correct nonce and balance propagation. Different sender groups run
    /// in parallel. This approach (inspired by Nethermind's per-sender prewarmer)
    /// improves warmup accuracy by avoiding nonce mismatches within sender groups.
    ///
    /// The `store` parameter should be a `CachingDatabase`-wrapped store so that
    /// parallel workers can benefit from shared caching. The same cache should
    /// be used by the sequential execution phase.
    pub fn warm_block(
        block: &Block,
        store: Arc<dyn Database>,
        vm_type: VMType,
    ) -> Result<(), EvmError> {
        let mut db = GeneralizedDatabase::new(store.clone());

        let txs_with_sender = block.body.get_transactions_with_sender().map_err(|error| {
            EvmError::Transaction(format!("Couldn't recover addresses with error: {error}"))
        })?;

        // Group transactions by sender for sequential execution within groups
        let mut sender_groups: FxHashMap<Address, Vec<&Transaction>> = FxHashMap::default();
        for (tx, sender) in &txs_with_sender {
            sender_groups.entry(*sender).or_default().push(tx);
        }

        // Parallel across sender groups, sequential within each group
        sender_groups.into_par_iter().for_each_with(
            Vec::with_capacity(STACK_LIMIT),
            |stack_pool, (sender, txs)| {
                // Each sender group gets its own db instance for state propagation
                let mut group_db = GeneralizedDatabase::new(store.clone());

                // Execute transactions sequentially within sender group
                // This ensures nonce and balance changes from tx[N] are visible to tx[N+1]
                for tx in txs {
                    let _ = Self::execute_tx_in_block(
                        tx,
                        sender,
                        &block.header,
                        &mut group_db,
                        vm_type,
                        stack_pool,
                        true, // WARMING - skip balance/nonce checks
                    );
                }
            },
        );

        for withdrawal in block
            .body
            .withdrawals
            .iter()
            .flatten()
            .filter(|withdrawal| withdrawal.amount > 0)
        {
            db.get_account_mut(withdrawal.address).map_err(|_| {
                EvmError::DB(format!(
                    "Withdrawal account {} not found",
                    withdrawal.address
                ))
            })?;
        }
        Ok(())
    }

    fn send_state_transitions_tx(
        merkleizer: &Sender<Vec<AccountUpdate>>,
        db: &mut GeneralizedDatabase,
        queue_length: &AtomicUsize,
    ) -> Result<(), EvmError> {
        let transitions = LEVM::get_state_transitions_tx(db)?;
        merkleizer
            .send(transitions)
            .map_err(|e| EvmError::Custom(format!("send failed: {e}")))?;
        queue_length.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    fn setup_env(
        tx: &Transaction,
        tx_sender: Address,
        block_header: &BlockHeader,
        db: &GeneralizedDatabase,
        vm_type: VMType,
        is_warming: bool,
    ) -> Result<Environment, EvmError> {
        let chain_config = db.store.get_chain_config()?;
        let gas_price: U256 = calculate_gas_price_for_tx(
            tx,
            block_header.base_fee_per_gas.unwrap_or_default(),
            &vm_type,
        )?;

        let block_excess_blob_gas = block_header.excess_blob_gas.map(U256::from);
        let config = EVMConfig::new_from_chain_config(&chain_config, block_header);
        let env = Environment {
            origin: tx_sender,
            gas_limit: tx.gas_limit(),
            config,
            block_number: block_header.number.into(),
            coinbase: block_header.coinbase,
            timestamp: block_header.timestamp.into(),
            prev_randao: Some(block_header.prev_randao),
            slot_number: block_header
                .slot_number
                .map(U256::from)
                .unwrap_or(U256::zero()),
            chain_id: chain_config.chain_id.into(),
            base_fee_per_gas: block_header.base_fee_per_gas.unwrap_or_default().into(),
            base_blob_fee_per_gas: get_base_fee_per_blob_gas(block_excess_blob_gas, &config)?,
            gas_price,
            block_excess_blob_gas,
            block_blob_gas_used: block_header.blob_gas_used.map(U256::from),
            tx_blob_hashes: tx.blob_versioned_hashes(),
            tx_max_priority_fee_per_gas: tx.max_priority_fee().map(U256::from),
            tx_max_fee_per_gas: tx.max_fee_per_gas().map(U256::from),
            tx_max_fee_per_blob_gas: tx.max_fee_per_blob_gas(),
            tx_nonce: tx.nonce(),
            block_gas_limit: block_header.gas_limit,
            difficulty: block_header.difficulty,
            is_privileged: matches!(tx, Transaction::PrivilegedL2Transaction(_)),
            is_warming,
            fee_token: tx.fee_token(),
        };

        Ok(env)
    }

    pub fn execute_tx(
        // The transaction to execute.
        tx: &Transaction,
        // The transaction's recovered address
        tx_sender: Address,
        // The block header for the current block.
        block_header: &BlockHeader,
        db: &mut GeneralizedDatabase,
        vm_type: VMType,
    ) -> Result<ExecutionReport, EvmError> {
        let env = Self::setup_env(tx, tx_sender, block_header, db, vm_type, false)?;
        let mut vm = VM::new(env, db, tx, LevmCallTracer::disabled(), vm_type)?;

        vm.execute().map_err(VMError::into)
    }

    // Like execute_tx but allows reusing the stack pool
    fn execute_tx_in_block(
        // The transaction to execute.
        tx: &Transaction,
        // The transaction's recovered address
        tx_sender: Address,
        // The block header for the current block.
        block_header: &BlockHeader,
        db: &mut GeneralizedDatabase,
        vm_type: VMType,
        stack_pool: &mut Vec<Stack>,
        is_warming: bool,
    ) -> Result<ExecutionReport, EvmError> {
        let env = Self::setup_env(tx, tx_sender, block_header, db, vm_type, is_warming)?;
        let mut vm = VM::new(env, db, tx, LevmCallTracer::disabled(), vm_type)?;

        std::mem::swap(&mut vm.stack_pool, stack_pool);
        let result = vm.execute().map_err(VMError::into);
        std::mem::swap(&mut vm.stack_pool, stack_pool);
        result
    }

    pub fn undo_last_tx(db: &mut GeneralizedDatabase) -> Result<(), EvmError> {
        db.undo_last_transaction()?;
        Ok(())
    }

    pub fn simulate_tx_from_generic(
        // The transaction to execute.
        tx: &GenericTransaction,
        // The block header for the current block.
        block_header: &BlockHeader,
        db: &mut GeneralizedDatabase,
        vm_type: VMType,
    ) -> Result<ExecutionResult, EvmError> {
        let mut env = env_from_generic(tx, block_header, db, vm_type)?;

        env.block_gas_limit = i64::MAX as u64; // disable block gas limit

        adjust_disabled_base_fee(&mut env);

        let mut vm = vm_from_generic(tx, env, db, vm_type)?;

        vm.execute()
            .map(|value| value.into())
            .map_err(VMError::into)
    }

    pub fn get_state_transitions(
        db: &mut GeneralizedDatabase,
    ) -> Result<Vec<AccountUpdate>, EvmError> {
        Ok(db.get_state_transitions()?)
    }

    pub fn get_state_transitions_tx(
        db: &mut GeneralizedDatabase,
    ) -> Result<Vec<AccountUpdate>, EvmError> {
        Ok(db.get_state_transitions_tx()?)
    }

    pub fn process_withdrawals(
        db: &mut GeneralizedDatabase,
        withdrawals: &[Withdrawal],
    ) -> Result<(), EvmError> {
        // For every withdrawal we increment the target account's balance
        for (address, increment) in withdrawals
            .iter()
            .filter(|withdrawal| withdrawal.amount > 0)
            .map(|w| (w.address, u128::from(w.amount) * u128::from(GWEI_TO_WEI)))
        {
            let account = db
                .get_account_mut(address)
                .map_err(|_| EvmError::DB(format!("Withdrawal account {address} not found")))?;

            let initial_balance = account.info.balance;
            account.info.balance += increment.into();
            let new_balance = account.info.balance;

            // Record balance change for BAL (EIP-7928)
            if let Some(recorder) = db.bal_recorder_mut() {
                recorder.set_initial_balance(address, initial_balance);
                recorder.record_balance_change(address, new_balance);
            }
        }
        Ok(())
    }

    // SYSTEM CONTRACTS
    pub fn beacon_root_contract_call(
        block_header: &BlockHeader,
        db: &mut GeneralizedDatabase,
        vm_type: VMType,
    ) -> Result<(), EvmError> {
        if let VMType::L2(_) = vm_type {
            return Err(EvmError::InvalidEVM(
                "beacon_root_contract_call should not be called for L2 VM".to_string(),
            ));
        }

        let beacon_root = block_header.parent_beacon_block_root.ok_or_else(|| {
            EvmError::Header("parent_beacon_block_root field is missing".to_string())
        })?;

        generic_system_contract_levm(
            block_header,
            Bytes::copy_from_slice(beacon_root.as_bytes()),
            db,
            BEACON_ROOTS_ADDRESS.address,
            SYSTEM_ADDRESS,
            vm_type,
        )?;
        Ok(())
    }

    pub fn process_block_hash_history(
        block_header: &BlockHeader,
        db: &mut GeneralizedDatabase,
        vm_type: VMType,
    ) -> Result<(), EvmError> {
        if let VMType::L2(_) = vm_type {
            return Err(EvmError::InvalidEVM(
                "process_block_hash_history should not be called for L2 VM".to_string(),
            ));
        }

        generic_system_contract_levm(
            block_header,
            Bytes::copy_from_slice(block_header.parent_hash.as_bytes()),
            db,
            HISTORY_STORAGE_ADDRESS.address,
            SYSTEM_ADDRESS,
            vm_type,
        )?;
        Ok(())
    }
    pub(crate) fn read_withdrawal_requests(
        block_header: &BlockHeader,
        db: &mut GeneralizedDatabase,
        vm_type: VMType,
    ) -> Result<ExecutionReport, EvmError> {
        if let VMType::L2(_) = vm_type {
            return Err(EvmError::InvalidEVM(
                "read_withdrawal_requests should not be called for L2 VM".to_string(),
            ));
        }

        let report = generic_system_contract_levm(
            block_header,
            Bytes::new(),
            db,
            WITHDRAWAL_REQUEST_PREDEPLOY_ADDRESS.address,
            SYSTEM_ADDRESS,
            vm_type,
        )?;

        match report.result {
            TxResult::Success => Ok(report),
            // EIP-7002 specifies that a failed system call invalidates the entire block.
            TxResult::Revert(vm_error) => Err(EvmError::SystemContractCallFailed(format!(
                "REVERT when reading withdrawal requests with error: {vm_error:?}. According to EIP-7002, the revert of this system call invalidates the block.",
            ))),
        }
    }

    pub(crate) fn dequeue_consolidation_requests(
        block_header: &BlockHeader,
        db: &mut GeneralizedDatabase,
        vm_type: VMType,
    ) -> Result<ExecutionReport, EvmError> {
        if let VMType::L2(_) = vm_type {
            return Err(EvmError::InvalidEVM(
                "dequeue_consolidation_requests should not be called for L2 VM".to_string(),
            ));
        }

        let report = generic_system_contract_levm(
            block_header,
            Bytes::new(),
            db,
            CONSOLIDATION_REQUEST_PREDEPLOY_ADDRESS.address,
            SYSTEM_ADDRESS,
            vm_type,
        )?;

        match report.result {
            TxResult::Success => Ok(report),
            // EIP-7251 specifies that a failed system call invalidates the entire block.
            TxResult::Revert(vm_error) => Err(EvmError::SystemContractCallFailed(format!(
                "REVERT when dequeuing consolidation requests with error: {vm_error:?}. According to EIP-7251, the revert of this system call invalidates the block.",
            ))),
        }
    }

    pub fn create_access_list(
        mut tx: GenericTransaction,
        header: &BlockHeader,
        db: &mut GeneralizedDatabase,
        vm_type: VMType,
    ) -> Result<(ExecutionResult, AccessList), VMError> {
        let mut env = env_from_generic(&tx, header, db, vm_type)?;

        adjust_disabled_base_fee(&mut env);

        let mut vm = vm_from_generic(&tx, env.clone(), db, vm_type)?;

        vm.stateless_execute()?;

        // Execute the tx again, now with the created access list.
        tx.access_list = vm.substate.make_access_list();
        let mut vm = vm_from_generic(&tx, env, db, vm_type)?;

        let report = vm.stateless_execute()?;

        Ok((
            report.into(),
            tx.access_list
                .into_iter()
                .map(|x| (x.address, x.storage_keys))
                .collect(),
        ))
    }

    pub fn prepare_block(
        block: &Block,
        db: &mut GeneralizedDatabase,
        vm_type: VMType,
    ) -> Result<(), EvmError> {
        let chain_config = db.store.get_chain_config()?;
        let block_header = &block.header;
        let fork = chain_config.fork(block_header.timestamp);

        // TODO: I don't like deciding the behavior based on the VMType here.
        if let VMType::L2(_) = vm_type {
            return Ok(());
        }

        if block_header.parent_beacon_block_root.is_some() && fork >= Fork::Cancun {
            Self::beacon_root_contract_call(block_header, db, vm_type)?;
        }

        if fork >= Fork::Prague {
            //eip 2935: stores parent block hash in system contract
            Self::process_block_hash_history(block_header, db, vm_type)?;
        }
        Ok(())
    }
}

pub fn generic_system_contract_levm(
    block_header: &BlockHeader,
    calldata: Bytes,
    db: &mut GeneralizedDatabase,
    contract_address: Address,
    system_address: Address,
    vm_type: VMType,
) -> Result<ExecutionReport, EvmError> {
    let chain_config = db.store.get_chain_config()?;
    let config = EVMConfig::new_from_chain_config(&chain_config, block_header);
    let system_account_backup = db.current_accounts_state.get(&system_address).cloned();
    let coinbase_backup = db
        .current_accounts_state
        .get(&block_header.coinbase)
        .cloned();
    let env = Environment {
        origin: system_address,
        // EIPs 2935, 4788, 7002 and 7251 dictate that the system calls have a gas limit of 30 million and they do not use intrinsic gas.
        // So we add the base cost that will be taken in the execution.
        gas_limit: SYS_CALL_GAS_LIMIT + TX_BASE_COST,
        block_number: block_header.number.into(),
        coinbase: block_header.coinbase,
        timestamp: block_header.timestamp.into(),
        prev_randao: Some(block_header.prev_randao),
        base_fee_per_gas: U256::zero(),
        gas_price: U256::zero(),
        block_excess_blob_gas: block_header.excess_blob_gas.map(U256::from),
        block_blob_gas_used: block_header.blob_gas_used.map(U256::from),
        block_gas_limit: i64::MAX as u64, // System calls, have no constraint on the block's gas limit.
        config,
        ..Default::default()
    };

    // This check is not necessary in practice, since contract deployment has succesfully happened in all relevant testnets and mainnet
    // However, it's necessary to pass some of the Hive tests related to system contract deployment, which is why we have it
    // The error that should be returned for the relevant contracts is indicated in the following:
    // https://github.com/ethereum/EIPs/blob/master/EIPS/eip-7002.md#empty-code-failure
    // https://github.com/ethereum/EIPs/blob/master/EIPS/eip-7251.md#empty-code-failure
    if PRAGUE_SYSTEM_CONTRACTS
        .iter()
        .any(|contract| contract.address == contract_address)
        && db.get_account_code(contract_address)?.bytecode.is_empty()
    {
        return Err(EvmError::SystemContractCallFailed(format!(
            "System contract: {contract_address} has no code after deployment"
        )));
    };

    let tx = &Transaction::EIP1559Transaction(EIP1559Transaction {
        to: TxKind::Call(contract_address),
        value: U256::zero(),
        data: calldata,
        ..Default::default()
    });
    let mut vm =
        VM::new(env, db, tx, LevmCallTracer::disabled(), vm_type).map_err(EvmError::from)?;

    let report = vm.execute().map_err(EvmError::from)?;

    if let Some(system_account) = system_account_backup {
        db.current_accounts_state
            .insert(system_address, system_account);
    } else {
        // If the system account was not in the cache, we need to remove it
        db.current_accounts_state.remove(&system_address);
    }

    if let Some(coinbase_account) = coinbase_backup {
        db.current_accounts_state
            .insert(block_header.coinbase, coinbase_account);
    } else {
        // If the coinbase account was not in the cache, we need to remove it
        db.current_accounts_state.remove(&block_header.coinbase);
    }

    Ok(report)
}

#[allow(unreachable_code)]
#[allow(unused_variables)]
pub fn extract_all_requests_levm(
    receipts: &[Receipt],
    db: &mut GeneralizedDatabase,
    header: &BlockHeader,
    vm_type: VMType,
) -> Result<Vec<Requests>, EvmError> {
    if let VMType::L2(_) = vm_type {
        return Err(EvmError::InvalidEVM(
            "extract_all_requests_levm should not be called for L2 VM".to_string(),
        ));
    }

    let chain_config = db.store.get_chain_config()?;
    let fork = chain_config.fork(header.timestamp);

    if fork < Fork::Prague {
        return Ok(Default::default());
    }

    let withdrawals_data: Vec<u8> = LEVM::read_withdrawal_requests(header, db, vm_type)?
        .output
        .into();
    let consolidation_data: Vec<u8> = LEVM::dequeue_consolidation_requests(header, db, vm_type)?
        .output
        .into();

    let deposits = Requests::from_deposit_receipts(chain_config.deposit_contract_address, receipts)
        .ok_or(EvmError::InvalidDepositRequest)?;
    let withdrawals = Requests::from_withdrawals_data(withdrawals_data);
    let consolidation = Requests::from_consolidation_data(consolidation_data);

    Ok(vec![deposits, withdrawals, consolidation])
}

/// Calculating gas_price according to EIP-1559 rules
/// See https://github.com/ethereum/go-ethereum/blob/7ee9a6e89f59cee21b5852f5f6ffa2bcfc05a25f/internal/ethapi/transaction_args.go#L430
pub fn calculate_gas_price_for_generic(tx: &GenericTransaction, basefee: u64) -> U256 {
    if tx.gas_price != 0 {
        // Legacy gas field was specified, use it
        tx.gas_price.into()
    } else {
        // Backfill the legacy gas price for EVM execution, (zero if max_fee_per_gas is zero)
        min(
            tx.max_priority_fee_per_gas.unwrap_or(0) + basefee,
            tx.max_fee_per_gas.unwrap_or(0),
        )
        .into()
    }
}

pub fn calculate_gas_price_for_tx(
    tx: &Transaction,
    mut fee_per_gas: u64,
    vm_type: &VMType,
) -> Result<U256, VMError> {
    let Some(max_priority_fee) = tx.max_priority_fee() else {
        // Legacy transaction
        return Ok(tx.gas_price());
    };

    let max_fee_per_gas = tx.max_fee_per_gas().ok_or(VMError::TxValidation(
        TxValidationError::InsufficientMaxFeePerGas,
    ))?;

    if let VMType::L2(fee_config) = vm_type
        && let Some(operator_fee_config) = &fee_config.operator_fee_config
    {
        fee_per_gas += operator_fee_config.operator_fee_per_gas;
    }

    if fee_per_gas > max_fee_per_gas {
        return Err(VMError::TxValidation(
            TxValidationError::InsufficientMaxFeePerGas,
        ));
    }

    Ok(min(max_priority_fee + fee_per_gas, max_fee_per_gas).into())
}

/// When basefee tracking is disabled  (ie. env.disable_base_fee = true; env.disable_block_gas_limit = true;)
/// and no gas prices were specified, lower the basefee to 0 to avoid breaking EVM invariants (basefee < feecap)
/// See https://github.com/ethereum/go-ethereum/blob/00294e9d28151122e955c7db4344f06724295ec5/core/vm/evm.go#L137
fn adjust_disabled_base_fee(env: &mut Environment) {
    if env.gas_price == U256::zero() {
        env.base_fee_per_gas = U256::zero();
    }
    if env
        .tx_max_fee_per_blob_gas
        .is_some_and(|v| v == U256::zero())
    {
        env.block_excess_blob_gas = None;
    }
}

/// When l2 fees are disabled (ie. env.gas_price = 0), set fee configs to None to avoid breaking failing fee deductions
fn adjust_disabled_l2_fees(env: &Environment, vm_type: VMType) -> VMType {
    if env.gas_price == U256::zero()
        && let VMType::L2(fee_config) = vm_type
    {
        // Don't deduct fees if no gas price is set
        return VMType::L2(FeeConfig {
            operator_fee_config: None,
            l1_fee_config: None,
            ..fee_config
        });
    }
    vm_type
}

fn env_from_generic(
    tx: &GenericTransaction,
    header: &BlockHeader,
    db: &GeneralizedDatabase,
    vm_type: VMType,
) -> Result<Environment, VMError> {
    let chain_config = db.store.get_chain_config()?;
    let gas_price =
        calculate_gas_price_for_generic(tx, header.base_fee_per_gas.unwrap_or(INITIAL_BASE_FEE));
    let block_excess_blob_gas = header.excess_blob_gas.map(U256::from);
    let config = EVMConfig::new_from_chain_config(&chain_config, header);

    // Validate slot_number for Amsterdam+ blocks
    // For L2 chains, slot_number is always 0
    let slot_number = if let VMType::L2(_) = vm_type {
        U256::zero()
    } else if config.fork >= Fork::Amsterdam {
        header
            .slot_number
            .map(U256::from)
            .ok_or(VMError::Internal(InternalError::Custom(
                "slot_number must be present in Amsterdam+ blocks".to_string(),
            )))?
    } else {
        // Pre-Amsterdam: slot_number should be None, default to zero
        // This value should never be used since SLOTNUM opcode doesn't exist pre-Amsterdam
        header.slot_number.map(U256::from).unwrap_or(U256::zero())
    };

    Ok(Environment {
        origin: tx.from.0.into(),
        gas_limit: tx
            .gas
            .unwrap_or(get_max_allowed_gas_limit(header.gas_limit, config.fork)), // Ensure tx doesn't fail due to gas limit
        config,
        block_number: header.number.into(),
        coinbase: header.coinbase,
        timestamp: header.timestamp.into(),
        prev_randao: Some(header.prev_randao),
        slot_number,
        chain_id: chain_config.chain_id.into(),
        base_fee_per_gas: header.base_fee_per_gas.unwrap_or_default().into(),
        base_blob_fee_per_gas: get_base_fee_per_blob_gas(block_excess_blob_gas, &config)?,
        gas_price,
        block_excess_blob_gas,
        block_blob_gas_used: header.blob_gas_used.map(U256::from),
        tx_blob_hashes: tx.blob_versioned_hashes.clone(),
        tx_max_priority_fee_per_gas: tx.max_priority_fee_per_gas.map(U256::from),
        tx_max_fee_per_gas: tx.max_fee_per_gas.map(U256::from),
        tx_max_fee_per_blob_gas: tx.max_fee_per_blob_gas,
        tx_nonce: tx.nonce.unwrap_or_default(),
        block_gas_limit: header.gas_limit,
        difficulty: header.difficulty,
        is_privileged: false,
        is_warming: false,
        fee_token: tx.fee_token,
    })
}

fn vm_from_generic<'a>(
    tx: &GenericTransaction,
    env: Environment,
    db: &'a mut GeneralizedDatabase,
    vm_type: VMType,
) -> Result<VM<'a>, VMError> {
    let tx = match &tx.authorization_list {
        Some(authorization_list) => Transaction::EIP7702Transaction(EIP7702Transaction {
            to: match tx.to {
                TxKind::Call(to) => to,
                TxKind::Create => {
                    return Err(InternalError::msg("Generic Tx cannot be create type").into());
                }
            },
            value: tx.value,
            data: tx.input.clone(),
            access_list: tx
                .access_list
                .iter()
                .map(|list| (list.address, list.storage_keys.clone()))
                .collect(),
            authorization_list: authorization_list
                .iter()
                .map(|auth| Into::<AuthorizationTuple>::into(auth.clone()))
                .collect(),
            ..Default::default()
        }),
        None => Transaction::EIP1559Transaction(EIP1559Transaction {
            to: tx.to.clone(),
            value: tx.value,
            data: tx.input.clone(),
            access_list: tx
                .access_list
                .iter()
                .map(|list| (list.address, list.storage_keys.clone()))
                .collect(),
            ..Default::default()
        }),
    };

    let vm_type = adjust_disabled_l2_fees(&env, vm_type);
    VM::new(env, db, &tx, LevmCallTracer::disabled(), vm_type)
}

pub fn get_max_allowed_gas_limit(block_gas_limit: u64, fork: Fork) -> u64 {
    if fork >= Fork::Osaka {
        POST_OSAKA_GAS_LIMIT_CAP
    } else {
        block_gas_limit
    }
}
