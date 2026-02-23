//! Error types for node-replay.

use thiserror::Error;

/// All typed error codes for the node-replay tool.
/// Agents use error_code strings for retry/branching logic.
#[derive(Debug, Error)]
pub enum ReplayError {
    // Input errors (exit code 10)
    #[error("invalid argument: {0}")]
    InvalidArgument(String),

    #[error("path conflict: {reason}")]
    PathConflict { reason: String },

    // State/lock conflicts (exit code 20)
    #[error("invalid state transition from {from} to {to}")]
    InvalidTransition { from: String, to: String },

    #[error("run not found: {0}")]
    RunNotFound(String),

    #[error("run already running: {0}")]
    RunAlreadyRunning(String),

    #[error("run canceled: {0}")]
    RunCanceled(String),

    #[error("lock already held by PID {pid} on {hostname}")]
    LockAlreadyHeld { pid: u32, hostname: String },

    // Chain consistency errors (exit code 30)
    #[error("reorg detected at block {block_number}: expected {expected}, got {actual}")]
    ReorgDetected {
        block_number: u64,
        expected: String,
        actual: String,
    },

    #[error("hash mismatch at block {block_number}: expected {expected}, got {actual}")]
    HashMismatch {
        block_number: u64,
        expected: String,
        actual: String,
    },

    #[error("genesis mismatch: checkpoint genesis {checkpoint} != store genesis {store}")]
    GenesisMismatch { checkpoint: String, store: String },

    #[error("chain ID mismatch: checkpoint {checkpoint} != store {store}")]
    ChainIdMismatch { checkpoint: u64, store: u64 },

    // Execution errors (exit code 40)
    #[error("checkpoint creation failed: {0}")]
    CheckpointFailed(String),

    #[error("block execution failed at block {block_number}: {reason}")]
    BlockFailed { block_number: u64, reason: String },

    // Verification errors (exit code 50)
    #[error(
        "state root mismatch at block {block_number}: expected {expected}, computed {computed}"
    )]
    StateRootMismatch {
        block_number: u64,
        expected: String,
        computed: String,
    },

    #[error(
        "receipts root mismatch at block {block_number}: expected {expected}, computed {computed}"
    )]
    ReceiptsRootMismatch {
        block_number: u64,
        expected: String,
        computed: String,
    },

    // Internal errors (exit code 70)
    #[error("internal error: {0}")]
    Internal(String),

    // Wrapped storage errors
    #[error("storage error: {0}")]
    Storage(String),

    // Wrapped IO errors
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    // Wrapped JSON errors
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
}

impl ReplayError {
    /// Returns the typed error code string for agent consumption
    pub fn error_code(&self) -> &'static str {
        match self {
            Self::InvalidArgument(_) => "input/invalid_argument",
            Self::PathConflict { .. } => "input/path_conflict",
            Self::InvalidTransition { .. } => "state/invalid_transition",
            Self::RunNotFound(_) => "state/run_not_found",
            Self::RunAlreadyRunning(_) => "conflict/run_already_running",
            Self::RunCanceled(_) => "state/run_canceled",
            Self::LockAlreadyHeld { .. } => "lock/already_held",
            Self::ReorgDetected { .. } => "chain/reorg_detected",
            Self::HashMismatch { .. } => "chain/hash_mismatch",
            Self::GenesisMismatch { .. } => "chain/genesis_mismatch",
            Self::ChainIdMismatch { .. } => "chain/chain_id_mismatch",
            Self::CheckpointFailed(_) => "storage/checkpoint_failed",
            Self::BlockFailed { .. } => "execution/block_failed",
            Self::StateRootMismatch { .. } => "verify/state_root_mismatch",
            Self::ReceiptsRootMismatch { .. } => "verify/receipts_root_mismatch",
            Self::Internal(_) => "internal/unexpected",
            Self::Storage(_) => "storage/error",
            Self::Io(_) => "internal/io_error",
            Self::Json(_) => "internal/json_error",
        }
    }

    /// Returns the process exit code
    pub fn exit_code(&self) -> i32 {
        match self {
            Self::InvalidArgument(_) | Self::PathConflict { .. } => 10,
            Self::InvalidTransition { .. }
            | Self::RunNotFound(_)
            | Self::RunAlreadyRunning(_)
            | Self::RunCanceled(_)
            | Self::LockAlreadyHeld { .. } => 20,
            Self::ReorgDetected { .. }
            | Self::HashMismatch { .. }
            | Self::GenesisMismatch { .. }
            | Self::ChainIdMismatch { .. } => 30,
            Self::CheckpointFailed(_) | Self::BlockFailed { .. } => 40,
            Self::StateRootMismatch { .. } | Self::ReceiptsRootMismatch { .. } => 50,
            Self::Internal(_) | Self::Storage(_) | Self::Io(_) | Self::Json(_) => 70,
        }
    }
}

/// Convert from ethrex StoreError
impl From<ethrex_storage::error::StoreError> for ReplayError {
    fn from(e: ethrex_storage::error::StoreError) -> Self {
        Self::Storage(e.to_string())
    }
}
