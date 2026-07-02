use ethrex_common::constants::EMPTY_KECCAK_HASH;
use ethrex_common::tracing::{PrePostState, PrestateAccountState, PrestateResult, PrestateTrace};
use ethrex_common::types::{Block, GenericTransaction, Transaction};
use ethrex_common::{
    Address, BigEndianHash, H256, U256,
    tracing::{CallTrace, CallTraceFrame, OpcodeTraceResult},
    types::BlockHeader,
};
use ethrex_crypto::Crypto;
use ethrex_levm::account::{AccountStatus, LevmAccount};
use ethrex_levm::db::gen_db::CacheDB;
use ethrex_levm::utils::get_base_fee_per_blob_gas;
use ethrex_levm::vm::VMType;
use ethrex_levm::{
    EVMConfig, Environment,
    db::gen_db::GeneralizedDatabase,
    tracing::{LevmCallTracer, LevmOpcodeTracer, OpcodeTracerConfig},
    vm::VM,
};

use crate::backends::levm::{
    adjust_disabled_base_fee, env_from_generic, generic_tx_to_transaction,
};
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
        let sender = tx
            .sender(crypto)
            .map_err(|e| EvmError::Transaction(format!("Couldn't recover sender: {e}")))?;
        let env = Self::setup_env(tx, sender, block_header, db, vm_type)?;
        Self::run_prestate_trace(db, env, tx, diff_mode, include_empty, vm_type, crypto)
    }

    /// `debug_traceCall` counterpart of [`Self::trace_tx_prestate`]: traces a synthetic
    /// `eth_call`-shaped request against `db` (which must already hold the target state).
    pub fn trace_call_prestate(
        db: &mut GeneralizedDatabase,
        block_header: &BlockHeader,
        tx: &GenericTransaction,
        diff_mode: bool,
        include_empty: bool,
        vm_type: VMType,
        crypto: &dyn Crypto,
    ) -> Result<PrestateResult, EvmError> {
        let (env, converted) = prepare_call_env(tx, block_header, db, vm_type)?;
        Self::run_prestate_trace(
            db,
            env,
            &converted,
            diff_mode,
            include_empty,
            vm_type,
            crypto,
        )
    }

    /// Runs `tx` with the prestateTracer over a prepared `env`, returning pre (and post,
    /// when `diff_mode`) account state. Shared by the tx and call entry points.
    fn run_prestate_trace(
        db: &mut GeneralizedDatabase,
        env: Environment,
        tx: &Transaction,
        diff_mode: bool,
        include_empty: bool,
        vm_type: VMType,
        crypto: &dyn Crypto,
    ) -> Result<PrestateResult, EvmError> {
        let pre_snapshot: CacheDB = db.current_accounts_state.clone();

        let mut vm = VM::new(env, db, tx, LevmCallTracer::disabled(), vm_type, crypto)?;
        vm.execute()?;

        preload_touched_codes(&pre_snapshot, db)?;

        let mut pre_map = build_pre_state_map(&pre_snapshot, &db.current_accounts_state, db)?;

        if diff_mode {
            let (post_map, kept) =
                build_post_state_map(&pre_snapshot, &db.current_accounts_state, db)?;
            filter_diff_pre_storage(&mut pre_map, &db.current_accounts_state);
            pre_map.retain(|addr, _| kept.contains(addr));
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

    /// Run transaction with opcode (EIP-3155) tracer activated.
    pub fn trace_tx_opcodes(
        db: &mut GeneralizedDatabase,
        block_header: &BlockHeader,
        tx: &Transaction,
        cfg: OpcodeTracerConfig,
        vm_type: VMType,
        crypto: &dyn Crypto,
    ) -> Result<OpcodeTraceResult, EvmError> {
        let env = Self::setup_env(
            tx,
            tx.sender(crypto).map_err(|error| {
                EvmError::Transaction(format!("Couldn't recover addresses with error: {error}"))
            })?,
            block_header,
            db,
            vm_type,
        )?;
        Self::run_opcode_trace(db, env, tx, cfg, vm_type, crypto)
    }

    /// `debug_traceCall` counterpart of [`Self::trace_tx_opcodes`].
    pub fn trace_call_opcodes(
        db: &mut GeneralizedDatabase,
        block_header: &BlockHeader,
        tx: &GenericTransaction,
        cfg: OpcodeTracerConfig,
        vm_type: VMType,
        crypto: &dyn Crypto,
    ) -> Result<OpcodeTraceResult, EvmError> {
        let (env, converted) = prepare_call_env(tx, block_header, db, vm_type)?;
        Self::run_opcode_trace(db, env, &converted, cfg, vm_type, crypto)
    }

    /// Runs `tx` with the opcode (EIP-3155) tracer over a prepared `env`. Shared by the
    /// tx and call entry points.
    fn run_opcode_trace(
        db: &mut GeneralizedDatabase,
        env: Environment,
        tx: &Transaction,
        cfg: OpcodeTracerConfig,
        vm_type: VMType,
        crypto: &dyn Crypto,
    ) -> Result<OpcodeTraceResult, EvmError> {
        let mut vm = VM::new(env, db, tx, LevmCallTracer::disabled(), vm_type, crypto)?;
        vm.opcode_tracer = LevmOpcodeTracer::new(cfg);
        vm.execute()?;
        Ok(vm.opcode_tracer.take_result())
    }

    /// Run transaction with callTracer activated. `log_index_base` is the number of logs
    /// emitted by preceding txs in the block, so `withLog` logs get geth's block-absolute
    /// `index` (pass 0 when there is no preceding context or logs aren't collected).
    #[allow(clippy::too_many_arguments)]
    pub fn trace_tx_calls(
        db: &mut GeneralizedDatabase,
        block_header: &BlockHeader,
        tx: &Transaction,
        only_top_call: bool,
        with_log: bool,
        log_index_base: u64,
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
        Self::run_call_trace(
            db,
            env,
            tx,
            only_top_call,
            with_log,
            log_index_base,
            vm_type,
            crypto,
        )
    }

    /// `debug_traceCall` counterpart of [`Self::trace_tx_calls`].
    #[allow(clippy::too_many_arguments)]
    pub fn trace_call_calls(
        db: &mut GeneralizedDatabase,
        block_header: &BlockHeader,
        tx: &GenericTransaction,
        only_top_call: bool,
        with_log: bool,
        log_index_base: u64,
        vm_type: VMType,
        crypto: &dyn Crypto,
    ) -> Result<CallTrace, EvmError> {
        let (env, converted) = prepare_call_env(tx, block_header, db, vm_type)?;
        Self::run_call_trace(
            db,
            env,
            &converted,
            only_top_call,
            with_log,
            log_index_base,
            vm_type,
            crypto,
        )
    }

    /// Runs `tx` with the callTracer over a prepared `env`. Shared by the tx and call
    /// entry points.
    #[allow(clippy::too_many_arguments)]
    fn run_call_trace(
        db: &mut GeneralizedDatabase,
        env: Environment,
        tx: &Transaction,
        only_top_call: bool,
        with_log: bool,
        log_index_base: u64,
        vm_type: VMType,
        crypto: &dyn Crypto,
    ) -> Result<CallTrace, EvmError> {
        let mut vm = VM::new(
            env,
            db,
            tx,
            LevmCallTracer::new(only_top_call, with_log, log_index_base),
            vm_type,
            crypto,
        )?;

        vm.execute()?;

        let callframe = vm.get_trace_result()?;

        // We only return the top call because a transaction only has one call with subcalls
        Ok(vec![callframe])
    }

    /// Traces every transaction in `block` with the callTracer, oldest to newest.
    /// `db` must already hold the block's parent state; this runs the block's system calls
    /// (beacon root, etc.) and then each transaction in order, accumulating state. The
    /// block-invariant `EVMConfig`/`chain_id`/blob fee are computed once and reused per tx
    /// (the single-tx entry points recompute them each call). Returns `(tx_hash, trace)`.
    pub fn trace_block_calls(
        db: &mut GeneralizedDatabase,
        block: &Block,
        only_top_call: bool,
        with_log: bool,
        vm_type: VMType,
        crypto: &dyn Crypto,
    ) -> Result<Vec<(H256, CallTrace)>, EvmError> {
        Self::rerun_block(db, block, Some(0), vm_type, crypto)?;
        let (config, chain_id, base_blob_fee) = block_trace_env_config(db, &block.header)?;
        let mut traces = Vec::with_capacity(block.body.transactions.len());
        // Running block-absolute log index: the whole block is traced from tx 0, so each
        // tx's logs continue where the previous tx left off (geth's cumulative `logSize`).
        let mut log_index_base = 0u64;
        for (tx, sender) in block
            .body
            .get_transactions_with_sender(crypto)
            .map_err(|e| EvmError::Transaction(e.to_string()))?
        {
            let env = Self::setup_env_with_config(
                tx,
                sender,
                &block.header,
                config,
                chain_id,
                vm_type,
                base_blob_fee,
            )?;
            let trace = Self::run_call_trace(
                db,
                env,
                tx,
                only_top_call,
                with_log,
                log_index_base,
                vm_type,
                crypto,
            )?;
            log_index_base = log_index_base.saturating_add(trace.iter().map(count_call_logs).sum());
            traces.push((tx.hash(crypto), trace));
        }
        Ok(traces)
    }

    /// Traces every transaction in `block` with the prestateTracer. See
    /// [`Self::trace_block_calls`] for the state/config-reuse semantics.
    pub fn trace_block_prestate(
        db: &mut GeneralizedDatabase,
        block: &Block,
        diff_mode: bool,
        include_empty: bool,
        vm_type: VMType,
        crypto: &dyn Crypto,
    ) -> Result<Vec<(H256, PrestateResult)>, EvmError> {
        Self::rerun_block(db, block, Some(0), vm_type, crypto)?;
        let (config, chain_id, base_blob_fee) = block_trace_env_config(db, &block.header)?;
        let mut traces = Vec::with_capacity(block.body.transactions.len());
        for (tx, sender) in block
            .body
            .get_transactions_with_sender(crypto)
            .map_err(|e| EvmError::Transaction(e.to_string()))?
        {
            let env = Self::setup_env_with_config(
                tx,
                sender,
                &block.header,
                config,
                chain_id,
                vm_type,
                base_blob_fee,
            )?;
            let trace =
                Self::run_prestate_trace(db, env, tx, diff_mode, include_empty, vm_type, crypto)?;
            traces.push((tx.hash(crypto), trace));
        }
        Ok(traces)
    }

    /// Traces every transaction in `block` with the opcode (EIP-3155) tracer. See
    /// [`Self::trace_block_calls`] for the state/config-reuse semantics.
    pub fn trace_block_opcodes(
        db: &mut GeneralizedDatabase,
        block: &Block,
        cfg: OpcodeTracerConfig,
        vm_type: VMType,
        crypto: &dyn Crypto,
    ) -> Result<Vec<(H256, OpcodeTraceResult)>, EvmError> {
        Self::rerun_block(db, block, Some(0), vm_type, crypto)?;
        let (config, chain_id, base_blob_fee) = block_trace_env_config(db, &block.header)?;
        let mut traces = Vec::with_capacity(block.body.transactions.len());
        for (tx, sender) in block
            .body
            .get_transactions_with_sender(crypto)
            .map_err(|e| EvmError::Transaction(e.to_string()))?
        {
            let env = Self::setup_env_with_config(
                tx,
                sender,
                &block.header,
                config,
                chain_id,
                vm_type,
                base_blob_fee,
            )?;
            let trace = Self::run_opcode_trace(db, env, tx, cfg.clone(), vm_type, crypto)?;
            traces.push((tx.hash(crypto), trace));
        }
        Ok(traces)
    }
}

/// Computes the block-invariant `(EVMConfig, chain_id, base_blob_fee)` once so a
/// whole-block trace can reuse them across transactions instead of recomputing per tx.
/// Recursively counts the logs captured in a call frame and all its subcalls — the
/// number of `withLog` logs a traced tx contributes to the block-absolute log index.
fn count_call_logs(frame: &CallTraceFrame) -> u64 {
    let own = u64::try_from(frame.logs.len()).unwrap_or(u64::MAX);
    frame.calls.iter().fold(own, |acc, subcall| {
        acc.saturating_add(count_call_logs(subcall))
    })
}

fn block_trace_env_config(
    db: &GeneralizedDatabase,
    header: &BlockHeader,
) -> Result<(EVMConfig, u64, U256), EvmError> {
    let chain_config = db.store.get_chain_config()?;
    let config = EVMConfig::new_from_chain_config(&chain_config, header);
    let base_blob_fee = get_base_fee_per_blob_gas(header.excess_blob_gas, &config)?;
    Ok((config, chain_config.chain_id, base_blob_fee))
}

/// Builds the call-style [`Environment`] and concrete [`Transaction`] for tracing a
/// synthetic `eth_call`-shaped request (`debug_traceCall`). Mirrors
/// `simulate_tx_from_generic`: the sender is taken from `tx.from` (no signature
/// recovery), the block gas limit is disabled, and the base fee is relaxed when no gas
/// price is provided.
fn prepare_call_env(
    tx: &GenericTransaction,
    block_header: &BlockHeader,
    db: &GeneralizedDatabase,
    vm_type: VMType,
) -> Result<(Environment, Transaction), EvmError> {
    let mut env = env_from_generic(tx, block_header, db, vm_type)?;
    env.block_gas_limit = i64::MAX as u64; // disable block gas limit
    adjust_disabled_base_fee(&mut env);
    let converted_tx = generic_tx_to_transaction(tx)?;
    Ok((env, converted_tx))
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
fn build_account_output(
    account: &LevmAccount,
    db: &GeneralizedDatabase,
) -> Result<PrestateAccountState, EvmError> {
    let has_code = account.info.code_hash != *EMPTY_KECCAK_HASH;
    let code = if has_code {
        get_preloaded_code(db, &account.info.code_hash)?
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

    Ok(PrestateAccountState {
        balance: Some(account.info.balance),
        nonce: account.info.nonce,
        code,
        code_hash,
        storage,
    })
}

/// Returns the bytecode for `hash`; caller must `preload_touched_codes` first.
fn get_preloaded_code(db: &GeneralizedDatabase, hash: &H256) -> Result<bytes::Bytes, EvmError> {
    db.codes
        .get(hash)
        .map(|c| c.code_bytes())
        .ok_or_else(|| EvmError::Custom(format!("missing preloaded code for {hash:?}")))
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
) -> Result<Option<PrestateAccountState>, EvmError> {
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
        if post_account.info.code_hash != *EMPTY_KECCAK_HASH {
            state.code_hash = post_account.info.code_hash;
            state.code = get_preloaded_code(db, &post_account.info.code_hash)?;
        }
        modified = true;
    }

    for (key, post_val) in &post_account.storage {
        let pre_val = pre_storage_value(addr, key, pre_snapshot, db).unwrap_or_default();
        if pre_val == *post_val {
            continue;
        }
        modified = true;
        // Cleared slots (post == 0) are encoded by absence in `post.storage`.
        if !post_val.is_zero() {
            state.storage.insert(*key, H256::from_uint(post_val));
        }
    }

    Ok(modified.then_some(state))
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
) -> Result<PrestateTrace, EvmError> {
    let mut result = PrestateTrace::new();

    for (addr, pre_account, post_account) in find_touched_accounts(pre_snapshot, post_cache, db) {
        let mut state = build_account_output(pre_account, db)?;

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

    Ok(result)
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
        .filter(|h| *h != *EMPTY_KECCAK_HASH)
        .collect();

    for hash in hashes {
        db.get_code(hash)?;
    }
    Ok(())
}

/// Builds the diff-mode post map and the set of modified-or-destroyed addresses
/// (used to prune diff `pre`) in a single pass.
fn build_post_state_map(
    pre_snapshot: &CacheDB,
    post_cache: &CacheDB,
    db: &GeneralizedDatabase,
) -> Result<(PrestateTrace, std::collections::HashSet<Address>), EvmError> {
    let mut post = PrestateTrace::new();
    let mut modified_or_destroyed = std::collections::HashSet::new();

    for (addr, pre_account, post_account) in find_touched_accounts(pre_snapshot, post_cache, db) {
        if matches!(
            post_account.status,
            AccountStatus::Destroyed | AccountStatus::DestroyedModified,
        ) {
            modified_or_destroyed.insert(addr);
            continue;
        }

        if let Some(state) = build_post_output(addr, pre_account, post_account, pre_snapshot, db)? {
            modified_or_destroyed.insert(addr);
            post.insert(addr, state);
        }
    }

    Ok((post, modified_or_destroyed))
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
