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
use ethrex_common::types::block_access_list::{
    BalAddressIndex, BlockAccessList, find_exact_change_balance, find_exact_change_code,
    find_exact_change_nonce, find_exact_change_storage, has_exact_change_balance,
    has_exact_change_code, has_exact_change_nonce, has_exact_change_storage,
};
use ethrex_common::types::fee_config::FeeConfig;
use ethrex_common::types::{AuthorizationTuple, Code, EIP7702Transaction};
use ethrex_common::{
    Address, BigEndianHash, H256, U256,
    types::{
        AccessList, AccountUpdate, Block, BlockHeader, EIP1559Transaction, Fork, GWEI_TO_WEI,
        GenericTransaction, INITIAL_BASE_FEE, Receipt, Transaction, TxKind, TxType, Withdrawal,
        requests::Requests,
    },
};
use ethrex_levm::EVMConfig;
use ethrex_levm::account::{AccountStatus, LevmAccount};
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

            // Drain system call state and snapshot for per-tx db seeding
            LEVM::get_state_transitions_tx(db)?;
            let system_seed = Arc::new(std::mem::take(&mut db.initial_accounts_state));

            let (receipts, block_gas_used) = Self::execute_block_parallel(
                block,
                &transactions_with_sender,
                db,
                vm_type,
                bal,
                &merkleizer,
                queue_length,
                system_seed,
            )?;

            // Seed main db with post-tx state (excluding withdrawal effects) so
            // request extraction system calls see user-queued requests on predeploys.
            // Withdrawal index is n_txs+1 in BAL; we use n_txs to avoid double-applying
            // withdrawal balances (process_withdrawals handles those below).
            #[allow(clippy::cast_possible_truncation)]
            let last_tx_idx = block.body.transactions.len() as u16;
            Self::seed_db_from_bal(db, bal, last_tx_idx)?;

            // Withdrawals apply on top of seeded state; requests read predeploy storage
            if let Some(withdrawals) = &block.body.withdrawals {
                Self::process_withdrawals(db, withdrawals)?;
            }

            let requests = match vm_type {
                VMType::L1 => extract_all_requests_levm(&receipts, db, &block.header, vm_type)?,
                VMType::L2(_) => Default::default(),
            };
            // State transitions for merkleizer come from bal_to_account_updates,
            // not from db — no need to call send_state_transitions_tx here.

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

    /// Convert BAL into `Vec<AccountUpdate>` for the merkleizer.
    ///
    /// For each account in the BAL, extracts the **final** post-block state
    /// (highest `block_access_index` entry per field) and builds an AccountUpdate.
    /// State comes entirely from the BAL — no execution needed.
    fn bal_to_account_updates(
        bal: &BlockAccessList,
        store: &dyn Database,
    ) -> Result<Vec<AccountUpdate>, EvmError> {
        use ethrex_common::types::AccountInfo;

        let mut updates = Vec::new();

        // Batch prefetch all accounts with writes so per-account lookups are cache hits
        let write_addrs: Vec<Address> = bal
            .accounts()
            .iter()
            .filter(|ac| {
                !ac.balance_changes.is_empty()
                    || !ac.nonce_changes.is_empty()
                    || !ac.code_changes.is_empty()
                    || !ac.storage_changes.is_empty()
            })
            .map(|ac| ac.address)
            .collect();
        store
            .prefetch_accounts(&write_addrs)
            .map_err(|e| EvmError::Custom(format!("bal_to_account_updates prefetch: {e}")))?;

        for acct_changes in bal.accounts() {
            let addr = acct_changes.address;

            // Skip accounts with only reads and no writes
            let has_writes = !acct_changes.balance_changes.is_empty()
                || !acct_changes.nonce_changes.is_empty()
                || !acct_changes.code_changes.is_empty()
                || !acct_changes.storage_changes.is_empty();
            if !has_writes {
                continue;
            }

            // Load pre-state for unchanged fields (cache hit after prefetch)
            let prestate = store
                .get_account_state(addr)
                .map_err(|e| EvmError::Custom(format!("bal_to_account_updates: {e}")))?;

            // Final balance: last entry (highest index) or prestate
            let balance = acct_changes
                .balance_changes
                .last()
                .map(|c| c.post_balance)
                .unwrap_or(prestate.balance);

            // Final nonce: last entry or prestate
            let nonce = acct_changes
                .nonce_changes
                .last()
                .map(|c| c.post_nonce)
                .unwrap_or(prestate.nonce);

            // Final code: last entry or prestate
            let (code_hash, code) = if let Some(c) = acct_changes.code_changes.last() {
                if c.new_code.is_empty() {
                    (*EMPTY_KECCACK_HASH, None)
                } else {
                    use ethrex_common::types::Code;
                    let code_obj = Code::from_bytecode(c.new_code.clone());
                    let hash = code_obj.hash;
                    (hash, Some(code_obj))
                }
            } else {
                (prestate.code_hash, None)
            };

            // Storage: per slot, last entry (highest index)
            let mut added_storage = FxHashMap::with_capacity_and_hasher(
                acct_changes.storage_changes.len(),
                Default::default(),
            );
            for slot_change in &acct_changes.storage_changes {
                if let Some(last) = slot_change.slot_changes.last() {
                    let key = ethrex_common::utils::u256_to_h256(slot_change.slot);
                    added_storage.insert(key, last.post_value);
                }
            }

            // Detect account removal (EIP-161): post-state empty but pre-state existed
            let post_empty = balance.is_zero() && nonce == 0 && code_hash == *EMPTY_KECCACK_HASH;
            let pre_empty = prestate.balance.is_zero()
                && prestate.nonce == 0
                && prestate.code_hash == *EMPTY_KECCACK_HASH;
            let removed = post_empty && !pre_empty;

            let balance_changed = acct_changes
                .balance_changes
                .last()
                .is_some_and(|c| c.post_balance != prestate.balance);
            let nonce_changed = acct_changes
                .nonce_changes
                .last()
                .is_some_and(|c| c.post_nonce != prestate.nonce);
            let code_changed = acct_changes.code_changes.last().is_some();
            let acc_info_updated = balance_changed || nonce_changed || code_changed;

            if !removed && !acc_info_updated && added_storage.is_empty() {
                continue;
            }

            let info = if acc_info_updated {
                Some(AccountInfo {
                    code_hash,
                    balance,
                    nonce,
                })
            } else {
                None
            };

            let update = AccountUpdate {
                address: addr,
                removed,
                info,
                code,
                added_storage,
                removed_storage: false, // EIP-6780: SELFDESTRUCT only same-tx
            };
            updates.push(update);
        }

        Ok(updates)
    }

    /// Pre-seed a GeneralizedDatabase with BAL-derived state for a specific tx.
    ///
    /// For each BAL-modified account, applies accumulated diffs with
    /// `block_access_index <= max_idx` on top of the loaded pre-block state.
    /// This matches geth's approach: each parallel tx sees the state as if
    /// all previous txs had already executed (via BAL intermediate values).
    ///
    /// `max_idx` is the BAL block_access_index of the last tx whose effects
    /// should be visible. BAL indexing: 0 = system calls, 1 = tx 0, 2 = tx 1, ...
    /// For tx at index `i`, pass `max_idx = i` (diffs with index <= i = system + txs 0..i-1).
    fn seed_db_from_bal(
        db: &mut GeneralizedDatabase,
        bal: &BlockAccessList,
        max_idx: u16,
    ) -> Result<(), EvmError> {
        for acct_changes in bal.accounts() {
            let addr = acct_changes.address;

            // Binary search (slices are sorted ascending by block_access_index):
            // partition_point returns the number of elements <= max_idx.
            let balance_pos = acct_changes
                .balance_changes
                .partition_point(|c| c.block_access_index <= max_idx);
            let nonce_pos = acct_changes
                .nonce_changes
                .partition_point(|c| c.block_access_index <= max_idx);
            let code_pos = acct_changes
                .code_changes
                .partition_point(|c| c.block_access_index <= max_idx);
            // Each slot's slot_changes are sorted ascending by block_access_index,
            // so if the first entry is <= max_idx, at least one change is in scope.
            let any_storage = acct_changes.storage_changes.iter().any(|sc| {
                sc.slot_changes
                    .first()
                    .is_some_and(|c| c.block_access_index <= max_idx)
            });

            if balance_pos == 0 && nonce_pos == 0 && !any_storage && code_pos == 0 {
                continue;
            }

            // Compute code update before borrowing acc (borrow checker: can't access
            // db.codes while acc holds a mutable borrow of db)
            let code_update = if code_pos > 0 {
                let last = &acct_changes.code_changes[code_pos - 1];
                if last.new_code.is_empty() {
                    Some((*EMPTY_KECCACK_HASH, None))
                } else {
                    use ethrex_common::types::Code;
                    let code_obj = Code::from_bytecode(last.new_code.clone());
                    Some((code_obj.hash, Some(code_obj)))
                }
            } else {
                None
            };

            // When BAL covers all account info fields (balance + nonce + code), insert
            // a default LevmAccount directly to skip the store/shared_base lookup.
            // For partial coverage, load from store to fill missing fields.
            let has_all_info = balance_pos > 0 && nonce_pos > 0 && code_pos > 0;
            if has_all_info {
                use ethrex_common::types::AccountInfo;
                let balance = acct_changes.balance_changes[balance_pos - 1].post_balance;
                let nonce = acct_changes.nonce_changes[nonce_pos - 1].post_nonce;
                let code_hash = code_update
                    .as_ref()
                    .map(|(h, _)| *h)
                    .unwrap_or(*EMPTY_KECCACK_HASH);
                // NOTE: has_storage is false for newly inserted accounts. This is safe
                // because this DB is only used for the parallel execution path (state
                // comes from BAL, not get_state_transitions_tx). Do not reuse this DB
                // for sequential fallback without fixing has_storage.
                let acc = db
                    .current_accounts_state
                    .entry(addr)
                    .or_insert_with(|| LevmAccount {
                        info: AccountInfo::default(),
                        storage: FxHashMap::default(),
                        has_storage: false,
                        status: AccountStatus::Modified,
                    });
                acc.info.balance = balance;
                acc.info.nonce = nonce;
                acc.info.code_hash = code_hash;
                acc.mark_modified();
            } else {
                // Partial BAL coverage — load from store/shared_base, then overwrite
                // the covered fields. get_account already caches, so get_account_mut
                // will be a cache hit.
                db.get_account(addr)
                    .map_err(|e| EvmError::Custom(format!("seed_db_from_bal load: {e}")))?;
                let acc = db
                    .get_account_mut(addr)
                    .map_err(|e| EvmError::Custom(format!("seed bal: {e}")))?;

                if balance_pos > 0 {
                    acc.info.balance = acct_changes.balance_changes[balance_pos - 1].post_balance;
                }
                if nonce_pos > 0 {
                    acc.info.nonce = acct_changes.nonce_changes[nonce_pos - 1].post_nonce;
                }
                if let Some((hash, _)) = &code_update {
                    acc.info.code_hash = *hash;
                }
            }

            // Apply storage changes (works for both paths since acc is now in current_accounts_state)
            if any_storage {
                let acc = db
                    .current_accounts_state
                    .get_mut(&addr)
                    .expect("account was just inserted");
                for sc in &acct_changes.storage_changes {
                    let pos = sc
                        .slot_changes
                        .partition_point(|c| c.block_access_index <= max_idx);
                    if pos > 0 {
                        let key = ethrex_common::utils::u256_to_h256(sc.slot);
                        acc.storage.insert(key, sc.slot_changes[pos - 1].post_value);
                    }
                }
            }

            // Insert code object after acc borrow is released
            if let Some((hash, Some(code_obj))) = code_update {
                db.codes.entry(hash).or_insert(code_obj);
            }
        }
        Ok(())
    }

    /// Execute block transactions in parallel using BAL-derived state.
    /// Only called for Amsterdam+ blocks when the header BAL is available.
    ///
    /// Each tx runs independently on its own database pre-seeded with BAL
    /// intermediate state (geth-style). State for the merkleizer comes from
    /// `bal_to_account_updates`, not from tx execution.
    #[allow(clippy::too_many_arguments)]
    fn execute_block_parallel(
        block: &Block,
        txs_with_sender: &[(&Transaction, Address)],
        db: &mut GeneralizedDatabase,
        vm_type: VMType,
        bal: &BlockAccessList,
        merkleizer: &Sender<Vec<AccountUpdate>>,
        queue_length: &AtomicUsize,
        system_seed: Arc<CacheDB>,
    ) -> Result<(Vec<Receipt>, u64), EvmError> {
        let store = db.store.clone();
        let header = &block.header;
        let n_txs = txs_with_sender.len();

        // 1. Convert BAL → AccountUpdates and send to merkleizer (single batch)
        //    This covers ALL state changes: system calls, txs, withdrawals.
        let account_updates = Self::bal_to_account_updates(bal, store.as_ref())?;
        merkleizer
            .send(account_updates)
            .map_err(|e| EvmError::Custom(format!("merkleizer send failed: {e}")))?;
        queue_length.fetch_add(1, Ordering::Relaxed);

        // Build validation index once — shared read-only across parallel tx validations.
        let validation_index = bal.build_validation_index();

        // Pre-compute capacity hint for per-tx DBs from BAL account count.
        let bal_account_count = bal.accounts().len();

        // 2. Execute all txs in parallel (embarrassingly parallel, BAL-seeded)
        let t_exec = std::time::Instant::now();
        let results: Result<Vec<(usize, TxType, ExecutionReport)>, EvmError> = (0..n_txs)
            .into_par_iter()
            .map(|tx_idx| -> Result<_, EvmError> {
                let (tx, sender) = &txs_with_sender[tx_idx];
                let mut tx_db = GeneralizedDatabase::new_with_shared_base_and_capacity(
                    store.clone(),
                    system_seed.clone(),
                    bal_account_count,
                );
                // Small capacity: parallel txs rarely nest >8 call frames, and
                // over-allocating per-tx wastes memory across many rayon tasks.
                let mut stack_pool = Vec::with_capacity(8);

                // Pre-seed with BAL-derived intermediate state.
                // BAL index: 0 = system calls, 1 = tx 0, 2 = tx 1, ...
                // For tx at index i, we want state through BAL index i
                // (= system calls + effects of txs 0..i-1).
                #[allow(clippy::cast_possible_truncation)]
                Self::seed_db_from_bal(&mut tx_db, bal, tx_idx as u16)?;

                let report = LEVM::execute_tx_in_block(
                    tx,
                    *sender,
                    header,
                    &mut tx_db,
                    vm_type,
                    &mut stack_pool,
                )?;

                // Validate execution results against BAL claims (per-tx).
                // BAL index for tx at position i is i+1 (0 = system calls).
                // seed_idx = tx_idx (the highest BAL index used for seeding).
                #[allow(clippy::cast_possible_truncation)]
                let bal_idx = (tx_idx + 1) as u16;
                #[allow(clippy::cast_possible_truncation)]
                let seed_idx = tx_idx as u16;
                Self::validate_tx_execution(
                    bal_idx,
                    seed_idx,
                    &tx_db.current_accounts_state,
                    &tx_db.codes,
                    bal,
                    &validation_index,
                )
                .map_err(|e| {
                    EvmError::Custom(format!("BAL validation failed for tx {tx_idx}: {e}"))
                })?;

                Ok((tx_idx, tx.tx_type(), report))
            })
            .collect();

        let exec_ms = t_exec.elapsed().as_secs_f64() * 1000.0;
        let mut results = results?;

        // 3. Sort by tx_idx and build receipts
        results.sort_unstable_by_key(|(idx, _, _)| *idx);

        let mut receipts = Vec::with_capacity(n_txs);
        let mut cumulative_gas_used = 0_u64;
        let mut block_gas_used = 0_u64;

        for (_, tx_type, report) in results {
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

        ::tracing::debug!(
            "[PARALLEL] block {} | {} txs | exec: {:.1}ms",
            block.header.number,
            n_txs,
            exec_ms,
        );

        Ok((receipts, block_gas_used))
    }

    /// Validates that a tx's post-execution state matches BAL claims.
    ///
    /// Replaces the previous snapshot->diff->validate approach:
    /// - No HashMap clone needed (reconstructs seeded values from BAL)
    /// - Uses pre-built index for O(1) account lookups
    /// - Uses binary search on sorted change lists
    ///
    /// `bal_idx`: block_access_index for this tx (tx_idx + 1)
    /// `seed_idx`: max BAL index used for seeding (= tx_idx = bal_idx - 1)
    /// `current_state`: post-execution account state from per-tx DB
    /// `codes`: code cache from per-tx DB (for code change validation)
    /// `bal`: the block access list
    /// `index`: pre-built validation index
    #[allow(clippy::too_many_arguments)]
    fn validate_tx_execution(
        bal_idx: u16,
        seed_idx: u16,
        current_state: &FxHashMap<Address, LevmAccount>,
        codes: &FxHashMap<H256, Code>,
        bal: &BlockAccessList,
        index: &BalAddressIndex,
    ) -> Result<(), String> {
        // PART A: For each BAL account with changes at bal_idx,
        //         verify execution produced matching post-state.
        if let Some(active_accounts) = index.tx_to_accounts.get(&bal_idx) {
            for &acct_inner_idx in active_accounts {
                let acct = &bal.accounts()[acct_inner_idx];
                let addr = acct.address;
                let actual = current_state.get(&addr);

                // Balance
                if let Some(expected) = find_exact_change_balance(&acct.balance_changes, bal_idx) {
                    match actual {
                        Some(a) if a.info.balance == expected => {}
                        Some(a) => {
                            return Err(format!(
                                "account {addr:?} balance mismatch at index {bal_idx}: BAL={expected}, exec={}",
                                a.info.balance
                            ));
                        }
                        None => {
                            return Err(format!(
                                "account {addr:?} has BAL balance change at {bal_idx} but not in execution state"
                            ));
                        }
                    }
                }

                // Nonce
                if let Some(expected) = find_exact_change_nonce(&acct.nonce_changes, bal_idx) {
                    match actual {
                        Some(a) if a.info.nonce == expected => {}
                        Some(a) => {
                            return Err(format!(
                                "account {addr:?} nonce mismatch at index {bal_idx}: BAL={expected}, exec={}",
                                a.info.nonce
                            ));
                        }
                        None => {
                            return Err(format!(
                                "account {addr:?} has BAL nonce change at {bal_idx} but not in execution state"
                            ));
                        }
                    }
                }

                // Code
                if let Some(expected_code) = find_exact_change_code(&acct.code_changes, bal_idx) {
                    match actual {
                        Some(a) => {
                            let actual_code = codes
                                .get(&a.info.code_hash)
                                .map(|c| &c.bytecode)
                                .cloned()
                                .unwrap_or_default();
                            if actual_code != *expected_code {
                                return Err(format!(
                                    "account {addr:?} code mismatch at index {bal_idx}"
                                ));
                            }
                        }
                        None => {
                            return Err(format!(
                                "account {addr:?} has BAL code change at {bal_idx} but not in execution state"
                            ));
                        }
                    }
                }

                // Storage
                for sc in &acct.storage_changes {
                    if let Some(expected_value) =
                        find_exact_change_storage(&sc.slot_changes, bal_idx)
                    {
                        let key = ethrex_common::utils::u256_to_h256(sc.slot);
                        let actual_value = actual.and_then(|a| a.storage.get(&key)).copied();
                        if actual_value != Some(expected_value) {
                            return Err(format!(
                                "account {addr:?} storage slot {} mismatch at index {bal_idx}: \
                                 BAL={expected_value}, exec={actual_value:?}",
                                sc.slot
                            ));
                        }
                    }
                }
            }
        }

        // PART B: For each modified account in execution state,
        //         verify no unexpected mutations (changes not claimed by BAL).
        for (addr, account) in current_state {
            if account.is_unmodified() {
                continue;
            }

            let Some(&bal_acct_idx) = index.addr_to_idx.get(addr) else {
                // Account not in BAL. Modified status can come from read-only
                // get_account_mut calls (warm access, etc.). Skip — state root
                // will catch any true discrepancy.
                continue;
            };

            let acct = &bal.accounts()[bal_acct_idx];

            // Balance: if BAL has no change at bal_idx, execution must not have changed it
            if !has_exact_change_balance(&acct.balance_changes, bal_idx) {
                let seeded_pos = acct
                    .balance_changes
                    .partition_point(|c| c.block_access_index <= seed_idx);
                if seeded_pos > 0 {
                    let seeded = acct.balance_changes[seeded_pos - 1].post_balance;
                    if account.info.balance != seeded {
                        return Err(format!(
                            "account {addr:?} balance changed by execution ({}) but BAL has no \
                             balance change at index {bal_idx} (seeded={seeded})",
                            account.info.balance
                        ));
                    }
                }
                // If seeded_pos == 0, balance was never seeded (loaded from store/shared_base).
                // We can't cheaply verify without store access. Skip.
            }

            // Nonce: same pattern
            if !has_exact_change_nonce(&acct.nonce_changes, bal_idx) {
                let seeded_pos = acct
                    .nonce_changes
                    .partition_point(|c| c.block_access_index <= seed_idx);
                if seeded_pos > 0 {
                    let seeded = acct.nonce_changes[seeded_pos - 1].post_nonce;
                    if account.info.nonce != seeded {
                        return Err(format!(
                            "account {addr:?} nonce changed by execution ({}) but BAL has no \
                             nonce change at index {bal_idx} (seeded={seeded})",
                            account.info.nonce
                        ));
                    }
                }
            }

            // Code: same pattern
            if !has_exact_change_code(&acct.code_changes, bal_idx) {
                let seeded_pos = acct
                    .code_changes
                    .partition_point(|c| c.block_access_index <= seed_idx);
                if seeded_pos > 0 {
                    let seeded_code = &acct.code_changes[seeded_pos - 1].new_code;
                    let seeded_hash = if seeded_code.is_empty() {
                        *EMPTY_KECCACK_HASH
                    } else {
                        Code::from_bytecode(seeded_code.clone()).hash
                    };
                    if account.info.code_hash != seeded_hash {
                        return Err(format!(
                            "account {addr:?} code changed by execution but BAL has no \
                             code change at index {bal_idx}"
                        ));
                    }
                }
            }

            // Storage: for each slot in execution state, check it's expected
            for (key_h256, &value) in &account.storage {
                let slot_u256 = U256::from_big_endian(key_h256.as_bytes());
                // EIP-7928 requires storage_changes sorted by slot, so use binary search.
                let pos = acct
                    .storage_changes
                    .partition_point(|sc| sc.slot < slot_u256);
                if pos < acct.storage_changes.len() && acct.storage_changes[pos].slot == slot_u256 {
                    let sc = &acct.storage_changes[pos];
                    if !has_exact_change_storage(&sc.slot_changes, bal_idx) {
                        let seeded_pos = sc
                            .slot_changes
                            .partition_point(|c| c.block_access_index <= seed_idx);
                        if seeded_pos > 0 {
                            let seeded = sc.slot_changes[seeded_pos - 1].post_value;
                            if value != seeded {
                                return Err(format!(
                                    "account {addr:?} storage slot {slot_u256} changed by \
                                     execution ({value}) but BAL has no change at index \
                                     {bal_idx} (seeded={seeded})"
                                ));
                            }
                        }
                    }
                }
                // Slot not in BAL storage_changes: was loaded from store during execution.
                // Skip — can't verify cheaply.
            }
        }

        Ok(())
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

        // Phase 1: Prefetch all account states — parallel inner fetch + single write-lock.
        // This warms the CachingDatabase account cache and the TrieLayerCache
        // with state trie nodes, so Phase 2 storage reads benefit from cached lookups.
        let account_addresses: Vec<Address> = accounts.iter().map(|ac| ac.address).collect();
        store
            .prefetch_accounts(&account_addresses)
            .map_err(|e| EvmError::Custom(format!("prefetch_accounts: {e}")))?;

        // Phase 2: Prefetch storage slots in batch — parallel inner fetch + single write-lock.
        // Storage is flattened to (address, slot) pairs so rayon can distribute
        // work across threads regardless of how many slots each account has.
        // Without flattening, a hot contract with hundreds of slots (e.g. a DEX
        // pool) would monopolize a single thread while others go idle.
        let slots: Vec<(Address, ethrex_common::H256)> = accounts
            .iter()
            .flat_map(|ac| {
                ac.all_storage_slots()
                    .map(move |slot| (ac.address, ethrex_common::H256::from_uint(&slot)))
            })
            .collect();
        store
            .prefetch_storage(&slots)
            .map_err(|e| EvmError::Custom(format!("prefetch_storage: {e}")))?;

        // Phase 3: Code prefetch — collect code hashes from Phase 1 account states
        // (already cached after Phase 1 prefetch), then batch-fetch codes in parallel.
        // Uses par_iter for collection since blocks can have thousands of accounts.
        let code_hashes: Vec<ethrex_common::H256> = accounts
            .par_iter()
            .filter_map(|ac| {
                store
                    .get_account_state(ac.address)
                    .ok()
                    .filter(|s| s.code_hash != *EMPTY_KECCACK_HASH)
                    .map(|s| s.code_hash)
            })
            .collect();
        code_hashes.par_iter().for_each(|&h| {
            let _ = store.get_account_code(h);
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
mod bal_tests {
    use super::*;
    use ethrex_common::H256;
    use ethrex_common::types::AccountState;
    use ethrex_common::types::block_access_list::{
        AccountChanges, BalanceChange, NonceChange, SlotChange, StorageChange,
    };
    use ethrex_levm::errors::DatabaseError;

    fn addr(byte: u8) -> Address {
        let mut a = Address::zero();
        a.0[19] = byte;
        a
    }

    /// Minimal in-memory store for testing bal_to_account_updates.
    struct MockStore {
        accounts: FxHashMap<Address, AccountState>,
    }

    impl MockStore {
        fn new() -> Self {
            Self {
                accounts: FxHashMap::default(),
            }
        }

        fn with_account(mut self, address: Address, state: AccountState) -> Self {
            self.accounts.insert(address, state);
            self
        }
    }

    impl Database for MockStore {
        fn get_account_state(&self, address: Address) -> Result<AccountState, DatabaseError> {
            Ok(self.accounts.get(&address).copied().unwrap_or_default())
        }
        fn get_storage_value(&self, _: Address, _: H256) -> Result<U256, DatabaseError> {
            Ok(U256::zero())
        }
        fn get_block_hash(&self, _: u64) -> Result<H256, DatabaseError> {
            Ok(H256::zero())
        }
        fn get_chain_config(&self) -> Result<ethrex_common::types::ChainConfig, DatabaseError> {
            Err(DatabaseError::Custom("not implemented".into()))
        }
        fn get_account_code(&self, _: H256) -> Result<ethrex_common::types::Code, DatabaseError> {
            Ok(ethrex_common::types::Code::from_bytecode(Bytes::new()))
        }
        fn get_code_metadata(
            &self,
            _: H256,
        ) -> Result<ethrex_common::types::CodeMetadata, DatabaseError> {
            Ok(ethrex_common::types::CodeMetadata { length: 0 })
        }
    }

    #[test]
    fn test_bal_to_account_updates_basic() {
        // Account with balance + nonce + storage changes → correct AccountUpdate
        let address = addr(1);
        let store = MockStore::new().with_account(
            address,
            AccountState {
                balance: U256::from(100),
                nonce: 5,
                code_hash: *EMPTY_KECCACK_HASH,
                storage_root: H256::zero(),
            },
        );

        let bal = BlockAccessList::from_accounts(vec![
            AccountChanges::new(address)
                .with_balance_changes(vec![
                    BalanceChange::new(1, U256::from(90)),
                    BalanceChange::new(2, U256::from(80)),
                ])
                .with_nonce_changes(vec![NonceChange::new(1, 6)])
                .with_storage_changes(vec![SlotChange::with_changes(
                    U256::from(42),
                    vec![StorageChange::new(1, U256::from(999))],
                )]),
        ]);

        let updates = LEVM::bal_to_account_updates(&bal, &store).unwrap();
        assert_eq!(updates.len(), 1);
        let u = &updates[0];
        assert_eq!(u.address, address);
        assert!(!u.removed);
        let info = u.info.as_ref().unwrap();
        // Last balance entry wins
        assert_eq!(info.balance, U256::from(80));
        assert_eq!(info.nonce, 6);
        assert_eq!(info.code_hash, *EMPTY_KECCACK_HASH);
        // Storage
        let key = ethrex_common::utils::u256_to_h256(U256::from(42));
        assert_eq!(*u.added_storage.get(&key).unwrap(), U256::from(999));
    }

    #[test]
    fn test_bal_to_account_updates_highest_index_wins() {
        // Multiple changes per field: the last entry (highest index) wins.
        let address = addr(2);
        let store = MockStore::new().with_account(
            address,
            AccountState {
                balance: U256::from(1000),
                nonce: 0,
                code_hash: *EMPTY_KECCACK_HASH,
                storage_root: H256::zero(),
            },
        );

        let bal = BlockAccessList::from_accounts(vec![
            AccountChanges::new(address).with_balance_changes(vec![
                BalanceChange::new(1, U256::from(900)),
                BalanceChange::new(2, U256::from(800)),
                BalanceChange::new(3, U256::from(700)),
            ]),
        ]);

        let updates = LEVM::bal_to_account_updates(&bal, &store).unwrap();
        assert_eq!(updates.len(), 1);
        assert_eq!(updates[0].info.as_ref().unwrap().balance, U256::from(700));
    }

    #[test]
    fn test_bal_to_account_updates_reads_only_skipped() {
        // Account with only storage_reads and no writes → no AccountUpdate.
        let address = addr(3);
        let store = MockStore::new();

        let bal = BlockAccessList::from_accounts(vec![
            AccountChanges::new(address).with_storage_reads(vec![U256::from(1)]),
        ]);

        let updates = LEVM::bal_to_account_updates(&bal, &store).unwrap();
        assert!(updates.is_empty());
    }

    #[test]
    fn test_bal_to_account_updates_removal() {
        // Account removal (EIP-161): post-state empty but pre-state existed.
        let address = addr(4);
        let store = MockStore::new().with_account(
            address,
            AccountState {
                balance: U256::from(50),
                nonce: 1,
                code_hash: *EMPTY_KECCACK_HASH,
                storage_root: H256::zero(),
            },
        );

        let bal = BlockAccessList::from_accounts(vec![
            AccountChanges::new(address)
                .with_balance_changes(vec![BalanceChange::new(1, U256::zero())])
                .with_nonce_changes(vec![NonceChange::new(1, 0)]),
        ]);

        let updates = LEVM::bal_to_account_updates(&bal, &store).unwrap();
        assert_eq!(updates.len(), 1);
        assert!(updates[0].removed);
    }

    #[test]
    fn test_bal_to_account_updates_storage_zero() {
        // Storage slot set to 0 → included in added_storage (valid trie deletion).
        let address = addr(5);
        let store = MockStore::new();

        let bal = BlockAccessList::from_accounts(vec![
            AccountChanges::new(address).with_storage_changes(vec![SlotChange::with_changes(
                U256::from(7),
                vec![StorageChange::new(1, U256::zero())],
            )]),
        ]);

        let updates = LEVM::bal_to_account_updates(&bal, &store).unwrap();
        assert_eq!(updates.len(), 1);
        let key = ethrex_common::utils::u256_to_h256(U256::from(7));
        assert_eq!(*updates[0].added_storage.get(&key).unwrap(), U256::zero());
    }

    #[test]
    fn test_bal_to_account_updates_code_deployment() {
        // Code deployment → correct code_hash computed.
        let address = addr(6);
        let store = MockStore::new();
        let code = Bytes::from(vec![0x60, 0x00, 0x60, 0x00, 0xf3]); // PUSH0 PUSH0 RETURN
        let expected_hash = ethrex_common::types::Code::from_bytecode(code.clone()).hash;

        let bal = BlockAccessList::from_accounts(vec![
            AccountChanges::new(address)
                .with_code_changes(vec![
                    ethrex_common::types::block_access_list::CodeChange::new(1, code.clone()),
                ])
                .with_nonce_changes(vec![NonceChange::new(1, 1)]),
        ]);

        let updates = LEVM::bal_to_account_updates(&bal, &store).unwrap();
        assert_eq!(updates.len(), 1);
        let u = &updates[0];
        assert_eq!(u.info.as_ref().unwrap().code_hash, expected_hash);
        assert_eq!(u.code.as_ref().unwrap().bytecode, code);
    }
}
