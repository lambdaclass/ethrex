//! Constants used throughout the snap sync module.
//!
//! This module centralizes all magic numbers and configuration values
//! for easier maintenance and tuning.

use std::time::Duration;

// ============================================================================
// Full Sync Constants
// ============================================================================

/// The minimum amount of blocks from the head that we want to full sync during a snap sync.
/// If fewer blocks need to be synced, we switch to full sync mode.
pub const MIN_FULL_BLOCKS: u64 = 10_000;

/// Amount of blocks to execute in a single batch during FullSync.
pub const EXECUTE_BATCH_SIZE_DEFAULT: usize = 1024;

/// Average amount of seconds between blocks on Ethereum mainnet.
pub const SECONDS_PER_BLOCK: u64 = 12;

/// Maximum attempts before giving up on header downloads during syncing.
pub const MAX_HEADER_FETCH_ATTEMPTS: u64 = 100;

// ============================================================================
// Account Download Constants
// ============================================================================

/// Number of chunks to split the account hash space into for parallel downloading.
/// Geth uses 16, but we use 800 for finer granularity.
pub const ACCOUNT_RANGE_CHUNKS: u64 = 800;

/// Maximum header chunk size when requesting headers by number.
pub const MAX_HEADER_CHUNK: u64 = 500_000;

/// Number of parallel tasks for header downloads.
/// When downloading many headers, we split the work into this many concurrent tasks.
pub const HEADER_DOWNLOAD_CONCURRENCY: u64 = 800;

// ============================================================================
// Storage Download Constants
// ============================================================================

/// Maximum number of accounts to request storage for in a single batch.
pub const STORAGE_ACCOUNTS_BATCH_SIZE: usize = 300;

/// Number of storage slots to estimate per chunk when splitting big account storage.
/// Used to calculate chunk sizes for accounts with large storage tries.
pub const BIG_ACCOUNT_CHUNK_SLOTS: usize = 10_000;

// ============================================================================
// Bytecode Download Constants
// ============================================================================

/// Number of bytecodes to download per batch.
pub const BYTECODE_CHUNK_SIZE: usize = 50_000;

/// Maximum number of bytecodes to request in a single peer request.
pub const MAX_BYTECODES_PER_REQUEST: usize = 100;

/// Number of chunks to split bytecode downloads into for parallel downloading.
pub const BYTECODE_DOWNLOAD_CHUNKS: usize = 800;

// ============================================================================
// Healing Constants
// ============================================================================

/// Maximum size of a batch to start a node fetch request during state healing.
pub const STATE_NODE_BATCH_SIZE: usize = 500;

/// Maximum size of a batch to start a storage fetch request during storage healing.
pub const STORAGE_NODE_BATCH_SIZE: usize = 300;

/// Maximum number of concurrent in-flight healing requests.
pub const MAX_HEALING_IN_FLIGHT: u32 = 77;

/// Interval at which healing progress is shown via info tracing.
pub const HEALING_PROGRESS_INTERVAL: Duration = Duration::from_secs(2);

// ============================================================================
// Code Hash Collection Constants
// ============================================================================

/// Size of the buffer to store code hashes before flushing to a file.
pub const CODE_HASH_WRITE_BUFFER_SIZE: usize = 100_000;

// ============================================================================
// File I/O Constants
// ============================================================================

/// How much data we store in memory during account/storage range downloads
/// before dumping it into a file. This tunes memory usage during
/// the first steps of snap sync.
pub const RANGE_FILE_CHUNK_SIZE: usize = 64 * 1024 * 1024; // 64MB

// ============================================================================
// Peer/Network Constants
// ============================================================================

/// Timeout for waiting for a peer reply to a request.
pub const PEER_REPLY_TIMEOUT: Duration = Duration::from_secs(15);

/// Number of retry attempts when selecting a peer.
pub const PEER_SELECT_RETRY_ATTEMPTS: u32 = 3;

/// Number of retry attempts for a request before giving up.
pub const REQUEST_RETRY_ATTEMPTS: u32 = 5;

/// Maximum bytes expected in a snap protocol response.
/// This is sent to peers to indicate how much data we're willing to receive.
pub const MAX_RESPONSE_BYTES: u64 = 512 * 1024;

/// The snap sync limit - number of blocks behind the head that snap sync can target.
/// Beyond this, the state may be pruned by peers.
pub const SNAP_LIMIT: usize = 128;

/// Maximum number of block bodies to request per request.
/// This magic number is not part of the protocol and is taken from geth.
/// See: https://github.com/ethereum/go-ethereum/blob/2585776aabbd4ae9b00050403b42afb0cee968ec/eth/downloader/downloader.go#L42-L43
///
/// Note: Larger values may cause peer disconnections.
pub const MAX_BLOCK_BODIES_TO_REQUEST: usize = 128;

/// Threshold at which we flush nodes to database during healing.
pub const HEALING_FLUSH_THRESHOLD: usize = 100_000;

// ============================================================================
// Pivot Block Constants
// ============================================================================

/// We assume this percentage of slots are present (vs missing/empty slots)
/// when calculating estimated block numbers based on timestamps.
/// This accounts for ~9% missing slots in testnets.
pub const SLOT_FILL_PERCENTAGE: f64 = 0.8;
