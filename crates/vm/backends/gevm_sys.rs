use std::os::raw::{c_char, c_void};

#[repr(C)]
pub struct GevmBlockEnv {
    pub beneficiary: [u8; 20],
    pub timestamp: [u8; 32],
    pub block_number: [u8; 32],
    pub gas_limit: [u8; 32],
    pub base_fee: [u8; 32],
    pub has_prevrandao: u8,
    pub prevrandao: [u8; 32],
    pub blob_gas_price: [u8; 32],
}

#[repr(C)]
pub struct GevmCfgEnv {
    pub chain_id: [u8; 32],
}

#[repr(C)]
pub struct GevmAccessListEntry {
    pub address: [u8; 20],
    pub storage_keys: *const u8, // packed n_keys * 32 bytes
    pub n_keys: usize,
}

#[repr(C)]
pub struct GevmAuthorization {
    pub chain_id: [u8; 32],
    pub address: [u8; 20],
    pub nonce: u64,
    pub y_parity: u8,
    pub r: [u8; 32],
    pub s: [u8; 32],
}

#[repr(C)]
pub struct GevmTxInput {
    pub kind: u8,
    pub tx_type: u8,
    pub caller: [u8; 20],
    pub to: [u8; 20],
    pub value: [u8; 32],
    pub input: *const u8,
    pub input_len: usize,
    pub gas_limit: u64,
    pub gas_price: [u8; 32],
    pub max_fee_per_gas: [u8; 32],
    pub max_priority_fee_per_gas: [u8; 32],
    pub max_fee_per_blob_gas: [u8; 32],
    pub nonce: u64,
    pub access_list: *const GevmAccessListEntry,
    pub n_access_entries: usize,
    pub blob_hashes: *const u8, // packed n * 32 bytes
    pub n_blob_hashes: usize,
    pub auth_list: *const GevmAuthorization,
    pub n_auth_entries: usize,
}

#[repr(C)]
pub struct GevmLog {
    pub address: [u8; 20],
    pub topics: [[u8; 32]; 4],
    pub n_topics: u8,
    pub data: *const u8,
    pub data_len: usize,
}

#[repr(C)]
pub struct GevmStorageEntry {
    pub key: [u8; 32],
    pub value: [u8; 32],
}

#[repr(C)]
pub struct GevmAccountUpdate {
    pub address: [u8; 20],
    pub removed: u8,
    pub has_info: u8,
    pub balance: [u8; 32],
    pub nonce: u64,
    pub code_hash: [u8; 32],
    pub code: *const u8,
    pub code_len: usize,
    pub storage: *const GevmStorageEntry,
    pub n_storage: usize,
}

#[repr(C)]
pub struct GevmExecResult {
    pub status: u8,
    pub gas_used: u64,
    pub gas_refund: i64,
    pub output: *const u8,
    pub output_len: usize,
    pub logs: *const GevmLog,
    pub n_logs: usize,
    pub has_created_addr: u8,
    pub created_addr: [u8; 20],
    pub updates: *const GevmAccountUpdate,
    pub n_updates: usize,
    pub is_validation_error: u8,
    pub error_msg: *const c_char,
}

// Callback function pointer types
pub type GevmBasicFn = unsafe extern "C" fn(
    handle: *mut c_void,
    addr: *const [u8; 20],
    balance_out: *mut [u8; 32],
    nonce_out: *mut u64,
    code_hash_out: *mut [u8; 32],
    exists_out: *mut i32,
) -> i32;

pub type GevmCodeByHashFn = unsafe extern "C" fn(
    handle: *mut c_void,
    code_hash: *const [u8; 32],
    code_out: *mut *mut u8,
    len_out: *mut usize,
) -> i32;

pub type GevmStorageFn = unsafe extern "C" fn(
    handle: *mut c_void,
    addr: *const [u8; 20],
    key: *const [u8; 32],
    value_out: *mut [u8; 32],
) -> i32;

pub type GevmHasStorageFn = unsafe extern "C" fn(
    handle: *mut c_void,
    addr: *const [u8; 20],
    has_storage_out: *mut i32,
) -> i32;

pub type GevmBlockHashFn = unsafe extern "C" fn(
    handle: *mut c_void,
    block_number: u64,
    hash_out: *mut [u8; 32],
) -> i32;

unsafe extern "C" {
    pub fn gevm_execute(
        fork_id: u8,
        block_env: *const GevmBlockEnv,
        cfg_env: *const GevmCfgEnv,
        tx: *const GevmTxInput,
        db_handle: *mut c_void,
        basic_cb: GevmBasicFn,
        code_by_hash_cb: GevmCodeByHashFn,
        storage_cb: GevmStorageFn,
        has_storage_cb: GevmHasStorageFn,
        block_hash_cb: GevmBlockHashFn,
    ) -> *mut GevmExecResult;

    pub fn gevm_free_result(result: *mut GevmExecResult);
}
