use ethrex_common::constants::EMPTY_KECCACK_HASH;
use ethrex_common::tracing::{PrePostState, PrestateAccountState, PrestateTrace};
use ethrex_common::types::{Block, Transaction};
use ethrex_common::{tracing::CallTrace, types::BlockHeader};
use ethrex_crypto::Crypto;
use ethrex_levm::db::gen_db::CacheDB;
use ethrex_levm::vm::VMType;
use ethrex_levm::{db::gen_db::GeneralizedDatabase, tracing::LevmCallTracer, vm::VM};

use crate::{EvmError, backends::levm::LEVM};

impl LEVM {
    /// Execute all transactions of the block up until a certain transaction specified in `stop_index`.
    /// The goal is to just mutate the state up to that point, without needing to process transaction receipts or requests.
    pub fn rerun_block(
        db: &mut GeneralizedDatabase,
        block: &Block,
        stop_index: Option<usize>,
        vm_type: VMType,
        crypto: &dyn Crypto,
    ) -> Result<(), EvmError> {
        Self::prepare_block(block, db, vm_type, crypto)?;

        // Executes transactions and stops when the index matches the stop index.
        for (index, (tx, sender)) in block
            .body
            .get_transactions_with_sender(crypto)
            .map_err(|error| EvmError::Transaction(error.to_string()))?
            .into_iter()
            .enumerate()
        {
            if stop_index.is_some_and(|stop| stop == index) {
                break;
            }

            Self::execute_tx(tx, sender, &block.header, db, vm_type, crypto)?;
        }

        // Process withdrawals only if the whole block has been executed.
        if stop_index.is_none()
            && let Some(withdrawals) = &block.body.withdrawals
        {
            Self::process_withdrawals(db, withdrawals)?;
        };

        Ok(())
    }

    /// Execute a transaction and capture the pre/post account state (prestateTracer).
    ///
    /// Captures a snapshot of all touched accounts before and after execution.
    /// The `diff_mode` flag controls whether to return both pre and post state or just pre state.
    ///
    /// Assumes the db already contains the state from all prior transactions in the block.
    pub fn trace_tx_prestate(
        db: &mut GeneralizedDatabase,
        block_header: &BlockHeader,
        tx: &Transaction,
        diff_mode: bool,
        vm_type: VMType,
        crypto: &dyn Crypto,
    ) -> Result<(PrestateTrace, Option<PrePostState>), EvmError> {
        // Snapshot the current cache state before executing the tx.
        // This is the pre-tx state for all accounts already loaded in the cache.
        let pre_snapshot: CacheDB = db.current_accounts_state.clone();

        // Execute the transaction (updates current_accounts_state in place)
        let sender = tx
            .sender(crypto)
            .map_err(|e| EvmError::Transaction(format!("Couldn't recover sender: {e}")))?;
        let env = Self::setup_env(tx, sender, block_header, db, vm_type)?;
        let mut vm = VM::new(env, db, tx, LevmCallTracer::disabled(), vm_type, crypto)?;
        vm.execute()?;

        // Build the pre and post state maps for all accounts that were touched
        let pre_map = build_account_state_map(&pre_snapshot, &db.current_accounts_state, db, true);
        let post_map =
            build_account_state_map(&pre_snapshot, &db.current_accounts_state, db, false);

        if diff_mode {
            Ok((
                PrestateTrace::new(),
                Some(PrePostState {
                    pre: pre_map,
                    post: post_map,
                }),
            ))
        } else {
            Ok((pre_map, None))
        }
    }

    /// Run transaction with callTracer activated.
    pub fn trace_tx_calls(
        db: &mut GeneralizedDatabase,
        block_header: &BlockHeader,
        tx: &Transaction,
        only_top_call: bool,
        with_log: bool,
        vm_type: VMType,
        crypto: &dyn Crypto,
    ) -> Result<CallTrace, EvmError> {
        let env = Self::setup_env(
            tx,
            tx.sender(crypto).map_err(|error| {
                EvmError::Transaction(format!("Couldn't recover addresses with error: {error}"))
            })?,
            block_header,
            db,
            vm_type,
        )?;
        let mut vm = VM::new(
            env,
            db,
            tx,
            LevmCallTracer::new(only_top_call, with_log),
            vm_type,
            crypto,
        )?;

        vm.execute()?;

        let callframe = vm.get_trace_result()?;

        // We only return the top call because a transaction only has one call with subcalls
        Ok(vec![callframe])
    }
}

/// Build a map of address -> `PrestateAccountState` for all accounts touched by a transaction.
///
/// `pre_snapshot` is a snapshot of `current_accounts_state` taken BEFORE the tx executed.
/// `post_cache` is `current_accounts_state` AFTER the tx executed.
/// `db` is the database (used to look up code bytes by hash for new accounts).
/// `use_pre` controls whether to use the pre-tx state (true) or post-tx state (false).
///
/// An account is "touched" if:
/// - It was newly loaded during this tx (present in `post_cache` but not in `pre_snapshot`)
/// - It was already cached and was modified (exists in both but differs)
fn build_account_state_map(
    pre_snapshot: &CacheDB,
    post_cache: &CacheDB,
    db: &GeneralizedDatabase,
    use_pre: bool,
) -> PrestateTrace {
    let mut result = PrestateTrace::new();

    for (addr, post_account) in post_cache {
        let (touched, pre_account_opt) = match pre_snapshot.get(addr) {
            None => {
                // Account was first loaded during this tx.
                // Pre-state comes from initial_accounts_state (the value loaded from DB before this tx).
                let pre_in_initial = db.initial_accounts_state.get(addr);
                // Consider touched only if the account changed (info or storage differ from initial).
                let changed = pre_in_initial.is_none_or(|pre| {
                    pre.info != post_account.info || pre.storage != post_account.storage
                });
                (changed, pre_in_initial)
            }
            Some(pre_account) => {
                // Account was already in cache. Only include if something changed.
                let changed = pre_account.info != post_account.info
                    || pre_account.storage != post_account.storage;
                (changed, Some(pre_account))
            }
        };

        if !touched {
            continue;
        }

        let source_account = if use_pre {
            match pre_account_opt {
                Some(a) => a,
                // If we can't find pre-state (shouldn't happen), skip this account
                None => continue,
            }
        } else {
            post_account
        };

        let address_hex = format!("0x{:x}", addr);

        // Look up code if account has non-empty code hash
        let code = if source_account.info.code_hash != *EMPTY_KECCACK_HASH {
            db.codes
                .get(&source_account.info.code_hash)
                .map(|c| format!("0x{}", hex::encode(&c.bytecode)))
        } else {
            None
        };

        // Convert storage slots to hex strings
        let storage: std::collections::HashMap<String, String> = source_account
            .storage
            .iter()
            .filter(|(_, v)| !v.is_zero())
            .map(|(k, v)| {
                let key_hex = format!("0x{:x}", k);
                let val_hex = format!("0x{:x}", v);
                (key_hex, val_hex)
            })
            .collect();

        let account_state = PrestateAccountState {
            balance: format!("0x{:x}", source_account.info.balance),
            nonce: source_account.info.nonce,
            code,
            storage,
        };

        result.insert(address_hex, account_state);
    }

    result
}
