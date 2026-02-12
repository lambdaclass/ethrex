//! Snap Sync Protocol Constants
//!
//! This module centralizes all constants used in the snap sync implementation.
//! Constants are organized by their functional area.

use ethrex_common::H256;
use std::time::Duration;

// =============================================================================
// RESPONSE LIMITS
// =============================================================================

/// Maximum response size in bytes for snap protocol requests (512 KB).
///
/// This limits the amount of data a peer can return in a single response,
/// preventing memory exhaustion and ensuring reasonable response times.
pub const MAX_RESPONSE_BYTES: u64 = 512 * 1024;

/// Maximum number of accounts/items to request in a single snap request.
///
/// This magic number is not part of the protocol specification and is taken
/// from geth. See:
/// <https://github.com/ethereum/go-ethereum/blob/2585776aabbd4ae9b00050403b42afb0cee968ec/eth/downloader/downloader.go#L42-L43>
pub const SNAP_LIMIT: usize = 128;

// =============================================================================
// HASH BOUNDARIES
// =============================================================================

/// Maximum hash value (all bits set to 1).
///
/// Used as the upper bound when requesting the full range of accounts/storage.
pub const HASH_MAX: H256 = H256([0xFF; 32]);

// =============================================================================
// BATCH SIZES
// =============================================================================

/// Size of the in-memory buffer before flushing to disk during snap sync (64 MB).
///
/// During account range and storage range downloads, data is accumulated in memory
/// before being written to temporary files. This constant controls memory usage
/// during the initial snap sync phases.
pub const RANGE_FILE_CHUNK_SIZE: usize = 1024 * 1024 * 64;

/// Number of chunks to split each account range partition into for parallel downloading.
pub const ACCOUNT_RANGE_CHUNK_COUNT: usize = 800;

/// Maximum number of concurrent partitions for account range downloads.
/// Each partition covers a disjoint slice of the address space and downloads independently.
pub const MAX_ACCOUNT_PARTITIONS: usize = 8;

/// Number of storage accounts to process per batch during state healing.
pub const STORAGE_BATCH_SIZE: usize = 300;

/// Number of trie nodes to request per batch during state/storage healing.
pub const NODE_BATCH_SIZE: usize = 500;

/// Number of bytecodes to download per batch.
pub const BYTECODE_CHUNK_SIZE: usize = 50_000;

/// Buffer size for code hash collection before writing.
pub const CODE_HASH_WRITE_BUFFER_SIZE: usize = 100_000;

// =============================================================================
// REQUEST CONFIGURATION
// =============================================================================

/// Timeout for peer responses in snap sync operations.
pub const PEER_REPLY_TIMEOUT: Duration = Duration::from_secs(15);

/// Number of retry attempts when selecting a peer for a request.
pub const PEER_SELECT_RETRY_ATTEMPTS: u32 = 3;

/// Number of retry attempts for individual requests.
pub const REQUEST_RETRY_ATTEMPTS: u32 = 5;

/// Maximum number of concurrent in-flight requests during storage healing.
pub const MAX_IN_FLIGHT_REQUESTS: u32 = 77;

// =============================================================================
// BLOCK SYNC CONFIGURATION
// =============================================================================

/// Maximum number of block headers to fetch in a single request.
pub const MAX_HEADER_CHUNK: u64 = 500_000;

/// Maximum number of block bodies to request per request.
///
/// This value is taken from geth. Higher values may cause peer disconnections.
/// See:
/// <https://github.com/ethereum/go-ethereum/blob/2585776aabbd4ae9b00050403b42afb0cee968ec/eth/downloader/downloader.go#L42-L43>
pub const MAX_BLOCK_BODIES_TO_REQUEST: usize = 128;

/// Maximum attempts before giving up on header downloads during syncing.
pub const MAX_HEADER_FETCH_ATTEMPTS: u64 = 100;

// =============================================================================
// SNAP SYNC THRESHOLDS
// =============================================================================

/// Minimum number of blocks from the head to full sync during a snap sync.
///
/// After snap syncing state, we full sync at least this many recent blocks
/// to ensure we have complete execution history for recent blocks.
pub const MIN_FULL_BLOCKS: u64 = 10_000;

/// Number of blocks to execute in a single batch during full sync.
pub const EXECUTE_BATCH_SIZE_DEFAULT: usize = 1024;

/// Average time between blocks (used for timestamp-based calculations).
pub const SECONDS_PER_BLOCK: u64 = 12;

/// Assumed percentage of slots that are missing blocks.
///
/// This is used to adjust timestamp-based pivot updates and to find "safe"
/// blocks in the chain that are unlikely to be re-orged.
pub const MISSING_SLOTS_PERCENTAGE: f64 = 0.8;

// =============================================================================
// ADAPTIVE REQUEST SIZING BOUNDS
// =============================================================================

/// Minimum/maximum accounts per adaptive account range request.
pub const ACCOUNT_RANGE_MIN_ITEMS: usize = 100;
pub const ACCOUNT_RANGE_MAX_ITEMS: usize = 4096;

/// Minimum/maximum storage ranges per adaptive request.
pub const STORAGE_RANGE_MIN_ITEMS: usize = 100;
pub const STORAGE_RANGE_MAX_ITEMS: usize = 2400;

/// Minimum/maximum bytecodes per adaptive request.
pub const BYTECODES_MIN_ITEMS: usize = 50;
pub const BYTECODES_MAX_ITEMS: usize = 500;

/// Minimum/maximum state trie nodes per adaptive request.
pub const STATE_TRIE_NODES_MIN_ITEMS: usize = 50;
pub const STATE_TRIE_NODES_MAX_ITEMS: usize = 1000;

/// Minimum/maximum storage trie nodes per adaptive request.
pub const STORAGE_TRIE_NODES_MIN_ITEMS: usize = 50;
pub const STORAGE_TRIE_NODES_MAX_ITEMS: usize = 1000;

// =============================================================================
// PROGRESS REPORTING
// =============================================================================

/// Interval between progress reports during healing operations.
pub const SHOW_PROGRESS_INTERVAL_DURATION: Duration = Duration::from_secs(2);
