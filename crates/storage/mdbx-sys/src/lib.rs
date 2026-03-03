//! Raw FFI bindings to libmdbx v0.13.x
//!
//! This crate vendors the libmdbx amalgamation (mdbx.c + mdbx.h) and exposes
//! the minimum C API surface needed by the `ethrex-mdbx` safe wrapper.

#![allow(non_camel_case_types, non_upper_case_globals, clippy::upper_case_acronyms)]

use std::os::raw::{c_char, c_int, c_void};

// ---------------------------------------------------------------------------
// Opaque types
// ---------------------------------------------------------------------------

/// Opaque environment handle.
pub enum MDBX_env {}

/// Opaque transaction handle.
pub enum MDBX_txn {}

/// Opaque cursor handle.
pub enum MDBX_cursor {}

/// Database handle (index into the environment's table array).
pub type MDBX_dbi = u32;

// ---------------------------------------------------------------------------
// MDBX_val — key/data slice passed to libmdbx
// ---------------------------------------------------------------------------

/// A value buffer. Mirrors `struct iovec` on unix / custom struct on Solaris.
#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct MDBX_val {
    /// Pointer to the data.
    pub iov_base: *mut c_void,
    /// Length of the data in bytes.
    pub iov_len: usize,
}

impl MDBX_val {
    /// Create a val from a byte slice (for keys/values we pass to libmdbx).
    pub fn from_slice(s: &[u8]) -> Self {
        MDBX_val {
            iov_base: s.as_ptr() as *mut c_void,
            iov_len: s.len(),
        }
    }

    /// Create an empty val (used as output parameter).
    pub fn empty() -> Self {
        MDBX_val {
            iov_base: std::ptr::null_mut(),
            iov_len: 0,
        }
    }

    /// View the val as a byte slice. Caller must ensure the pointer is valid.
    ///
    /// # Safety
    /// The pointer must be valid for `iov_len` bytes and must not be null
    /// (unless iov_len is 0).
    pub unsafe fn as_slice(&self) -> &[u8] {
        if self.iov_len == 0 {
            &[]
        } else {
            unsafe { std::slice::from_raw_parts(self.iov_base as *const u8, self.iov_len) }
        }
    }
}

// ---------------------------------------------------------------------------
// Environment flags (MDBX_env_flags_t)
// ---------------------------------------------------------------------------

pub type MDBX_env_flags_t = u32;

pub const MDBX_ENV_DEFAULTS: MDBX_env_flags_t = 0;
pub const MDBX_NOSUBDIR: MDBX_env_flags_t = 0x4000;
pub const MDBX_RDONLY: MDBX_env_flags_t = 0x20000;
pub const MDBX_EXCLUSIVE: MDBX_env_flags_t = 0x400000;
pub const MDBX_WRITEMAP: MDBX_env_flags_t = 0x80000;
pub const MDBX_NORDAHEAD: MDBX_env_flags_t = 0x800000;
pub const MDBX_NOSTICKYTHREADS: MDBX_env_flags_t = 0x200000;
pub const MDBX_NOMEMINIT: MDBX_env_flags_t = 0x1000000;
pub const MDBX_COALESCE: MDBX_env_flags_t = 0x2000000;
pub const MDBX_LIFORECLAIM: MDBX_env_flags_t = 0x4000000;
pub const MDBX_SYNC_DURABLE: MDBX_env_flags_t = 0;
pub const MDBX_NOMETASYNC: MDBX_env_flags_t = 0x40000;
pub const MDBX_SAFE_NOSYNC: MDBX_env_flags_t = 0x10000;

// ---------------------------------------------------------------------------
// Transaction flags (MDBX_txn_flags_t)
// ---------------------------------------------------------------------------

pub type MDBX_txn_flags_t = u32;

pub const MDBX_TXN_READWRITE: MDBX_txn_flags_t = 0;
pub const MDBX_TXN_RDONLY: MDBX_txn_flags_t = MDBX_RDONLY;

// ---------------------------------------------------------------------------
// Database/table flags (MDBX_db_flags_t)
// ---------------------------------------------------------------------------

pub type MDBX_db_flags_t = u32;

pub const MDBX_DB_DEFAULTS: MDBX_db_flags_t = 0;
pub const MDBX_CREATE: MDBX_db_flags_t = 0x40000;

// ---------------------------------------------------------------------------
// Put flags (MDBX_put_flags_t)
// ---------------------------------------------------------------------------

pub type MDBX_put_flags_t = u32;

pub const MDBX_UPSERT: MDBX_put_flags_t = 0;

// ---------------------------------------------------------------------------
// Copy flags (MDBX_copy_flags_t)
// ---------------------------------------------------------------------------

pub type MDBX_copy_flags_t = u32;

pub const MDBX_CP_DEFAULTS: MDBX_copy_flags_t = 0;
pub const MDBX_CP_COMPACT: MDBX_copy_flags_t = 1;

// ---------------------------------------------------------------------------
// Cursor operations (MDBX_cursor_op)
// ---------------------------------------------------------------------------

pub type MDBX_cursor_op = c_int;

pub const MDBX_FIRST: MDBX_cursor_op = 0;
pub const MDBX_NEXT: MDBX_cursor_op = 8;
pub const MDBX_SET_RANGE: MDBX_cursor_op = 17;

// ---------------------------------------------------------------------------
// Options (MDBX_option_t)
// ---------------------------------------------------------------------------

pub type MDBX_option_t = c_int;

pub const MDBX_opt_max_db: MDBX_option_t = 0;
pub const MDBX_opt_max_readers: MDBX_option_t = 1;

// ---------------------------------------------------------------------------
// Error codes
// ---------------------------------------------------------------------------

pub const MDBX_SUCCESS: c_int = 0;
pub const MDBX_RESULT_TRUE: c_int = -1;
pub const MDBX_KEYEXIST: c_int = -30799;
pub const MDBX_NOTFOUND: c_int = -30798;
pub const MDBX_PAGE_NOTFOUND: c_int = -30797;
pub const MDBX_CORRUPTED: c_int = -30796;
pub const MDBX_PANIC: c_int = -30795;
pub const MDBX_MAP_FULL: c_int = -30792;
pub const MDBX_DBS_FULL: c_int = -30791;
pub const MDBX_READERS_FULL: c_int = -30790;
pub const MDBX_TXN_FULL: c_int = -30788;
pub const MDBX_CURSOR_FULL: c_int = -30787;
pub const MDBX_PAGE_FULL: c_int = -30786;
pub const MDBX_BUSY: c_int = -30778;
pub const MDBX_BAD_TXN: c_int = -30782;
pub const MDBX_BAD_DBI: c_int = -30780;

// ---------------------------------------------------------------------------
// C API functions
// ---------------------------------------------------------------------------

unsafe extern "C" {
    // -- Environment --

    pub fn mdbx_env_create(penv: *mut *mut MDBX_env) -> c_int;

    pub fn mdbx_env_set_option(
        env: *mut MDBX_env,
        option: MDBX_option_t,
        value: u64,
    ) -> c_int;

    pub fn mdbx_env_set_geometry(
        env: *mut MDBX_env,
        size_lower: isize,
        size_now: isize,
        size_upper: isize,
        growth_step: isize,
        shrink_threshold: isize,
        pagesize: isize,
    ) -> c_int;

    pub fn mdbx_env_open(
        env: *mut MDBX_env,
        pathname: *const c_char,
        flags: MDBX_env_flags_t,
        mode: u32,
    ) -> c_int;

    pub fn mdbx_env_close_ex(env: *mut MDBX_env, dont_sync: bool) -> c_int;

    pub fn mdbx_env_set_flags(
        env: *mut MDBX_env,
        flags: MDBX_env_flags_t,
        onoff: bool,
    ) -> c_int;

    pub fn mdbx_env_copy(
        env: *mut MDBX_env,
        dest: *const c_char,
        flags: MDBX_copy_flags_t,
    ) -> c_int;

    pub fn mdbx_strerror(errnum: c_int) -> *const c_char;

    // -- Transactions --

    pub fn mdbx_txn_begin_ex(
        env: *mut MDBX_env,
        parent: *mut MDBX_txn,
        flags: MDBX_txn_flags_t,
        txn: *mut *mut MDBX_txn,
        context: *mut c_void,
    ) -> c_int;

    pub fn mdbx_txn_commit_ex(
        txn: *mut MDBX_txn,
        latency: *mut c_void,
    ) -> c_int;

    pub fn mdbx_txn_abort(txn: *mut MDBX_txn) -> c_int;

    // -- Table (DBI) --

    pub fn mdbx_dbi_open(
        txn: *mut MDBX_txn,
        name: *const c_char,
        flags: MDBX_db_flags_t,
        dbi: *mut MDBX_dbi,
    ) -> c_int;

    pub fn mdbx_drop(txn: *mut MDBX_txn, dbi: MDBX_dbi, del: bool) -> c_int;

    // -- Data operations --

    pub fn mdbx_get(
        txn: *const MDBX_txn,
        dbi: MDBX_dbi,
        key: *const MDBX_val,
        data: *mut MDBX_val,
    ) -> c_int;

    pub fn mdbx_put(
        txn: *mut MDBX_txn,
        dbi: MDBX_dbi,
        key: *const MDBX_val,
        data: *mut MDBX_val,
        flags: MDBX_put_flags_t,
    ) -> c_int;

    pub fn mdbx_del(
        txn: *mut MDBX_txn,
        dbi: MDBX_dbi,
        key: *const MDBX_val,
        data: *const MDBX_val,
    ) -> c_int;

    // -- Cursors --

    pub fn mdbx_cursor_open(
        txn: *mut MDBX_txn,
        dbi: MDBX_dbi,
        cursor: *mut *mut MDBX_cursor,
    ) -> c_int;

    pub fn mdbx_cursor_close(cursor: *mut MDBX_cursor);

    pub fn mdbx_cursor_get(
        cursor: *mut MDBX_cursor,
        key: *mut MDBX_val,
        data: *mut MDBX_val,
        op: MDBX_cursor_op,
    ) -> c_int;
}
