use ethrex_common::constants::EMPTY_KECCACK_HASH;
use ethrex_common::tracing::{PrePostState, PrestateAccountState, PrestateResult, PrestateTrace};
use ethrex_common::types::{Block, Transaction};
use ethrex_common::{Address, BigEndianHash, H256, tracing::CallTrace, types::BlockHeader};
use ethrex_crypto::Crypto;
use ethrex_levm::account::LevmAccount;
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
    ) -> Result<PrestateResult, EvmError> {
        // Snapshot the current cache state before executing the tx.
        let pre_snapshot: CacheDB = db.current_accounts_state.clone();

        // Execute the transaction (updates current_accounts_state in place).
        let sender = tx
            .sender(crypto)
            .map_err(|e| EvmError::Transaction(format!("Couldn't recover sender: {e}")))?;
        let env = Self::setup_env(tx, sender, block_header, db, vm_type)?;
        let mut vm = VM::new(env, db, tx, LevmCallTracer::disabled(), vm_type, crypto)?;
        vm.execute()?;

        let pre_map = build_pre_state_map(&pre_snapshot, &db.current_accounts_state, db);

        if diff_mode {
            let post_map = build_post_state_map(&pre_snapshot, &db.current_accounts_state, db);
            Ok(PrestateResult::Diff(PrePostState {
                pre: pre_map,
                post: post_map,
            }))
        } else {
            Ok(PrestateResult::Prestate(pre_map))
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

/// Identifies accounts touched by a transaction by comparing `pre_snapshot`
/// (cache before the tx) with `post_cache` (cache after the tx).
///
/// Returns `(address, pre_account, post_account)` for each touched account.
/// `pre_account` is the account state before the tx ran — sourced from
/// `pre_snapshot` if the account was already cached, or from
/// `initial_accounts_state` if the account was first loaded during this tx.
fn find_touched_accounts<'a>(
    pre_snapshot: &'a CacheDB,
    post_cache: &'a CacheDB,
    db: &'a GeneralizedDatabase,
) -> Vec<(Address, &'a LevmAccount, &'a LevmAccount)> {
    let mut touched = Vec::new();

    for (addr, post_account) in post_cache {
        let pre_account = match pre_snapshot.get(addr) {
            Some(pre) => {
                if pre.info == post_account.info && pre.storage == post_account.storage {
                    continue;
                }
                pre
            }
            None => {
                // Account was first loaded during this tx.
                // Pre-state comes from initial_accounts_state (the pristine DB-loaded value).
                let Some(initial) = db.initial_accounts_state.get(addr) else {
                    continue;
                };
                if initial.info == post_account.info && initial.storage == post_account.storage {
                    continue;
                }
                initial
            }
        };

        touched.push((*addr, pre_account, post_account));
    }

    touched
}

/// Build the account state output for one account.
fn build_account_output(account: &LevmAccount, db: &GeneralizedDatabase) -> PrestateAccountState {
    let code = if account.info.code_hash != *EMPTY_KECCACK_HASH {
        db.codes
            .get(&account.info.code_hash)
            .map(|c| c.bytecode.clone())
            .unwrap_or_default()
    } else {
        bytes::Bytes::new()
    };

    let storage = account
        .storage
        .iter()
        .filter(|(_, v)| !v.is_zero())
        .map(|(k, v)| (*k, H256::from_uint(v)))
        .collect();

    PrestateAccountState {
        balance: account.info.balance,
        nonce: account.info.nonce,
        code,
        storage,
    }
}

/// Build the pre-tx state map for all accounts touched by a transaction.
///
/// For already-cached accounts, the pre_snapshot only contains storage slots
/// loaded by *previous* transactions. Any slot first accessed during *this*
/// transaction has its original value in `initial_accounts_state`. We merge
/// both sources so the output includes every accessed slot.
fn build_pre_state_map(
    pre_snapshot: &CacheDB,
    post_cache: &CacheDB,
    db: &GeneralizedDatabase,
) -> PrestateTrace {
    let mut result = PrestateTrace::new();

    for (addr, pre_account, post_account) in find_touched_accounts(pre_snapshot, post_cache, db) {
        let mut state = build_account_output(pre_account, db);

        // For already-cached accounts, merge newly-loaded slots from initial_accounts_state
        // and filter to only slots accessed in this tx.
        if pre_snapshot.contains_key(&addr) {
            if let Some(initial) = db.initial_accounts_state.get(&addr) {
                for (k, v) in &initial.storage {
                    state
                        .storage
                        .entry(*k)
                        .or_insert_with(|| H256::from_uint(v));
                }
            }
            // Only keep slots actually accessed in this tx.
            state
                .storage
                .retain(|k, _| post_account.storage.contains_key(k));
        }

        result.insert(addr, state);
    }

    result
}

/// Build the post-tx state map for all accounts touched by a transaction.
fn build_post_state_map(
    pre_snapshot: &CacheDB,
    post_cache: &CacheDB,
    db: &GeneralizedDatabase,
) -> PrestateTrace {
    let mut result = PrestateTrace::new();

    for (addr, _, post_account) in find_touched_accounts(pre_snapshot, post_cache, db) {
        result.insert(addr, build_account_output(post_account, db));
    }

    result
}
