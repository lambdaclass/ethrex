#![allow(unsafe_code)]
#![allow(unsafe_op_in_unsafe_fn)]

use super::BlockExecutionResult;
use super::gevm_sys::{
    GevmAccessListEntry, GevmAuthorization, GevmBlockEnv, GevmCfgEnv, GevmExecResult, GevmLog,
    GevmTxInput, gevm_execute, gevm_free_result,
};
use super::levm::{LEVM, extract_all_requests_levm};
use crate::EvmError;
use bytes::Bytes;
use ethrex_common::constants::EMPTY_KECCACK_HASH;
use ethrex_common::types::block_access_list::BlockAccessList;
use ethrex_common::types::{
    AccountUpdate, Block, BlockHeader, Fork, Log, Receipt, Transaction, TxKind, TxType,
};
use ethrex_common::utils::{u256_from_big_endian_const, u256_to_big_endian};
use ethrex_common::{Address, H256, U256};
use ethrex_levm::EVMConfig;
use ethrex_levm::db::gen_db::GeneralizedDatabase;
use ethrex_levm::errors::{ExecutionReport, ExceptionalHalt, TxResult, VMError};
use ethrex_levm::utils::get_base_fee_per_blob_gas;
use ethrex_levm::vm::VMType;
use std::os::raw::c_void;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::Sender;

#[derive(Debug)]
pub struct GEVM;

// ======================== Type conversion helpers ========================

fn u256_to_be32(v: U256) -> [u8; 32] {
    u256_to_big_endian(v)
}

fn be32_to_u256(b: &[u8; 32]) -> U256 {
    u256_from_big_endian_const(*b)
}

fn fork_to_gevm_id(fork: Fork) -> u8 {
    // ethrex Fork discriminants 0-19 match gevm ForkIDs 0-19 exactly.
    // BPO1-BPO5 (ethrex 20-24) and Amsterdam (ethrex 25) cap to Osaka (19)
    // because gevm's gasParamsCache only covers indices 0-19.
    let id = fork as u8;
    id.min(19)
}

fn compute_blob_gas_price(block_header: &BlockHeader, db: &GeneralizedDatabase) -> U256 {
    let chain_config = match db.store.get_chain_config() {
        Ok(c) => c,
        Err(_) => return U256::zero(),
    };
    let config = EVMConfig::new_from_chain_config(&chain_config, block_header);
    let excess_blob_gas = block_header.excess_blob_gas.map(U256::from);
    get_base_fee_per_blob_gas(excess_blob_gas, &config).unwrap_or(U256::zero())
}

// ======================== DB Callbacks ========================

unsafe extern "C" fn basic_cb(
    handle: *mut c_void,
    addr: *const [u8; 20],
    balance_out: *mut [u8; 32],
    nonce_out: *mut u64,
    code_hash_out: *mut [u8; 32],
    exists_out: *mut i32,
) -> i32 {
    let db = &mut *(handle as *mut GeneralizedDatabase);
    let address = Address::from(*addr);

    match db.get_account(address) {
        Ok(acc) => {
            *exists_out = if acc.info.balance == U256::zero()
                && acc.info.nonce == 0
                && acc.info.code_hash == *EMPTY_KECCACK_HASH
            {
                0
            } else {
                1
            };
            *balance_out = u256_to_be32(acc.info.balance);
            *nonce_out = acc.info.nonce;
            *code_hash_out = acc.info.code_hash.0;
            0
        }
        Err(_) => {
            *exists_out = 0;
            *balance_out = [0u8; 32];
            *nonce_out = 0;
            *code_hash_out = EMPTY_KECCACK_HASH.0;
            0
        }
    }
}

unsafe extern "C" fn code_by_hash_cb(
    handle: *mut c_void,
    code_hash: *const [u8; 32],
    code_out: *mut *mut u8,
    len_out: *mut usize,
) -> i32 {
    let db = &mut *(handle as *mut GeneralizedDatabase);
    let hash = H256::from(*code_hash);

    match db.get_code(hash) {
        Ok(code) => {
            let bytes = &code.bytecode;
            if bytes.is_empty() {
                *code_out = std::ptr::null_mut();
                *len_out = 0;
                return 0;
            }
            // Allocate memory for code (Go will free it with C.free)
            let ptr = libc::malloc(bytes.len()) as *mut u8;
            if ptr.is_null() {
                return -1;
            }
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr, bytes.len());
            *code_out = ptr;
            *len_out = bytes.len();
            0
        }
        Err(_) => {
            *code_out = std::ptr::null_mut();
            *len_out = 0;
            0
        }
    }
}

unsafe extern "C" fn storage_cb(
    handle: *mut c_void,
    addr: *const [u8; 20],
    key: *const [u8; 32],
    value_out: *mut [u8; 32],
) -> i32 {
    let db = &mut *(handle as *mut GeneralizedDatabase);
    let address = Address::from(*addr);
    let key_h256 = H256::from(*key);

    // Check current_accounts_state cache first
    if let Some(account) = db.current_accounts_state.get(&address) {
        if let Some(value) = account.storage.get(&key_h256) {
            *value_out = u256_to_be32(*value);
            return 0;
        }
    }
    // Check initial_accounts_state cache
    if let Some(account) = db.initial_accounts_state.get(&address) {
        if let Some(value) = account.storage.get(&key_h256) {
            *value_out = u256_to_be32(*value);
            return 0;
        }
    }

    // Fall back to store
    match db.store.get_storage_value(address, key_h256) {
        Ok(value) => {
            *value_out = u256_to_be32(value);
            // Cache in initial state for subsequent reads
            if let Some(account) = db.initial_accounts_state.get_mut(&address) {
                account.storage.insert(key_h256, value);
            }
            0
        }
        Err(_) => {
            *value_out = [0u8; 32];
            -1
        }
    }
}

unsafe extern "C" fn has_storage_cb(
    handle: *mut c_void,
    addr: *const [u8; 20],
    has_storage_out: *mut i32,
) -> i32 {
    let db = &mut *(handle as *mut GeneralizedDatabase);
    let address = Address::from(*addr);

    // Check cache first
    if let Some(acc) = db.current_accounts_state.get(&address) {
        *has_storage_out = i32::from(acc.has_storage);
        return 0;
    }
    if let Some(acc) = db.initial_accounts_state.get(&address) {
        *has_storage_out = i32::from(acc.has_storage);
        return 0;
    }

    // Check store via account state
    match db.store.get_account_state(address) {
        Ok(state) => {
            use ethrex_common::constants::EMPTY_TRIE_HASH;
            *has_storage_out = i32::from(state.storage_root != *EMPTY_TRIE_HASH);
            0
        }
        Err(_) => {
            *has_storage_out = 0;
            -1
        }
    }
}

unsafe extern "C" fn block_hash_cb(
    handle: *mut c_void,
    block_number: u64,
    hash_out: *mut [u8; 32],
) -> i32 {
    let db = &*(handle as *const GeneralizedDatabase);

    match db.store.get_block_hash(block_number) {
        Ok(hash) => {
            *hash_out = hash.0;
            0
        }
        Err(_) => {
            *hash_out = [0u8; 32];
            -1
        }
    }
}

// ======================== Transaction Input Building ========================

/// Holds the allocated C-compatible data for a transaction so the memory
/// stays alive for the duration of the FFI call.
struct TxCData {
    // The actual input struct
    pub tx_input: GevmTxInput,
    // Buffers that are pointed to by tx_input - must outlive tx_input
    #[allow(dead_code)]
    pub input_data: Vec<u8>,
    #[allow(dead_code)]
    pub access_list_entries: Vec<GevmAccessListEntry>,
    // Storage key bytes: one flat Vec per access list entry
    #[allow(dead_code)]
    pub access_list_key_bufs: Vec<Vec<u8>>,
    #[allow(dead_code)]
    pub auth_list_entries: Vec<GevmAuthorization>,
    #[allow(dead_code)]
    pub blob_hashes_packed: Vec<u8>,
}

fn build_tx_c_data(tx: &Transaction, tx_sender: Address) -> Result<TxCData, EvmError> {
    let tx_type: u8 = match tx.tx_type() {
        TxType::Legacy => 0,
        TxType::EIP2930 => 1,
        TxType::EIP1559 => 2,
        TxType::EIP4844 => 3,
        TxType::EIP7702 => 4,
        // L2-specific transactions are treated as EIP1559 for gevm
        TxType::Privileged => 2,
        TxType::FeeToken => 2,
    };

    // kind byte: 0 = call, 1 = create
    let tx_kind_byte: u8 = if tx.is_contract_creation() { 1 } else { 0 };

    let to = match tx.to() {
        TxKind::Call(addr) => addr.0,
        TxKind::Create => [0u8; 20],
    };

    let input_data: Vec<u8> = tx.data().to_vec();

    let gas_price = u256_to_be32(tx.gas_price());
    let max_fee_per_gas = u256_to_be32(
        tx.max_fee_per_gas()
            .map(U256::from)
            .unwrap_or(tx.gas_price()),
    );
    let max_priority_fee_per_gas = u256_to_be32(
        tx.max_priority_fee()
            .map(U256::from)
            .unwrap_or(tx.gas_price()),
    );
    let max_fee_per_blob_gas = u256_to_be32(tx.max_fee_per_blob_gas().unwrap_or_default());

    // Build access list
    let raw_access_list = tx.access_list();
    let mut access_list_key_bufs: Vec<Vec<u8>> = Vec::with_capacity(raw_access_list.len());
    let mut access_list_entries: Vec<GevmAccessListEntry> =
        Vec::with_capacity(raw_access_list.len());

    for (addr, keys) in raw_access_list {
        let mut key_buf: Vec<u8> = Vec::with_capacity(keys.len() * 32);
        for key in keys {
            key_buf.extend_from_slice(&key.0);
        }
        access_list_entries.push(GevmAccessListEntry {
            address: addr.0,
            storage_keys: if key_buf.is_empty() {
                std::ptr::null()
            } else {
                key_buf.as_ptr()
            },
            n_keys: keys.len(),
        });
        access_list_key_bufs.push(key_buf);
    }

    // Build auth list (EIP-7702)
    let mut auth_list_entries: Vec<GevmAuthorization> = Vec::new();
    if let Some(auth_list) = tx.authorization_list() {
        for auth in auth_list {
            let chain_id_bytes = u256_to_be32(auth.chain_id);
            let r_bytes = u256_to_be32(auth.r_signature);
            let s_bytes = u256_to_be32(auth.s_signature);
            #[allow(clippy::cast_possible_truncation)]
            let y_parity = auth.y_parity.as_u64() as u8;
            auth_list_entries.push(GevmAuthorization {
                chain_id: chain_id_bytes,
                address: auth.address.0,
                nonce: auth.nonce,
                y_parity,
                r: r_bytes,
                s: s_bytes,
            });
        }
    }

    // Build blob hashes
    let blob_hashes = tx.blob_versioned_hashes();
    let mut blob_hashes_packed: Vec<u8> = Vec::with_capacity(blob_hashes.len() * 32);
    for bh in &blob_hashes {
        blob_hashes_packed.extend_from_slice(&bh.0);
    }

    let n_access_entries = access_list_entries.len();
    let access_list_ptr = if access_list_entries.is_empty() {
        std::ptr::null()
    } else {
        access_list_entries.as_ptr()
    };

    let n_auth_entries = auth_list_entries.len();
    let auth_list_ptr = if auth_list_entries.is_empty() {
        std::ptr::null()
    } else {
        auth_list_entries.as_ptr()
    };

    let n_blob_hashes = blob_hashes.len();
    let blob_hashes_ptr = if blob_hashes_packed.is_empty() {
        std::ptr::null()
    } else {
        blob_hashes_packed.as_ptr()
    };

    let input_ptr = if input_data.is_empty() {
        std::ptr::null()
    } else {
        input_data.as_ptr()
    };

    let tx_input = GevmTxInput {
        kind: tx_kind_byte,
        tx_type,
        caller: tx_sender.0,
        to,
        value: u256_to_be32(tx.value()),
        input: input_ptr,
        input_len: input_data.len(),
        gas_limit: tx.gas_limit(),
        gas_price,
        max_fee_per_gas,
        max_priority_fee_per_gas,
        max_fee_per_blob_gas,
        nonce: tx.nonce(),
        access_list: access_list_ptr,
        n_access_entries,
        blob_hashes: blob_hashes_ptr,
        n_blob_hashes,
        auth_list: auth_list_ptr,
        n_auth_entries,
    };

    Ok(TxCData {
        tx_input,
        input_data,
        access_list_entries,
        access_list_key_bufs,
        auth_list_entries,
        blob_hashes_packed,
    })
}

// ======================== Result Conversion ========================

fn convert_logs(result: &GevmExecResult) -> Vec<Log> {
    if result.n_logs == 0 || result.logs.is_null() {
        return Vec::new();
    }

    let raw_logs: &[GevmLog] =
        unsafe { std::slice::from_raw_parts(result.logs, result.n_logs) };

    raw_logs
        .iter()
        .map(|log| {
            let data = if log.data_len == 0 || log.data.is_null() {
                Bytes::new()
            } else {
                let slice = unsafe { std::slice::from_raw_parts(log.data, log.data_len) };
                Bytes::copy_from_slice(slice)
            };

            #[allow(clippy::cast_possible_truncation)]
            let n_topics = log.n_topics as usize;
            let topics: Vec<H256> = (0..n_topics)
                .map(|i| H256::from(log.topics[i]))
                .collect();

            Log {
                address: Address::from(log.address),
                topics,
                data,
            }
        })
        .collect()
}

fn convert_result_to_report(
    result: &GevmExecResult,
    is_amsterdam: bool,
) -> Result<ExecutionReport, EvmError> {
    let output = if result.output_len == 0 || result.output.is_null() {
        Bytes::new()
    } else {
        let slice = unsafe { std::slice::from_raw_parts(result.output, result.output_len) };
        Bytes::copy_from_slice(slice)
    };

    let logs = convert_logs(result);

    // status: 0 = success, 1 = revert, 2 = halt/other error
    let tx_result = match result.status {
        0 => TxResult::Success,
        1 => TxResult::Revert(VMError::RevertOpcode),
        _ => TxResult::Revert(VMError::ExceptionalHalt(ExceptionalHalt::OutOfGas)),
    };

    // gevm's gas accounting:
    //   result.gas_used   = gas.Used() = gas.Spent() - gas.Refunded() = POST-REFUND
    //   result.gas_refund = gas.Refunded() (capped by EIP-3529)
    //
    // LEVM's ExecutionReport semantics (per errors.rs docs):
    //   gas_used:    PRE-REFUND for Amsterdam+ (EIP-7778), POST-REFUND for pre-Amsterdam
    //   gas_spent:   always POST-REFUND
    //   gas_refunded: the refund amount
    #[allow(clippy::cast_sign_loss)]
    let gas_refunded = result.gas_refund.max(0) as u64;
    // gevm returns post-refund gas in gas_used; reconstruct pre-refund by adding back refund
    let gas_post_refund = result.gas_used;
    let gas_pre_refund = gas_post_refund + gas_refunded;

    // For block accounting (gas_used field):
    //   - Amsterdam+: PRE-REFUND (EIP-7778 changes block gas_used to exclude refunds)
    //   - Pre-Amsterdam: POST-REFUND (traditional semantics)
    let gas_used = if is_amsterdam { gas_pre_refund } else { gas_post_refund };
    let gas_spent = gas_post_refund;

    Ok(ExecutionReport {
        result: tx_result,
        gas_used,
        gas_spent,
        gas_refunded,
        output,
        logs,
    })
}

// ======================== Account Update Application ========================

fn apply_account_updates(
    result: &GevmExecResult,
    db: &mut GeneralizedDatabase,
) -> Result<(), EvmError> {
    if result.n_updates == 0 || result.updates.is_null() {
        return Ok(());
    }

    let updates =
        unsafe { std::slice::from_raw_parts(result.updates, result.n_updates) };

    for update in updates {
        let address = Address::from(update.address);

        if update.removed != 0 {
            // Selfdestruct: mirror what LEVM's default_hook does —
            // reset the account to the empty default and mark destroyed.
            // This ensures get_state_transitions sees is_empty()=true and
            // emits a proper removed=true AccountUpdate.
            if let Ok(acc) = db.get_account_mut(address) {
                acc.info.balance = U256::zero();
                acc.info.nonce = 0;
                acc.info.code_hash = *EMPTY_KECCACK_HASH;
                acc.storage.clear();
                acc.has_storage = false;
                acc.mark_destroyed();
            }
            // get_state_transitions checks codes[new_code_hash] when code_hash changes.
            // We changed code_hash to EMPTY_KECCAK, so ensure it's in the codes map.
            use ethrex_common::types::Code;
            db.codes
                .entry(*EMPTY_KECCACK_HASH)
                .or_insert_with(|| Code::from_bytecode_unchecked(Bytes::new(), *EMPTY_KECCACK_HASH));
            continue;
        }

        if update.has_info != 0 {
            let acc = db.get_account_mut(address).map_err(|e| {
                EvmError::DB(format!("Failed to get account {address}: {e}"))
            })?;
            acc.info.balance = be32_to_u256(&update.balance);
            acc.info.nonce = update.nonce;
            acc.info.code_hash = H256::from(update.code_hash);

            // If code is provided, insert into db.codes
            if update.code_len > 0 && !update.code.is_null() {
                let code_bytes =
                    unsafe { std::slice::from_raw_parts(update.code, update.code_len) };
                use ethrex_common::types::Code;
                let code_hash = H256::from(update.code_hash);
                let code = Code::from_bytecode_unchecked(
                    Bytes::copy_from_slice(code_bytes),
                    code_hash,
                );
                db.codes.insert(code_hash, code);
            }
        }

        // Apply storage updates
        if update.n_storage > 0 && !update.storage.is_null() {
            let storage =
                unsafe { std::slice::from_raw_parts(update.storage, update.n_storage) };

            // Ensure account is loaded into both caches before modifying storage.
            // get_account_mut loads it into initial_accounts_state on first call.
            db.get_account_mut(address).map_err(|e| {
                EvmError::DB(format!("Failed to get account {address} for storage: {e}"))
            })?;

            for slot in storage {
                let key = H256::from(slot.key);
                let value = be32_to_u256(&slot.value);

                // Ensure the key exists in initial_accounts_state.storage.
                // get_state_transitions() diffs current vs initial per-slot, so every
                // key written to current_accounts_state must have a corresponding entry
                // in initial_accounts_state (even if the slot was never read through
                // storage_cb, e.g. for newly-created contracts doing SSTORE).
                if db
                    .initial_accounts_state
                    .get(&address)
                    .map_or(true, |a| !a.storage.contains_key(&key))
                {
                    let orig = db
                        .store
                        .get_storage_value(address, key)
                        .unwrap_or_default();
                    if let Some(init_acc) = db.initial_accounts_state.get_mut(&address) {
                        init_acc.storage.insert(key, orig);
                    }
                }

                // Write new value to current state.
                if let Some(curr_acc) = db.current_accounts_state.get_mut(&address) {
                    curr_acc.storage.insert(key, value);
                }
            }
        }
    }
    Ok(())
}

// ======================== GEVM Implementation ========================

impl GEVM {
    pub fn execute_tx(
        tx: &Transaction,
        tx_sender: Address,
        block_header: &BlockHeader,
        db: &mut GeneralizedDatabase,
        vm_type: VMType,
    ) -> Result<ExecutionReport, EvmError> {
        // Disallow L2-specific behavior: delegate entirely to LEVM for L2
        // (gevm doesn't know about L2 fee tokens etc.)
        if let VMType::L2(_) = vm_type {
            return LEVM::execute_tx(tx, tx_sender, block_header, db, vm_type);
        }

        let chain_config = db.store.get_chain_config()?;
        let fork = chain_config.fork(block_header.timestamp);
        let fork_id = fork_to_gevm_id(fork);
        let is_amsterdam = chain_config.is_amsterdam_activated(block_header.timestamp);

        let block_env = GevmBlockEnv {
            beneficiary: block_header.coinbase.0,
            timestamp: u256_to_be32(U256::from(block_header.timestamp)),
            block_number: u256_to_be32(U256::from(block_header.number)),
            gas_limit: u256_to_be32(U256::from(block_header.gas_limit)),
            base_fee: u256_to_be32(block_header.base_fee_per_gas.unwrap_or_default().into()),
            has_prevrandao: 1,
            prevrandao: block_header.prev_randao.0,
            blob_gas_price: u256_to_be32(compute_blob_gas_price(block_header, db)),
        };

        let cfg_env = GevmCfgEnv {
            chain_id: u256_to_be32(U256::from(chain_config.chain_id)),
        };

        // Build the C-compatible tx data (keep alive for entire FFI call)
        let tx_c_data = build_tx_c_data(tx, tx_sender)?;

        let db_handle = db as *mut GeneralizedDatabase as *mut c_void;

        let result_ptr = unsafe {
            gevm_execute(
                fork_id,
                &block_env,
                &cfg_env,
                &tx_c_data.tx_input,
                db_handle,
                basic_cb,
                code_by_hash_cb,
                storage_cb,
                has_storage_cb,
                block_hash_cb,
            )
        };

        if result_ptr.is_null() {
            return Err(EvmError::Custom(
                "gevm_execute returned null".to_string(),
            ));
        }

        let result = unsafe { &*result_ptr };

        // Check for validation errors
        if result.is_validation_error != 0 {
            let msg = if !result.error_msg.is_null() {
                unsafe {
                    std::ffi::CStr::from_ptr(result.error_msg)
                        .to_string_lossy()
                        .into_owned()
                }
            } else {
                "validation error".to_string()
            };
            unsafe { gevm_free_result(result_ptr) };
            return Err(EvmError::Transaction(msg));
        }

        let report = convert_result_to_report(result, is_amsterdam);
        let apply_result = apply_account_updates(result, db);

        // Always free the result, even on error paths
        unsafe { gevm_free_result(result_ptr) };

        let report = report?;
        apply_result?;

        Ok(report)
    }

    pub fn execute_block(
        block: &Block,
        db: &mut GeneralizedDatabase,
        vm_type: VMType,
    ) -> Result<(BlockExecutionResult, Option<BlockAccessList>), EvmError> {
        let chain_config = db.store.get_chain_config()?;
        let record_bal = chain_config.is_amsterdam_activated(block.header.timestamp);

        if record_bal {
            db.enable_bal_recording();
            db.set_bal_index(0);
        }

        // System calls still use LEVM
        LEVM::prepare_block(block, db, vm_type)?;

        let mut receipts = Vec::new();
        let mut cumulative_gas_used = 0_u64;
        let mut block_gas_used = 0_u64;

        let transactions_with_sender =
            block.body.get_transactions_with_sender().map_err(|error| {
                EvmError::Transaction(format!("Couldn't recover addresses with error: {error}"))
            })?;

        for (tx_idx, (tx, tx_sender)) in transactions_with_sender.into_iter().enumerate() {
            // Check gas limit before executing
            if block_gas_used + tx.gas_limit() > block.header.gas_limit {
                return Err(EvmError::Transaction(format!(
                    "Gas allowance exceeded: Block gas used overflow: \
                     used {block_gas_used} + tx limit {} > block limit {}",
                    tx.gas_limit(),
                    block.header.gas_limit
                )));
            }

            if record_bal {
                #[allow(clippy::cast_possible_truncation)]
                db.set_bal_index((tx_idx + 1) as u16);

                if let Some(recorder) = db.bal_recorder_mut() {
                    recorder.record_touched_address(tx_sender);
                    if let TxKind::Call(to) = tx.to() {
                        recorder.record_touched_address(to);
                    }
                }
            }

            let report = Self::execute_tx(tx, tx_sender, &block.header, db, vm_type)?;

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

        if record_bal {
            #[allow(clippy::cast_possible_truncation)]
            let withdrawal_index = (block.body.transactions.len() + 1) as u16;
            db.set_bal_index(withdrawal_index);
        }

        if let Some(withdrawals) = &block.body.withdrawals {
            if record_bal {
                if let Some(recorder) = db.bal_recorder_mut() {
                    recorder.extend_touched_addresses(withdrawals.iter().map(|w| w.address));
                }
            }
            LEVM::process_withdrawals(db, withdrawals)?;
        }

        let requests = match vm_type {
            VMType::L1 => extract_all_requests_levm(&receipts, db, &block.header, vm_type)?,
            VMType::L2(_) => Default::default(),
        };

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
        // Execute block (gevm doesn't do incremental merkleization)
        let result = Self::execute_block(block, db, vm_type)?;
        // Send accumulated state transitions to the merkleizer pipeline
        let transitions = LEVM::get_state_transitions_tx(db)?;
        merkleizer
            .send(transitions)
            .map_err(|e| EvmError::Custom(format!("merkleizer send failed: {e}")))?;
        queue_length.fetch_add(1, Ordering::Relaxed);
        Ok(result)
    }
}
