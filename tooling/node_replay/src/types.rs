//! Core types for node-replay.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

/// Replay mode for a run
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, clap::ValueEnum)]
#[serde(rename_all = "snake_case")]
pub enum ReplayMode {
    Isolated,
    StopLiveNode,
}

/// Run state in the state machine
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RunState {
    Planned,
    Running,
    Paused,
    Completed,
    Failed,
    Canceled,
}

impl RunState {
    /// Returns true if this state is a terminal state (no further transitions possible).
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            RunState::Completed | RunState::Failed | RunState::Canceled
        )
    }
}

/// Finality level for block selection
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, clap::ValueEnum)]
#[serde(rename_all = "snake_case")]
pub enum Finality {
    Safe,
    Finalized,
    Head,
}

/// checkpoint.json - Immutable checkpoint metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckpointMeta {
    pub checkpoint_id: String,
    pub datadir: PathBuf,
    pub checkpoint_db_path: PathBuf,
    pub anchor_number: u64,
    /// Hex-encoded H256 block hash
    pub anchor_hash: String,
    pub created_at: DateTime<Utc>,
    pub ethrex_commit: Option<String>,
    pub chain_id: u64,
    pub network: String,
    pub label: String,
}

/// run_manifest.json - Pinned replay plan
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunManifest {
    pub run_id: String,
    pub checkpoint_id: String,
    pub mode: ReplayMode,
    pub start_number: u64,
    pub end_number: u64,
    /// Ordered array of hex-encoded block hashes for the range
    pub canonical_hashes: Vec<String>,
    /// Hex-encoded expected head hash after full replay
    pub expected_head_hash: String,
    pub created_at: DateTime<Utc>,
    pub idempotency_key: Option<String>,
}

/// status.json - Mutable run status
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunStatus {
    pub run_id: String,
    pub state: RunState,
    pub last_completed_block: Option<u64>,
    /// Hex-encoded hash of the last completed block
    pub last_completed_hash: Option<String>,
    pub started_at: Option<DateTime<Utc>>,
    pub updated_at: DateTime<Utc>,
    pub error_code: Option<String>,
    pub error_message: Option<String>,
}

impl RunStatus {
    /// Creates an initial planned status for a new run.
    pub fn new_planned(run_id: String) -> Self {
        RunStatus {
            run_id,
            state: RunState::Planned,
            last_completed_block: None,
            last_completed_hash: None,
            started_at: None,
            updated_at: Utc::now(),
            error_code: None,
            error_message: None,
        }
    }
}

/// summary.json - Final run summary
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunSummary {
    pub run_id: String,
    pub state: RunState,
    pub total_blocks: u64,
    pub executed_blocks: u64,
    pub duration_ms: u64,
    pub avg_block_ms: f64,
    pub mismatch_count: u64,
    pub final_head_number: Option<u64>,
    /// Hex-encoded final head hash
    pub final_head_hash: Option<String>,
}

impl RunSummary {
    /// Creates a run summary from a completed run's status and manifest.
    pub fn from_status(status: &RunStatus, manifest: &RunManifest, duration_ms: u64) -> Self {
        let total_blocks = manifest.end_number.saturating_sub(manifest.start_number) + 1;
        let executed_blocks = status
            .last_completed_block
            .map(|b| b.saturating_sub(manifest.start_number) + 1)
            .unwrap_or(0);
        let avg_block_ms = if executed_blocks > 0 {
            duration_ms as f64 / executed_blocks as f64
        } else {
            0.0
        };

        RunSummary {
            run_id: status.run_id.clone(),
            state: status.state.clone(),
            total_blocks,
            executed_blocks,
            duration_ms,
            avg_block_ms,
            mismatch_count: 0,
            final_head_number: status.last_completed_block,
            final_head_hash: status.last_completed_hash.clone(),
        }
    }
}

/// Event names for events.ndjson
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EventName {
    RunPlanned,
    RunStarted,
    BlockStarted,
    BlockExecuted,
    BlockVerified,
    RunPaused,
    RunResumed,
    RunFailed,
    RunCompleted,
}

/// Single event in events.ndjson (line-delimited JSON)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplayEvent {
    pub ts: DateTime<Utc>,
    pub run_id: String,
    pub event: EventName,
    pub block_number: Option<u64>,
    /// Hex-encoded block hash
    pub block_hash: Option<String>,
    #[serde(default)]
    pub payload: HashMap<String, serde_json::Value>,
}

impl ReplayEvent {
    /// Creates a new event with the current UTC timestamp.
    pub fn new(
        run_id: String,
        event: EventName,
        block_number: Option<u64>,
        block_hash: Option<String>,
    ) -> Self {
        ReplayEvent {
            ts: Utc::now(),
            run_id,
            event,
            block_number,
            block_hash,
            payload: HashMap::new(),
        }
    }
}

/// Lock file content
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LockInfo {
    pub holder_pid: u32,
    pub holder_hostname: String,
    pub acquired_at: DateTime<Utc>,
    pub run_id: String,
}

/// Generic JSON command response wrapper
#[derive(Debug, Serialize, Deserialize)]
pub struct CommandResponse<T: Serialize> {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<T>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<ErrorResponse>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
}

impl<T: Serialize> CommandResponse<T> {
    /// Creates a successful response with optional data and request ID.
    pub fn success(data: Option<T>, request_id: Option<String>) -> Self {
        CommandResponse {
            success: true,
            data,
            error: None,
            request_id,
        }
    }

    /// Creates an error response with error code, message, and optional request ID.
    pub fn error(code: String, message: String, request_id: Option<String>) -> Self {
        CommandResponse {
            success: false,
            data: None,
            error: Some(ErrorResponse { code, message }),
            request_id,
        }
    }
}

/// Error detail in command responses
#[derive(Debug, Serialize, Deserialize)]
pub struct ErrorResponse {
    pub code: String,
    pub message: String,
}
