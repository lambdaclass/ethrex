use ethrex_mdbx_sys as ffi;
use std::ffi::CStr;
use std::fmt;

/// Errors returned by MDBX operations.
#[derive(Debug)]
pub enum MdbxError {
    /// Key not found (MDBX_NOTFOUND).
    KeyNotFound,
    /// Database mapsize reached (MDBX_MAP_FULL).
    MapFull,
    /// Page has not enough space (MDBX_PAGE_FULL).
    PageFull,
    /// Transaction has too many dirty pages (MDBX_TXN_FULL).
    TxnFull,
    /// Cursor stack too deep (MDBX_CURSOR_FULL).
    CursorFull,
    /// Max named databases reached (MDBX_DBS_FULL).
    DbsFull,
    /// Max readers reached (MDBX_READERS_FULL).
    ReadersFull,
    /// Write transaction already active (MDBX_BUSY).
    Busy,
    /// Catch-all with the raw error code and description.
    Other(i32, String),
}

impl MdbxError {
    /// Convert a raw MDBX return code into a Result.
    /// Returns `Ok(())` for `MDBX_SUCCESS` and `MDBX_RESULT_TRUE`.
    pub fn from_code(rc: i32) -> Result<(), Self> {
        match rc {
            ffi::MDBX_SUCCESS | ffi::MDBX_RESULT_TRUE => Ok(()),
            ffi::MDBX_NOTFOUND => Err(MdbxError::KeyNotFound),
            ffi::MDBX_MAP_FULL => Err(MdbxError::MapFull),
            ffi::MDBX_PAGE_FULL => Err(MdbxError::PageFull),
            ffi::MDBX_TXN_FULL => Err(MdbxError::TxnFull),
            ffi::MDBX_CURSOR_FULL => Err(MdbxError::CursorFull),
            ffi::MDBX_DBS_FULL => Err(MdbxError::DbsFull),
            ffi::MDBX_READERS_FULL => Err(MdbxError::ReadersFull),
            ffi::MDBX_BUSY => Err(MdbxError::Busy),
            code => {
                let msg = unsafe {
                    let ptr = ffi::mdbx_strerror(code);
                    if ptr.is_null() {
                        format!("unknown error {code}")
                    } else {
                        CStr::from_ptr(ptr).to_string_lossy().into_owned()
                    }
                };
                Err(MdbxError::Other(code, msg))
            }
        }
    }
}

impl fmt::Display for MdbxError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MdbxError::KeyNotFound => write!(f, "MDBX_NOTFOUND: key/data not found"),
            MdbxError::MapFull => write!(f, "MDBX_MAP_FULL: environment mapsize reached"),
            MdbxError::PageFull => write!(f, "MDBX_PAGE_FULL: page has not enough space"),
            MdbxError::TxnFull => write!(f, "MDBX_TXN_FULL: transaction has too many dirty pages"),
            MdbxError::CursorFull => write!(f, "MDBX_CURSOR_FULL: cursor stack too deep"),
            MdbxError::DbsFull => write!(f, "MDBX_DBS_FULL: max named databases reached"),
            MdbxError::ReadersFull => {
                write!(f, "MDBX_READERS_FULL: max reader slots reached")
            }
            MdbxError::Busy => write!(f, "MDBX_BUSY: write transaction already active"),
            MdbxError::Other(code, msg) => write!(f, "MDBX error {code}: {msg}"),
        }
    }
}

impl std::error::Error for MdbxError {}
