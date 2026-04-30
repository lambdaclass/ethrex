use ethrex_common::constants::EMPTY_KECCACK_HASH;
use ethrex_common::tracing::{PrePostState, PrestateAccountState, PrestateResult, PrestateTrace};
use ethrex_common::types::{Block, Transaction};
use ethrex_common::{Address, BigEndianHash, H256, tracing::CallTrace, types::BlockHeader};
use ethrex_crypto::Crypto;
use ethrex_levm::account::{AccountStatus, LevmAccount};
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

    /// Executes `tx` and returns the prestateTracer result. `diff_mode` toggles between
    /// pre-only and pre+post output. Assumes `db` already reflects all prior txs in the block.
    pub fn trace_tx_prestate(
        db: &mut GeneralizedDatabase,
        block_header: &BlockHeader,
        tx: &Transaction,
        diff_mode: bool,
        vm_type: VMType,
        crypto: &dyn Crypto,
    ) -> Result<PrestateResult, EvmError> {
        let pre_snapshot: CacheDB = db.current_accounts_state.clone();

        let sender = tx
            .sender(crypto)
            .map_err(|e| EvmError::Transaction(format!("Couldn't recover sender: {e}")))?;
        let env = Self::setup_env(tx, sender, block_header, db, vm_type)?;
        let mut vm = VM::new(env, db, tx, LevmCallTracer::disabled(), vm_type, crypto)?;
        vm.execute()?;

        preload_touched_codes(&pre_snapshot, db)?;

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

/// Returns `(address, pre_account, post_account)` for every account in `post_cache`.
/// `pre_account` comes from `pre_snapshot` if cached before the tx, otherwise from
/// `initial_accounts_state`. Filtering unchanged accounts is the caller's job.
fn find_touched_accounts<'a>(
    pre_snapshot: &'a CacheDB,
    post_cache: &'a CacheDB,
    db: &'a GeneralizedDatabase,
) -> Vec<(Address, &'a LevmAccount, &'a LevmAccount)> {
    let mut touched = Vec::new();

    for (addr, post_account) in post_cache {
        let pre_account = match pre_snapshot.get(addr) {
            Some(pre) => pre,
            None => {
                let Some(initial) = db.initial_accounts_state.get(addr) else {
                    continue;
                };
                initial
            }
        };

        touched.push((*addr, pre_account, post_account));
    }

    touched
}

/// Reads code from `db.codes`; caller must `preload_touched_codes` first.
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

/// Builds the pre-tx state map. For accounts cached before this tx, `pre_snapshot`
/// only holds slots loaded by previous txs; slots first read in this tx come from
/// `initial_accounts_state`, so we merge both then keep only slots touched here.
fn build_pre_state_map(
    pre_snapshot: &CacheDB,
    post_cache: &CacheDB,
    db: &GeneralizedDatabase,
) -> PrestateTrace {
    let mut result = PrestateTrace::new();

    for (addr, pre_account, post_account) in find_touched_accounts(pre_snapshot, post_cache, db) {
        let mut state = build_account_output(pre_account, db);

        if let Some(pre_cached) = pre_snapshot.get(&addr) {
            if let Some(initial) = db.initial_accounts_state.get(&addr) {
                for (k, v) in &initial.storage {
                    state
                        .storage
                        .entry(*k)
                        .or_insert_with(|| H256::from_uint(v));
                }
            }
            state.storage.retain(|k, _| {
                if !pre_cached.storage.contains_key(k) {
                    return true;
                }
                pre_cached.storage.get(k) != post_account.storage.get(k)
            });
        }

        result.insert(addr, state);
    }

    result
}

/// Loads code into `db.codes` for every touched contract whose code wasn't executed
/// (SELFDESTRUCT beneficiaries, plain-value transfer recipients) — without this they'd
/// serialize as `code: 0x` despite a non-empty `code_hash`.
fn preload_touched_codes(
    pre_snapshot: &CacheDB,
    db: &mut GeneralizedDatabase,
) -> Result<(), EvmError> {
    let hashes: Vec<H256> = db
        .current_accounts_state
        .iter()
        .flat_map(|(addr, post)| {
            let pre_hash = pre_snapshot
                .get(addr)
                .or_else(|| db.initial_accounts_state.get(addr))
                .map(|a| a.info.code_hash)
                .unwrap_or_default();
            [post.info.code_hash, pre_hash]
        })
        .filter(|h| *h != *EMPTY_KECCACK_HASH)
        .collect();

    for hash in hashes {
        db.get_code(hash)?;
    }
    Ok(())
}

/// Builds the diff-mode post-state: only changed accounts; destroyed accounts omitted.
fn build_post_state_map(
    pre_snapshot: &CacheDB,
    post_cache: &CacheDB,
    db: &GeneralizedDatabase,
) -> PrestateTrace {
    let mut result = PrestateTrace::new();

    for (addr, pre_account, post_account) in find_touched_accounts(pre_snapshot, post_cache, db) {
        if matches!(
            post_account.status,
            AccountStatus::Destroyed | AccountStatus::DestroyedModified,
        ) {
            continue;
        }

        if pre_account.info == post_account.info && pre_account.storage == post_account.storage {
            continue;
        }

        let mut state = build_account_output(post_account, db);

        if let Some(pre_cached) = pre_snapshot.get(&addr) {
            state.storage.retain(|k, _| {
                if !pre_cached.storage.contains_key(k) {
                    return true;
                }
                pre_cached.storage.get(k) != post_account.storage.get(k)
            });
        }

        result.insert(addr, state);
    }

    result
}
