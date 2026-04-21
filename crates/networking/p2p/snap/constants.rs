//! Snap Sync Protocol Constants
//!
//! This module centralizes all constants used in the snap sync implementation.
//! Constants are organized by their functional area.

use ethrex_common::H256;
use std::time::Duration;

// =============================================================================
// RESPONSE LIMITS
// =============================================================================

/// Maximum response size in bytes for snap protocol requests (2 MiB).
///
/// Matches bsc-geth/geth's `softResponseLimit` so peers can fill responses
/// up to their own cap. Lower values (we were at 512 KiB) leave bandwidth
/// on the table on every round-trip.
pub const MAX_RESPONSE_BYTES: u64 = 2 * 1024 * 1024;

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

/// Number of chunks to split the account range into for parallel downloading.
pub const ACCOUNT_RANGE_CHUNK_COUNT: usize = 800;

/// Number of storage accounts to process per batch during state healing.
pub const STORAGE_BATCH_SIZE: usize = 300;

/// Number of trie nodes to request per batch during state/storage healing.
/// Matches bsc-geth's maxTrieRequestCount = maxRequestSize / 512B (avg node size),
/// sized to fill MAX_RESPONSE_BYTES (2 MiB) per round-trip.
pub const NODE_BATCH_SIZE: usize = 4096;

/// Number of bytecodes to download per batch.
pub const BYTECODE_CHUNK_SIZE: usize = 50_000;

/// Buffer size for code hash collection before writing.
pub const CODE_HASH_WRITE_BUFFER_SIZE: usize = 100_000;

// =============================================================================
// REQUEST CONFIGURATION
// =============================================================================

/// Timeout for peer responses in snap sync operations.
pub const PEER_REPLY_TIMEOUT: Duration = Duration::from_secs(15);

/// Timeout for block header fetches. Headers are small (a few KB) and cheap
/// to serve, so a slow reply signals an unresponsive peer. Keeping this short
/// avoids pivot refresh stalling on dead peers for 15s each.
pub const HEADER_REQUEST_TIMEOUT: Duration = Duration::from_secs(3);

/// Number of retry attempts when selecting a peer for a request.
pub const PEER_SELECT_RETRY_ATTEMPTS: u32 = 3;

/// Number of retry attempts for individual requests.
pub const REQUEST_RETRY_ATTEMPTS: u32 = 5;

/// Maximum number of concurrent in-flight requests during storage healing.
/// bsc-geth allows one concurrent heal request per connected peer (no global
/// cap), so sized to allow full utilization with 100+ snap-capable peers.
pub const MAX_IN_FLIGHT_REQUESTS: u32 = 256;

/// Soft limit on the number of entries in the storage healing queue
/// (`StorageHealingQueue` — the pending-parents `HashMap` drained by
/// `commit_node` cascades). Sized to bound resident memory of that map near
/// ~1 GB.
///
/// Per-entry cost, branch-dominated worst case:
/// - `NodeResponse.node`: a branch `Node` is a `Box<BranchNode>` with 16
///   `NodeRef` choices (~56 B each in the `Hash` variant) plus `ValueRLP`
///   header, ≈ 950 B on the heap.
/// - `NodeResponse.node_request`: three `Nibbles` (each a `Vec<u8>`, ~24 B
///   header + up to 64 B data) + one `H256`, ≈ 250 B inline+heap.
/// - `HashMap<(Nibbles, Nibbles), _>` key and bucket overhead, ≈ 100 B.
///
/// Total ≈ 1.3 KB per entry → 800_000 entries ≈ 1.0 GB. Leaf-dominated
/// entries are smaller, so this is an upper-bound estimate. The limit gates
/// `healing_queue` only; `download_queue` is a separate (smaller) allocation.
///
/// When exceeded, the dispatcher stops issuing new download requests and
/// waits for in-flight responses to drain the queue. The download queue is a
/// max-heap by depth, so in-flight work is the deepest available — which
/// frees pending parents fastest via `commit_node` cascades.
pub const HEALING_QUEUE_SOFT_LIMIT: usize = 800_000;

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
/// Each attempt queries multiple peers with a 15s timeout, so this
/// should be low to avoid stalling for tens of minutes when the sync
/// head is unknown to peers. The caller can re-enter with a newer
/// sync head from the CL.
pub const MAX_HEADER_FETCH_ATTEMPTS: u64 = 5;

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
pub const MISSING_SLOTS_PERCENTAGE: f64 = 0.98;

// =============================================================================
// PROGRESS REPORTING
// =============================================================================

/// Interval between progress reports during healing operations.
pub const SHOW_PROGRESS_INTERVAL_DURATION: Duration = Duration::from_secs(2);
