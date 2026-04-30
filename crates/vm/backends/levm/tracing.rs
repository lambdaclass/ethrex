use ethrex_common::constants::EMPTY_KECCACK_HASH;
use ethrex_common::tracing::{PrePostState, PrestateAccountState, PrestateResult, PrestateTrace};
use ethrex_common::types::{Block, Transaction};
use ethrex_common::{Address, BigEndianHash, H256, U256, tracing::CallTrace, types::BlockHeader};
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
    /// pre-only and pre+post output. `include_empty` keeps entries that would otherwise
    /// be all-default (must be false in diff mode). Assumes `db` already reflects all
    /// prior txs in the block.
    pub fn trace_tx_prestate(
        db: &mut GeneralizedDatabase,
        block_header: &BlockHeader,
        tx: &Transaction,
        diff_mode: bool,
        include_empty: bool,
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

        let mut pre_map = build_pre_state_map(&pre_snapshot, &db.current_accounts_state, db);

        if diff_mode {
            let post_map = build_post_state_map(&pre_snapshot, &db.current_accounts_state, db);
            // Storage in diff pre keeps only slots that changed AND have a non-zero pre value.
            filter_diff_pre_storage(&mut pre_map, &db.current_accounts_state);
            // Pre keeps only accounts that ended up in post (modified) or were destroyed
            // in this tx (those appear in pre but not post).
            let kept =
                modified_or_destroyed_addresses(&pre_snapshot, &db.current_accounts_state, db);
            pre_map.retain(|addr, _| kept.contains(addr));
            // Empty entries are always dropped in diff mode.
            pre_map.retain(|_, state| !state.is_empty());
            Ok(PrestateResult::Diff(PrePostState {
                pre: pre_map,
                post: post_map,
            }))
        } else {
            if !include_empty {
                pre_map.retain(|_, state| !state.is_empty());
            }
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
/// Storage values are passed through as-is (including zero); per-field filtering
/// for diff-mode post is applied by `build_post_output`.
fn build_account_output(account: &LevmAccount, db: &GeneralizedDatabase) -> PrestateAccountState {
    let has_code = account.info.code_hash != *EMPTY_KECCACK_HASH;
    let code = if has_code {
        db.codes
            .get(&account.info.code_hash)
            .map(|c| c.bytecode.clone())
            .expect("code preloaded by preload_touched_codes")
    } else {
        bytes::Bytes::new()
    };
    let code_hash = if has_code {
        account.info.code_hash
    } else {
        H256::zero()
    };

    let storage = account
        .storage
        .iter()
        .map(|(k, v)| (*k, H256::from_uint(v)))
        .collect();

    PrestateAccountState {
        balance: Some(account.info.balance),
        nonce: account.info.nonce,
        code,
        code_hash,
        storage,
    }
}

/// Builds the diff-mode post entry for a touched account, emitting only fields whose
/// value differs from the pre-tx state. Storage entries are limited to slots that
/// actually changed and have a non-zero post value. Returns `None` if nothing changed.
fn build_post_output(
    addr: Address,
    pre_account: &LevmAccount,
    post_account: &LevmAccount,
    pre_snapshot: &CacheDB,
    db: &GeneralizedDatabase,
) -> Option<PrestateAccountState> {
    let mut state = PrestateAccountState::default();
    let mut modified = false;

    if pre_account.info.balance != post_account.info.balance {
        state.balance = Some(post_account.info.balance);
        modified = true;
    }
    if pre_account.info.nonce != post_account.info.nonce {
        state.nonce = post_account.info.nonce;
        modified = true;
    }
    if pre_account.info.code_hash != post_account.info.code_hash {
        if post_account.info.code_hash != *EMPTY_KECCACK_HASH {
            state.code_hash = post_account.info.code_hash;
            state.code = db
                .codes
                .get(&post_account.info.code_hash)
                .map(|c| c.bytecode.clone())
                .expect("code preloaded by preload_touched_codes");
        }
        modified = true;
    }

    for (key, post_val) in &post_account.storage {
        if post_val.is_zero() {
            continue;
        }
        let pre_val = pre_storage_value(addr, key, pre_snapshot, db).unwrap_or_default();
        if pre_val == *post_val {
            continue;
        }
        state.storage.insert(*key, H256::from_uint(post_val));
        modified = true;
    }

    modified.then_some(state)
}

/// Resolves the pre-tx value of `slot` for `addr`. Slots accessed in earlier txs are in
/// `pre_snapshot`; slots first loaded in this tx live only in `initial_accounts_state`.
fn pre_storage_value(
    addr: Address,
    slot: &H256,
    pre_snapshot: &CacheDB,
    db: &GeneralizedDatabase,
) -> Option<U256> {
    if let Some(account) = pre_snapshot.get(&addr)
        && let Some(value) = account.storage.get(slot)
    {
        return Some(*value);
    }
    db.initial_accounts_state
        .get(&addr)
        .and_then(|a| a.storage.get(slot).copied())
}

/// Builds the pre-tx state map. Pre storage is restricted to slots accessed by THIS
/// tx — for accounts cached before this tx that means slots first loaded here or slots
/// whose value changed; for accounts first accessed here, every slot in `post.storage`.
/// The final `post.storage` membership check is a defensive guard: it bounds pre to
/// the set of slots that ended up in the post cache, so unrelated slots that ever leak
/// into `initial_accounts_state` (e.g. via more eager caching upstream) cannot leak into
/// pre output.
fn build_pre_state_map(
    pre_snapshot: &CacheDB,
    post_cache: &CacheDB,
    db: &GeneralizedDatabase,
) -> PrestateTrace {
    let mut result = PrestateTrace::new();

    for (addr, pre_account, post_account) in find_touched_accounts(pre_snapshot, post_cache, db) {
        let mut state = build_account_output(pre_account, db);

        // For already-cached accounts, the pre-tx values of slots first loaded in this
        // tx live in `initial_accounts_state` rather than in `pre_snapshot`. Newly-accessed
        // accounts already have those values via `pre_account` (which comes from
        // `initial_accounts_state` in `find_touched_accounts`).
        if pre_snapshot.contains_key(&addr)
            && let Some(initial) = db.initial_accounts_state.get(&addr)
        {
            for (k, v) in &initial.storage {
                state
                    .storage
                    .entry(*k)
                    .or_insert_with(|| H256::from_uint(v));
            }
        }

        let pre_cached_storage = pre_snapshot.get(&addr).map(|a| &a.storage);
        state.storage.retain(|k, _| {
            if !post_account.storage.contains_key(k) {
                return false;
            }
            match pre_cached_storage {
                Some(pre) if pre.contains_key(k) => pre.get(k) != post_account.storage.get(k),
                _ => true,
            }
        });

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

/// Builds the diff-mode post map. Only accounts whose state actually changed are emitted,
/// destroyed accounts are dropped, and each entry carries only the fields that differ
/// from the pre-tx state.
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

        if let Some(state) = build_post_output(addr, pre_account, post_account, pre_snapshot, db) {
            result.insert(addr, state);
        }
    }

    result
}

/// Trims storage entries in a diff-mode pre map: drops slots whose pre value is zero
/// or whose pre value equals the post value (unchanged in this tx).
fn filter_diff_pre_storage(pre: &mut PrestateTrace, post_cache: &CacheDB) {
    for (addr, state) in pre.iter_mut() {
        let post_storage = post_cache.get(addr).map(|a| &a.storage);
        state.storage.retain(|k, v| {
            if v.is_zero() {
                return false;
            }
            let post_val = post_storage
                .and_then(|s| s.get(k).copied())
                .unwrap_or_default();
            *v != H256::from_uint(&post_val)
        });
    }
}

/// Returns the set of addresses whose state changed in this tx (i.e. would appear
/// in diff `post`). Used to prune diff `pre` to the same set, plus destroyed accounts
/// which appear only in `pre`.
fn modified_or_destroyed_addresses(
    pre_snapshot: &CacheDB,
    post_cache: &CacheDB,
    db: &GeneralizedDatabase,
) -> std::collections::HashSet<Address> {
    let mut set = std::collections::HashSet::new();
    for (addr, pre_account, post_account) in find_touched_accounts(pre_snapshot, post_cache, db) {
        if matches!(
            post_account.status,
            AccountStatus::Destroyed | AccountStatus::DestroyedModified,
        ) {
            set.insert(addr);
            continue;
        }
        if build_post_output(addr, pre_account, post_account, pre_snapshot, db).is_some() {
            set.insert(addr);
        }
    }
    set
}
