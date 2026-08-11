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
pub const SNAP_LIMIT_DEFAULT: usize = 128;

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
pub const NODE_BATCH_SIZE: usize = 500;

/// Number of bytecodes to download per batch.
pub const BYTECODE_CHUNK_SIZE: usize = 50_000;

/// Buffer size for code hash collection before writing.
pub const CODE_HASH_WRITE_BUFFER_SIZE: usize = 100_000;

// =============================================================================
// REQUEST CONFIGURATION
// =============================================================================

/// Timeout for peer responses in snap sync operations.
pub const PEER_REPLY_TIMEOUT: Duration = Duration::from_secs(5);

/// Number of retry attempts when selecting a peer for a request.
pub const PEER_SELECT_RETRY_ATTEMPTS: u32 = 3;

/// Number of retry attempts for individual requests.
pub const REQUEST_RETRY_ATTEMPTS: u32 = 5;

/// Maximum number of concurrent in-flight requests during storage healing.
pub const MAX_IN_FLIGHT_REQUESTS: u32 = 77;

/// Soft limit on the number of entries in a healing pending-parents queue.
///
/// Shared by storage healing (`StorageHealingQueue`) and state healing
/// (`StateHealingQueue`). Both are `HashMap`s of nodes awaiting their missing
/// children; both are drained via `commit_node` cascades. The limit is sized
/// for the larger of the two (storage) and is therefore conservative for
/// state.
///
/// Storage per-entry cost, branch-dominated worst case:
/// - `NodeResponse.node`: a branch `Node` is a `Box<BranchNode>` with 16
///   `NodeRef` choices (~56 B each in the `Hash` variant) plus `ValueRLP`
///   header, ≈ 950 B on the heap.
/// - `NodeResponse.node_request`: three `Nibbles` (each a `Vec<u8>`, ~24 B
///   header + up to 64 B data) + one `H256`, ≈ 250 B inline+heap.
/// - `HashMap<(Nibbles, Nibbles), _>` key and bucket overhead, ≈ 100 B.
///
/// Total ≈ 1.3 KB per entry → 800_000 entries ≈ 1.0 GB. State entries omit
/// the extra `acc_path` `Nibbles` and use a single-`Nibbles` key, so they're
/// smaller — the same count uses less memory on that side. Leaf-dominated
/// entries are smaller still, so this is an upper-bound estimate. The limit
/// gates the pending-parents map only; the download queue is a separate
/// (smaller) allocation.
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

/// Maximum *consecutive* failures before giving up on header downloads.
/// The counter resets on each successful response, so this only triggers
/// when no peer can serve headers at all.
pub const MAX_HEADER_FETCH_ATTEMPTS: u64 = 10;

/// Maximum attempts before giving up on a block-body download during full sync.
/// Mirrors the header-fetch policy; split out so the two can diverge if needed.
pub const MAX_BODY_FETCH_ATTEMPTS: u64 = MAX_HEADER_FETCH_ATTEMPTS;

// =============================================================================
// SNAP SYNC THRESHOLDS
// =============================================================================

/// Minimum number of blocks from the head to full sync during a snap sync.
///
/// After snap syncing state, we full sync at least this many recent blocks
/// to ensure we have complete execution history for recent blocks.
pub const MIN_FULL_BLOCKS_DEFAULT: u64 = 10_000;

/// Number of blocks to execute in a single batch during full sync.
pub const EXECUTE_BATCH_SIZE_DEFAULT: usize = 1024;

/// Average time between blocks (used for timestamp-based calculations).
pub const SECONDS_PER_BLOCK_DEFAULT: u64 = 12;

/// Assumed percentage of slots that are missing blocks.
///
/// This is used to adjust timestamp-based pivot updates and to find "safe"
/// blocks in the chain that are unlikely to be re-orged.
pub const MISSING_SLOTS_PERCENTAGE: f64 = 0.8;

// =============================================================================
// PROGRESS REPORTING
// =============================================================================

/// Interval between progress reports during healing operations.
pub const SHOW_PROGRESS_INTERVAL_DURATION: Duration = Duration::from_secs(2);

// =============================================================================
// snap/2 BAL CONFIGURATION (EIP-8189)
// =============================================================================

/// Soft response size cap for `BlockAccessLists` responses, per EIP-8189
/// ("BlockAccessLists"): 2 MiB is the recommended limit when a request names none.
pub const BAL_RESPONSE_SOFT_CAP_BYTES: u64 = 2 * 1024 * 1024;

/// Average compressed BAL size at a 60M block gas limit, per EIP-7928
/// ("BAL Size Considerations").
const BAL_AVERAGE_SIZE_BYTES: u64 = 72 * 1024;

/// Number of block hashes to request in a single `GetBlockAccessLists` batch,
/// sized so an average-BAL response fits within the soft cap instead of being
/// truncated on every round trip.
pub const BAL_REQUEST_BATCH_SIZE: usize =
    (BAL_RESPONSE_SOFT_CAP_BYTES / BAL_AVERAGE_SIZE_BYTES) as usize;

/// Maximum retry attempts per block before falling back to snap/1 healing.
pub const BAL_MAX_RETRIES_PER_BLOCK: u32 = 3;

/// Maximum number of hashes served in a single `Snap2GetBlockAccessLists` response.
/// EIP-8189 leaves the per-request hash count to implementations; bounding it
/// defends against a flood of hashes sent to force expensive per-hash storage
/// lookups, as the snap/1 handler already does for trie-node lookups.
pub const BAL_MAX_REQUEST_HASHES: usize = 1024;

// =============================================================================
// TEST-ONLY OVERRIDES
// =============================================================================
//
// A devnet cannot exercise snap sync at production values. `MIN_FULL_BLOCKS`
// alone means the chain has to be 10,000 blocks deep before snap sync engages
// at all, and `SNAP_LIMIT` sets the pivot's lifetime to ~25 minutes, so a small
// devnet finishes its download before the pivot ever moves — leaving the whole
// access-list catch-up path unexercised.
//
// `SECONDS_PER_BLOCK` is here for a subtler reason: `update_pivot` estimates
// how far the chain advanced by dividing elapsed time by it. A devnet running
// faster slots than this value makes that estimate wrong by the ratio, so the
// new pivot lands short of the head and can be stale on arrival.
//
// Only overridable under `sync-test`, so a release binary cannot be talked into
// a degraded sync by its environment.

#[cfg(feature = "sync-test")]
fn override_from_env<T: std::str::FromStr>(name: &str, default: T) -> T {
    match std::env::var(name) {
        Ok(value) => value
            .parse()
            .unwrap_or_else(|_| panic!("{name} is not a valid number: {value:?}")),
        Err(_) => default,
    }
}

#[cfg(feature = "sync-test")]
lazy_static::lazy_static! {
    pub static ref SNAP_LIMIT: usize = override_from_env("SNAP_LIMIT", SNAP_LIMIT_DEFAULT);
    pub static ref MIN_FULL_BLOCKS: u64 = override_from_env("MIN_FULL_BLOCKS", MIN_FULL_BLOCKS_DEFAULT);
    pub static ref SECONDS_PER_BLOCK: u64 = override_from_env("SECONDS_PER_BLOCK", SECONDS_PER_BLOCK_DEFAULT);
}

#[cfg(not(feature = "sync-test"))]
lazy_static::lazy_static! {
    pub static ref SNAP_LIMIT: usize = SNAP_LIMIT_DEFAULT;
    pub static ref MIN_FULL_BLOCKS: u64 = MIN_FULL_BLOCKS_DEFAULT;
    pub static ref SECONDS_PER_BLOCK: u64 = SECONDS_PER_BLOCK_DEFAULT;
}
