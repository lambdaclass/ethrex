use ethrex_mdbx_sys as ffi;

use crate::error::MdbxError;
use crate::txn::{RO, RW, Transaction};

use std::collections::HashMap;
use std::ffi::CString;
use std::path::Path;
use std::ptr;
use std::sync::Arc;

/// Configuration for opening an MDBX environment.
pub struct EnvConfig {
    /// Maximum DB file size. Default: 4 TB.
    pub max_size: isize,
    /// Size at which the file grows. Default: 2 GB.
    pub growth_step: isize,
    /// Page size in bytes (must be power of 2). Default: 8192.
    pub page_size: isize,
    /// Maximum number of named tables (DBIs). Default: 32.
    pub max_dbs: u64,
    /// Maximum number of concurrent reader slots. Default: 256.
    pub max_readers: u64,
    /// Environment flags applied at open time.
    pub env_flags: ffi::MDBX_env_flags_t,
    /// File creation mode (unix). Default: 0o664.
    pub mode: u32,
}

impl Default for EnvConfig {
    fn default() -> Self {
        EnvConfig {
            max_size: 4 * 1024 * 1024 * 1024 * 1024, // 4 TB
            growth_step: 2 * 1024 * 1024 * 1024,      // 2 GB
            page_size: 8192,                           // 8 KB
            max_dbs: 32,
            max_readers: 256,
            env_flags: ffi::MDBX_NORDAHEAD | ffi::MDBX_WRITEMAP,
            mode: 0o664,
        }
    }
}

/// An opened MDBX environment.
///
/// The environment is the top-level object that holds the memory-mapped file
/// and all database handles (DBIs). It is safe to share across threads.
pub struct Environment {
    env: *mut ffi::MDBX_env,
    /// Cached DBI handles, opened once at init and reused for the lifetime.
    dbis: Arc<HashMap<&'static str, ffi::MDBX_dbi>>,
}

// SAFETY: MDBX environment handles are thread-safe. We compile with
// MDBX_TXN_CHECKOWNER=0 to allow cross-thread transaction use.
unsafe impl Send for Environment {}
unsafe impl Sync for Environment {}

impl Environment {
    /// Open (or create) an MDBX environment at the given directory path.
    ///
    /// All tables in `table_names` are created if they don't exist.
    pub fn open(
        path: &Path,
        config: EnvConfig,
        table_names: &[&'static str],
    ) -> Result<Self, MdbxError> {
        let mut env: *mut ffi::MDBX_env = ptr::null_mut();

        // 1. Create environment handle
        unsafe {
            MdbxError::from_code(ffi::mdbx_env_create(&mut env))?;
        }

        // Guard: if anything below fails, close the env handle so we don't leak.
        // On success we defuse the guard with `mem::forget`.
        struct EnvGuard(*mut ffi::MDBX_env);
        impl Drop for EnvGuard {
            fn drop(&mut self) {
                unsafe {
                    ffi::mdbx_env_close_ex(self.0, false);
                }
            }
        }
        let guard = EnvGuard(env);

        // 2. Set max DBs
        unsafe {
            MdbxError::from_code(ffi::mdbx_env_set_option(
                env,
                ffi::MDBX_opt_max_db,
                config.max_dbs,
            ))?;
        }

        // 3. Set max readers
        unsafe {
            MdbxError::from_code(ffi::mdbx_env_set_option(
                env,
                ffi::MDBX_opt_max_readers,
                config.max_readers,
            ))?;
        }

        // 4. Set geometry (page size, max size, growth step)
        unsafe {
            MdbxError::from_code(ffi::mdbx_env_set_geometry(
                env,
                0,                     // size_lower: minimum
                -1,                    // size_now: current/default
                config.max_size,       // size_upper
                config.growth_step,    // growth_step
                -1,                    // shrink_threshold: auto
                config.page_size,      // pagesize
            ))?;
        }

        // 5. Open the environment
        let c_path = path_to_cstring(path)?;
        unsafe {
            MdbxError::from_code(ffi::mdbx_env_open(
                env,
                c_path.as_ptr(),
                config.env_flags | ffi::MDBX_SYNC_DURABLE,
                config.mode,
            ))?;
        }

        // 6. Open all named tables in a write transaction
        let mut dbis = HashMap::new();
        unsafe {
            let mut txn: *mut ffi::MDBX_txn = ptr::null_mut();
            MdbxError::from_code(ffi::mdbx_txn_begin_ex(
                env,
                ptr::null_mut(),
                ffi::MDBX_TXN_READWRITE,
                &mut txn,
                ptr::null_mut(),
            ))?;

            for &table_name in table_names {
                let c_name = CString::new(table_name).map_err(|e| {
                    ffi::mdbx_txn_abort(txn);
                    MdbxError::Other(-1, format!("invalid table name: {e}"))
                })?;
                let mut dbi: ffi::MDBX_dbi = 0;
                let rc =
                    ffi::mdbx_dbi_open(txn, c_name.as_ptr(), ffi::MDBX_CREATE, &mut dbi);
                if rc != ffi::MDBX_SUCCESS {
                    ffi::mdbx_txn_abort(txn);
                    return Err(MdbxError::from_code(rc).unwrap_err());
                }
                dbis.insert(table_name, dbi);
            }

            MdbxError::from_code(ffi::mdbx_txn_commit_ex(txn, ptr::null_mut()))?;
        }

        // Success — defuse the guard so Drop doesn't close our env.
        std::mem::forget(guard);

        Ok(Environment {
            env,
            dbis: Arc::new(dbis),
        })
    }

    /// Begin a read-only transaction.
    pub fn begin_ro_txn(&self) -> Result<Transaction<RO>, MdbxError> {
        let mut txn: *mut ffi::MDBX_txn = ptr::null_mut();
        unsafe {
            MdbxError::from_code(ffi::mdbx_txn_begin_ex(
                self.env,
                ptr::null_mut(),
                ffi::MDBX_TXN_RDONLY,
                &mut txn,
                ptr::null_mut(),
            ))?;
        }
        Ok(Transaction::new(txn, self.dbis.clone()))
    }

    /// Begin a read-write transaction.
    pub fn begin_rw_txn(&self) -> Result<Transaction<RW>, MdbxError> {
        let mut txn: *mut ffi::MDBX_txn = ptr::null_mut();
        unsafe {
            MdbxError::from_code(ffi::mdbx_txn_begin_ex(
                self.env,
                ptr::null_mut(),
                ffi::MDBX_TXN_READWRITE,
                &mut txn,
                ptr::null_mut(),
            ))?;
        }
        Ok(Transaction::new(txn, self.dbis.clone()))
    }

    /// Copy the environment to the given path (for checkpoints).
    pub fn copy_to(&self, dest: &Path) -> Result<(), MdbxError> {
        let c_dest = path_to_cstring(dest)?;
        unsafe {
            MdbxError::from_code(ffi::mdbx_env_copy(
                self.env,
                c_dest.as_ptr(),
                ffi::MDBX_CP_COMPACT,
            ))?;
        }
        Ok(())
    }
}

impl Drop for Environment {
    fn drop(&mut self) {
        unsafe {
            ffi::mdbx_env_close_ex(self.env, false);
        }
    }
}

/// Convert a Path to a CString, handling non-UTF8 paths on unix.
fn path_to_cstring(path: &Path) -> Result<CString, MdbxError> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        CString::new(path.as_os_str().as_bytes())
            .map_err(|e| MdbxError::Other(-1, format!("path contains null byte: {e}")))
    }
    #[cfg(not(unix))]
    {
        let s = path
            .to_str()
            .ok_or_else(|| MdbxError::Other(-1, "path is not valid UTF-8".into()))?;
        CString::new(s)
            .map_err(|e| MdbxError::Other(-1, format!("path contains null byte: {e}")))
    }
}
