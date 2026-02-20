pub mod db;
mod tracing;

use super::BlockExecutionResult;
use crate::system_contracts::{
    BEACON_ROOTS_ADDRESS, CONSOLIDATION_REQUEST_PREDEPLOY_ADDRESS, HISTORY_STORAGE_ADDRESS,
    PRAGUE_SYSTEM_CONTRACTS, SYSTEM_ADDRESS, WITHDRAWAL_REQUEST_PREDEPLOY_ADDRESS,
};
use crate::{EvmError, ExecutionResult};
use bytes::Bytes;
use ethrex_common::constants::EMPTY_KECCACK_HASH;
use ethrex_common::types::block_access_list::BlockAccessList;
use ethrex_common::types::fee_config::FeeConfig;
use ethrex_common::types::{AuthorizationTuple, EIP7702Transaction};
use ethrex_common::{
    Address, BigEndianHash, U256,
    types::{
        AccessList, AccountUpdate, Block, BlockHeader, EIP1559Transaction, Fork, GWEI_TO_WEI,
        GenericTransaction, INITIAL_BASE_FEE, Receipt, Transaction, TxKind, TxType, Withdrawal,
        requests::Requests,
    },
};
use ethrex_levm::EVMConfig;
use ethrex_levm::call_frame::Stack;
use ethrex_levm::constants::{
    POST_OSAKA_GAS_LIMIT_CAP, STACK_LIMIT, SYS_CALL_GAS_LIMIT, TX_BASE_COST,
};
use ethrex_levm::db::Database;
use ethrex_levm::db::gen_db::{CacheDB, GeneralizedDatabase};
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
use rayon::iter::{IntoParallelIterator, IntoParallelRefIterator, ParallelIterator};
use rustc_hash::{FxHashMap, FxHashSet};
use std::cmp::min;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::Sender;

/// Builds groups of transaction indices that can execute in parallel.
/// Uses the BAL write-set information to detect conflicts.
/// Transactions from the same sender are chained into the same group to preserve nonce order.
/// Coinbase is excluded from conflict detection (every tx writes it; treating it as a conflict
/// would force all txs sequential).
fn build_parallel_groups(
    bal: &BlockAccessList,
    txs_with_sender: &[(&Transaction, Address)],
    coinbase: Address,
) -> Vec<Vec<usize>> {
    let n = txs_with_sender.len();
    if n == 0 {
        return Vec::new();
    }

    // Build write sets per tx (BAL uses 1-indexed, we convert to 0-indexed)
    let mut writes: Vec<FxHashSet<Address>> = (0..n).map(|_| FxHashSet::default()).collect();
    for account in bal.accounts() {
        if account.address == coinbase {
            continue;
        }
        let addr = account.address;
        for change in &account.balance_changes {
            let idx = change.block_access_index as usize;
            if idx >= 1 && idx <= n {
                writes[idx - 1].insert(addr);
            }
        }
        for change in &account.nonce_changes {
            let idx = change.block_access_index as usize;
            if idx >= 1 && idx <= n {
                writes[idx - 1].insert(addr);
            }
        }
        for change in &account.code_changes {
            let idx = change.block_access_index as usize;
            if idx >= 1 && idx <= n {
                writes[idx - 1].insert(addr);
            }
        }
        for slot_change in &account.storage_changes {
            for sc in &slot_change.slot_changes {
                let idx = sc.block_access_index as usize;
                if idx >= 1 && idx <= n {
                    writes[idx - 1].insert(addr);
                }
            }
        }
    }

    // Build sender chains: same-sender txs must execute sequentially in the same group
    let mut chain: Vec<Option<usize>> = vec![None; n];
    let mut sender_last: FxHashMap<Address, usize> = FxHashMap::default();
    for (i, (_, sender)) in txs_with_sender.iter().enumerate() {
        if let Some(&prev) = sender_last.get(sender) {
            chain[i] = Some(prev);
        }
        sender_last.insert(*sender, i);
    }

    // Greedy group assignment
    let mut tx_group: Vec<usize> = vec![0; n];
    let mut group_write_sets: Vec<FxHashSet<Address>> = Vec::new();

    for i in 0..n {
        let group_idx = if let Some(prev) = chain[i] {
            // Same sender: must go in same group as previous tx from this sender
            tx_group[prev]
        } else {
            // Find first group that conflicts with this tx's write set.
            // Conflicting txs must execute sequentially → same group.
            // If no group conflicts, this tx is independent → new parallel group.
            match group_write_sets
                .iter()
                .position(|gws| !gws.is_disjoint(&writes[i]))
            {
                Some(idx) => idx,
                None => {
                    group_write_sets.push(FxHashSet::default());
                    group_write_sets.len() - 1
                }
            }
        };
        tx_group[i] = group_idx;
        // Extend the group's write set with this tx's writes
        // (ensures future txs that conflict won't join this group)
        if group_idx < group_write_sets.len() {
            group_write_sets[group_idx].extend(writes[i].iter().copied());
        } else {
            let mut new_set = FxHashSet::default();
            new_set.extend(writes[i].iter().copied());
            group_write_sets.push(new_set);
        }
    }

    // Reconstruct groups in original tx order
    let num_groups = group_write_sets.len();
    let mut groups: Vec<Vec<usize>> = vec![Vec::new(); num_groups];
    for i in 0..n {
        groups[tx_group[i]].push(i);
    }
    groups.retain(|g| !g.is_empty());
    groups
}

/// The struct implements the following functions:
/// [LEVM::execute_block]
/// [LEVM::execute_tx]
/// [LEVM::get_state_transitions]
/// [LEVM::process_withdrawals]
#[derive(Debug)]
pub struct LEVM;

/// Checks that adding `tx_gas_limit` to `block_gas_used` doesn't exceed `block_gas_limit`.
/// NOTE: Message must contain "Gas allowance exceeded" and "Block gas used overflow"
/// as literal substrings for the EELS exception mapper (see execution-specs ethrex.py).
/// Can be simplified once we update the mapper regexes.
fn check_gas_limit(
    block_gas_used: u64,
    tx_gas_limit: u64,
    block_gas_limit: u64,
) -> Result<(), EvmError> {
    if block_gas_used + tx_gas_limit > block_gas_limit {
        return Err(EvmError::Transaction(format!(
            "Gas allowance exceeded: Block gas used overflow: \
             used {block_gas_used} + tx limit {tx_gas_limit} > block limit {block_gas_limit}"
        )));
    }
    Ok(())
}

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
            check_gas_limit(block_gas_used, tx.gas_limit(), block.header.gas_limit)?;

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
        header_bal: Option<&BlockAccessList>,
    ) -> Result<(BlockExecutionResult, Option<BlockAccessList>), EvmError> {
        let chain_config = db.store.get_chain_config()?;
        let is_amsterdam = chain_config.is_amsterdam_activated(block.header.timestamp);

        let transactions_with_sender =
            block.body.get_transactions_with_sender().map_err(|error| {
                EvmError::Transaction(format!("Couldn't recover addresses with error: {error}"))
            })?;

        // When BAL is provided (Amsterdam+ validation path): use parallel execution
        if let Some(bal) = header_bal {
            // No BAL recording needed: we have the header BAL, not building a new one
            Self::prepare_block(block, db, vm_type)?;

            // Drain system call state changes and snapshot for group db seeding
            let sys_updates = LEVM::get_state_transitions_tx(db)?;
            let system_seed = db.initial_accounts_state.clone();

            let (receipts, block_gas_used) = Self::execute_block_parallel(
                block,
                &transactions_with_sender,
                db,
                vm_type,
                bal,
                block.header.coinbase,
                &merkleizer,
                queue_length,
                sys_updates,
                system_seed,
            )?;

            // Withdrawals (sequential, on main db)
            if let Some(withdrawals) = &block.body.withdrawals {
                Self::process_withdrawals(db, withdrawals)?;
            }

            let requests = match vm_type {
                VMType::L1 => extract_all_requests_levm(&receipts, db, &block.header, vm_type)?,
                VMType::L2(_) => Default::default(),
            };
            LEVM::send_state_transitions_tx(&merkleizer, db, queue_length)?;

            // BAL is not recorded in parallel path (header BAL is trusted)
            return Ok((
                BlockExecutionResult {
                    receipts,
                    requests,
                    block_gas_used,
                },
                None,
            ));
        }

        // Sequential path (existing code, for block production and non-Amsterdam)
        if is_amsterdam {
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

        for (tx_idx, (tx, tx_sender)) in transactions_with_sender.into_iter().enumerate() {
            check_gas_limit(block_gas_used, tx.gas_limit(), block.header.gas_limit)?;

            // Set BAL index for this transaction (1-indexed per EIP-7928, uint16)
            if is_amsterdam {
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
        if is_amsterdam {
            #[allow(clippy::cast_possible_truncation)]
            let withdrawal_index = (block.body.transactions.len() + 1) as u16;
            db.set_bal_index(withdrawal_index);
        }

        if let Some(withdrawals) = &block.body.withdrawals {
            // Record ALL withdrawal recipients for BAL per EIP-7928
            if is_amsterdam && let Some(recorder) = db.bal_recorder_mut() {
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

    /// Run all txs sequentially on `db` (must be in post-system-call state).
    /// Returns receipts, block gas used, and the merged AccountUpdates for all txs.
    /// Used as a fallback from the parallel path when RAW conflicts are detected.
    fn execute_txs_sequential(
        block: &Block,
        txs_with_sender: &[(&Transaction, Address)],
        db: &mut GeneralizedDatabase,
        vm_type: VMType,
    ) -> Result<(Vec<Receipt>, u64, Vec<AccountUpdate>), EvmError> {
        let header = &block.header;
        let mut stack_pool = Vec::with_capacity(STACK_LIMIT);
        let mut all_updates = Vec::new();
        let mut receipts = Vec::new();
        let mut cumulative_gas = 0u64;
        let mut block_gas = 0u64;
        for (tx, sender) in txs_with_sender {
            let report =
                LEVM::execute_tx_in_block(tx, *sender, header, db, vm_type, &mut stack_pool)?;
            let updates = LEVM::get_state_transitions_tx(db)?;
            all_updates.extend(updates);
            cumulative_gas += report.gas_spent;
            block_gas += report.gas_used;
            let receipt = Receipt::new(
                tx.tx_type(),
                matches!(report.result, TxResult::Success),
                cumulative_gas,
                report.logs,
            );
            receipts.push(receipt);
        }
        Ok((receipts, block_gas, all_updates))
    }

    /// Execute block transactions in parallel using BAL conflict graph.
    /// Only called for Amsterdam+ blocks when the header BAL is available.
    ///
    /// Groups are built from the BAL write-set. Each group executes sequentially
    /// on its own GeneralizedDatabase seeded with post-system-call state.
    /// Coinbase gas deltas are collected and applied to main db after merge.
    #[allow(clippy::too_many_arguments)]
    fn execute_block_parallel<'blk>(
        block: &'blk Block,
        txs_with_sender: &[(&'blk Transaction, Address)],
        db: &mut GeneralizedDatabase,
        vm_type: VMType,
        bal: &BlockAccessList,
        coinbase: Address,
        merkleizer: &Sender<Vec<AccountUpdate>>,
        queue_length: &AtomicUsize,
        sys_updates: Vec<AccountUpdate>,
        system_seed: CacheDB,
    ) -> Result<(Vec<Receipt>, u64), EvmError> {
        // Send system call updates to merkleizer first
        merkleizer
            .send(sys_updates)
            .map_err(|e| EvmError::Custom(format!("merkleizer send failed: {e}")))?;
        queue_length.fetch_add(1, Ordering::Relaxed);

        // Snapshot coinbase balance after system calls (before any tx gas)
        let coinbase_initial_balance = db
            .get_account(coinbase)
            .map_err(|e| EvmError::Custom(format!("failed to load coinbase: {e}")))?
            .info
            .balance;

        let groups = build_parallel_groups(bal, txs_with_sender, coinbase);

        let store = db.store.clone();
        let header = &block.header;

        // GroupResult: per-tx results + final coinbase balance + read set for RAW detection.
        type GroupResult = (
            Vec<(usize, TxType, ExecutionReport, Vec<AccountUpdate>)>,
            U256,              // final coinbase balance in this group
            FxHashSet<Address>, // addresses read from initial/store (RAW conflict detection)
        );
        // Execute each group in parallel; within each group txs are sequential
        let all_results: Result<Vec<GroupResult>, EvmError> = groups
            .into_par_iter()
            .map(|group| -> Result<_, EvmError> {
                let mut group_db = GeneralizedDatabase::new(store.clone());
                // Seed with post-system-call state so group txs see updated system contract state
                group_db
                    .initial_accounts_state
                    .extend(system_seed.iter().map(|(a, ac)| (*a, ac.clone())));
                // Enable read tracking for RAW conflict detection
                group_db.reads = Some(FxHashSet::default());
                let mut stack_pool = Vec::with_capacity(STACK_LIMIT);
                let mut per_tx = Vec::new();
                for &tx_idx in &group {
                    let (tx, sender) = &txs_with_sender[tx_idx];
                    let report = LEVM::execute_tx_in_block(
                        tx,
                        *sender,
                        header,
                        &mut group_db,
                        vm_type,
                        &mut stack_pool,
                    )?;
                    // Drain current state into initial state so next tx in group sees updated state
                    let updates = LEVM::get_state_transitions_tx(&mut group_db)?;
                    per_tx.push((tx_idx, tx.tx_type(), report, updates));
                }
                // Read final coinbase balance once per group (after all txs have been drained).
                // This avoids double-counting: each get_state_transitions_tx promotes the
                // coinbase to initial_accounts_state, so per-tx updates show an accumulated
                // absolute balance, not an incremental delta.
                let final_coinbase = group_db
                    .initial_accounts_state
                    .get(&coinbase)
                    .map(|a| a.info.balance)
                    .unwrap_or(coinbase_initial_balance);
                let reads = group_db.reads.take().unwrap_or_default();
                Ok((per_tx, final_coinbase, reads))
            })
            .collect();

        let all_results = all_results?;

        // RAW conflict detection: check if any group reads an address that another group writes.
        // If so, the parallel results would be incorrect — fall back to sequential execution.
        if all_results.len() > 1 {
            let write_sets: Vec<FxHashSet<Address>> = all_results
                .iter()
                .map(|(group_txs, _, _)| {
                    group_txs
                        .iter()
                        .flat_map(|(_, _, _, updates)| updates.iter().map(|u| u.address))
                        .filter(|&a| a != coinbase)
                        .collect()
                })
                .collect();

            let has_raw_conflict = all_results.iter().enumerate().any(|(i, (_, _, reads))| {
                write_sets
                    .iter()
                    .enumerate()
                    .any(|(j, writes)| i != j && !reads.is_disjoint(writes))
            });

            if has_raw_conflict {
                // Discard parallel results; re-run all txs sequentially on the main db
                // (which is already in post-system-call state, current_accounts_state empty)
                let (receipts, block_gas, seq_updates) =
                    Self::execute_txs_sequential(block, txs_with_sender, db, vm_type)?;
                merkleizer
                    .send(seq_updates)
                    .map_err(|e| EvmError::Custom(format!("merkleizer send failed: {e}")))?;
                queue_length.fetch_add(1, Ordering::Relaxed);
                return Ok((receipts, block_gas));
            }
        }

        // No RAW conflicts — merge all AccountUpdates; accumulate per-group coinbase deltas
        let mut indexed_reports: Vec<(usize, TxType, ExecutionReport)> = Vec::new();
        let mut merged: FxHashMap<Address, AccountUpdate> = FxHashMap::default();
        let mut coinbase_delta = U256::zero();

        for (group_txs, final_coinbase, _reads) in all_results {
            // One delta per group, computed from the final coinbase balance in that group
            coinbase_delta += final_coinbase.saturating_sub(coinbase_initial_balance);
            for (tx_idx, tx_type, report, updates) in group_txs {
                indexed_reports.push((tx_idx, tx_type, report));
                for update in updates {
                    if update.address == coinbase {
                        // Skip per-tx coinbase updates; handled via per-group delta above
                        continue;
                    }
                    merged
                        .entry(update.address)
                        .and_modify(|e| e.merge(update.clone()))
                        .or_insert(update);
                }
            }
        }

        // Apply total coinbase delta to main db and extract its AccountUpdate
        if !coinbase_delta.is_zero() {
            let coinbase_account = db
                .get_account_mut(coinbase)
                .map_err(|e| EvmError::Custom(format!("failed to load coinbase for delta: {e}")))?;
            coinbase_account.info.balance += coinbase_delta;
        }
        // Extract coinbase update (and any other updates on main db)
        let main_updates = LEVM::get_state_transitions_tx(db)?;
        for update in main_updates {
            merged
                .entry(update.address)
                .and_modify(|e| e.merge(update.clone()))
                .or_insert(update);
        }

        // Send merged tx + coinbase updates to merkleizer
        merkleizer
            .send(merged.into_values().collect())
            .map_err(|e| EvmError::Custom(format!("merkleizer send failed: {e}")))?;
        queue_length.fetch_add(1, Ordering::Relaxed);

        // Sort by tx_idx and reconstruct receipts in block order
        indexed_reports.sort_unstable_by_key(|(idx, _, _)| *idx);

        let mut receipts = Vec::with_capacity(indexed_reports.len());
        let mut cumulative_gas_used = 0_u64;
        let mut block_gas_used = 0_u64;

        for (_, tx_type, report) in indexed_reports {
            cumulative_gas_used += report.gas_spent;
            block_gas_used += report.gas_used;
            let receipt = Receipt::new(
                tx_type,
                matches!(report.result, TxResult::Success),
                cumulative_gas_used,
                report.logs,
            );
            receipts.push(receipt);
        }

        Ok((receipts, block_gas_used))
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

    /// Pre-warms state by loading all accounts and storage slots listed in the
    /// Block Access List directly, without speculative re-execution.
    ///
    /// Two-phase approach:
    /// - Phase 1: Load all account states (parallel via rayon) -> warms CachingDatabase
    ///   account cache AND trie layer cache nodes
    /// - Phase 2: Load all storage slots (parallel via rayon, per-slot) + contract code
    ///   (parallel via rayon, per-account) -> benefits from trie nodes cached in Phase 1
    pub fn warm_block_from_bal(
        bal: &BlockAccessList,
        store: Arc<dyn Database>,
    ) -> Result<(), EvmError> {
        let accounts = bal.accounts();
        if accounts.is_empty() {
            return Ok(());
        }

        // Phase 1: Prefetch all account states in parallel.
        // This warms the CachingDatabase account cache and the TrieLayerCache
        // with state trie nodes, so Phase 2 storage reads benefit from cached lookups.
        accounts.par_iter().for_each(|ac| {
            let _ = store.get_account_state(ac.address);
        });

        // Phase 2: Prefetch storage slots and contract code in parallel.
        // Storage is flattened to (address, slot) pairs so rayon can distribute
        // work across threads regardless of how many slots each account has.
        // Without flattening, a hot contract with hundreds of slots (e.g. a DEX
        // pool) would monopolize a single thread while others go idle.
        let slots: Vec<(ethrex_common::Address, ethrex_common::H256)> = accounts
            .iter()
            .flat_map(|ac| {
                ac.all_storage_slots()
                    .map(move |slot| (ac.address, ethrex_common::H256::from_uint(&slot)))
            })
            .collect();
        slots.par_iter().for_each(|(addr, key)| {
            let _ = store.get_storage_value(*addr, *key);
        });

        // Code prefetch: get_account_state is a cache hit from Phase 1
        accounts.par_iter().for_each(|ac| {
            if let Ok(acct) = store.get_account_state(ac.address)
                && acct.code_hash != *EMPTY_KECCACK_HASH
            {
                let _ = store.get_account_code(acct.code_hash);
            }
        });

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
        let env = Self::setup_env(tx, tx_sender, block_header, db, vm_type)?;
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
    ) -> Result<ExecutionReport, EvmError> {
        let env = Self::setup_env(tx, tx_sender, block_header, db, vm_type)?;
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
    // EIP-7928: Mark BAL recorder as in system call mode to filter SYSTEM_ADDRESS changes
    if let Some(recorder) = db.bal_recorder.as_mut() {
        recorder.enter_system_call();
    }

    let result = VM::new(env, db, tx, LevmCallTracer::disabled(), vm_type)
        .and_then(|mut vm| vm.execute())
        .map_err(EvmError::from);

    // EIP-7928: Exit system call mode before restoring accounts (must run even on error)
    if let Some(recorder) = db.bal_recorder.as_mut() {
        recorder.exit_system_call();
    }

    let report = result?;

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

#[cfg(test)]
mod parallel_group_tests {
    use super::*;
    use ethrex_common::types::block_access_list::{AccountChanges, BalanceChange};

    fn addr(byte: u8) -> Address {
        let mut a = Address::zero();
        a.0[19] = byte;
        a
    }

    /// Build a BAL where the given (1-indexed tx index, address) pairs mark balance writes.
    fn bal_writes(entries: &[(u16, Address)]) -> BlockAccessList {
        let mut by_addr: FxHashMap<Address, Vec<u16>> = FxHashMap::default();
        for &(idx, address) in entries {
            by_addr.entry(address).or_default().push(idx);
        }
        let accounts = by_addr
            .into_iter()
            .map(|(address, indices)| {
                let balance_changes = indices
                    .into_iter()
                    .map(|idx| BalanceChange {
                        block_access_index: idx,
                        post_balance: U256::zero(),
                    })
                    .collect();
                AccountChanges::new(address).with_balance_changes(balance_changes)
            })
            .collect();
        BlockAccessList::from_accounts(accounts)
    }

    fn dummy_tx() -> Transaction {
        Transaction::EIP1559Transaction(EIP1559Transaction::default())
    }

    #[test]
    fn test_empty_block() {
        let bal = BlockAccessList::new();
        let txs: Vec<(&Transaction, Address)> = vec![];
        let groups = build_parallel_groups(&bal, &txs, addr(0xff));
        assert!(groups.is_empty());
    }

    #[test]
    fn test_single_tx() {
        let bal = BlockAccessList::new();
        let tx = dummy_tx();
        let groups = build_parallel_groups(&bal, &[(&tx, addr(1))], addr(0xff));
        assert_eq!(groups, vec![vec![0usize]]);
    }

    #[test]
    fn test_same_sender_preserves_order() {
        // All txs from the same sender → one group, indices in original order.
        let bal = BlockAccessList::new();
        let tx = dummy_tx();
        let sender = addr(1);
        let txs = vec![(&tx, sender), (&tx, sender), (&tx, sender)];
        let groups = build_parallel_groups(&bal, &txs, addr(0xff));
        assert_eq!(groups, vec![vec![0, 1, 2]]);
    }

    #[test]
    fn test_non_conflicting_txs_get_separate_groups() {
        // tx0 writes addr_a, tx1 writes addr_b (disjoint write sets → no conflict).
        // Non-conflicting txs each get their own group so they can run in parallel.
        let addr_a = addr(1);
        let addr_b = addr(2);
        let bal = bal_writes(&[(1, addr_a), (2, addr_b)]);
        let tx = dummy_tx();
        let txs = vec![(&tx, addr(10)), (&tx, addr(11))];
        let groups = build_parallel_groups(&bal, &txs, addr(0xff));
        assert_eq!(groups, vec![vec![0], vec![1]]);
    }

    #[test]
    fn test_conflicting_txs_serialized_in_same_group() {
        // tx0 and tx1 both write addr_a → conflict → placed in same group (serialized).
        let addr_a = addr(1);
        let bal = bal_writes(&[(1, addr_a), (2, addr_a)]);
        let tx = dummy_tx();
        let txs = vec![(&tx, addr(10)), (&tx, addr(11))];
        let groups = build_parallel_groups(&bal, &txs, addr(0xff));
        assert_eq!(groups, vec![vec![0, 1]]);
    }

    #[test]
    fn test_coinbase_writes_do_not_cause_conflict() {
        // Both txs write only to coinbase → coinbase excluded from write sets → no conflict.
        // Empty write sets are disjoint → each tx gets its own parallel group.
        let coinbase = addr(0xff);
        let bal = bal_writes(&[(1, coinbase), (2, coinbase)]);
        let tx = dummy_tx();
        let txs = vec![(&tx, addr(10)), (&tx, addr(11))];
        let groups = build_parallel_groups(&bal, &txs, coinbase);
        assert_eq!(groups, vec![vec![0], vec![1]]);
    }

    #[test]
    fn test_conflict_graph_three_txs() {
        // tx0 (A) writes {X}
        // tx1 (B) writes {X, Y}  — conflicts with A on X → same group as A
        // tx2 (C) writes {Y}    — conflicts with group 0 on Y → same group
        //
        // All three conflict transitively → one sequential group.
        let addr_x = addr(1);
        let addr_y = addr(2);
        let bal = bal_writes(&[
            (1, addr_x),
            (2, addr_x),
            (2, addr_y),
            (3, addr_y),
        ]);
        let tx = dummy_tx();
        let txs = vec![
            (&tx, addr(10)), // A  (tx_idx 0)
            (&tx, addr(11)), // B  (tx_idx 1)
            (&tx, addr(12)), // C  (tx_idx 2)
        ];
        let groups = build_parallel_groups(&bal, &txs, addr(0xff));
        assert_eq!(groups, vec![vec![0, 1, 2]]);
    }

    #[test]
    fn test_three_independent_txs_all_parallel() {
        // Three txs each writing unique addresses → no conflicts → each gets its own parallel group.
        let bal = bal_writes(&[(1, addr(1)), (2, addr(2)), (3, addr(3))]);
        let tx = dummy_tx();
        let txs = vec![(&tx, addr(10)), (&tx, addr(11)), (&tx, addr(12))];
        let groups = build_parallel_groups(&bal, &txs, addr(0xff));
        assert_eq!(groups, vec![vec![0], vec![1], vec![2]]);
    }

    #[test]
    fn test_all_conflicting_serialized_in_one_group() {
        // Every tx writes the same address → all conflict → all serialized in one group.
        let addr_a = addr(1);
        let bal = bal_writes(&[(1, addr_a), (2, addr_a), (3, addr_a)]);
        let tx = dummy_tx();
        let txs = vec![(&tx, addr(10)), (&tx, addr(11)), (&tx, addr(12))];
        let groups = build_parallel_groups(&bal, &txs, addr(0xff));
        assert_eq!(groups, vec![vec![0, 1, 2]]);
    }

    #[test]
    fn test_mixed_sender_chains_and_conflicts() {
        // tx0 (sender A) writes X
        // tx1 (sender A) — same sender as tx0, chained into tx0's group
        // tx2 (sender B) writes X — conflicts with group 0 on X → also joins group 0
        //
        // All three end up in the same sequential group.
        let addr_x = addr(1);
        let bal = bal_writes(&[(1, addr_x), (3, addr_x)]);
        let tx = dummy_tx();
        let sender_a = addr(10);
        let sender_b = addr(11);
        let txs = vec![
            (&tx, sender_a), // tx0
            (&tx, sender_a), // tx1 — chained with tx0
            (&tx, sender_b), // tx2 — different sender, conflicts on X → serialized with group 0
        ];
        let groups = build_parallel_groups(&bal, &txs, addr(0xff));
        assert_eq!(groups, vec![vec![0, 1, 2]]);
    }
}
