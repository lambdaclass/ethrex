//! Table names used by the storage engine.

/// Canonical block hashes column family: [`u8;_`] => [`Vec<u8>`]
/// - [`u8;_`] = `block_number.to_le_bytes()`
/// - [`Vec<u8>`] = `block_hash.encode_to_vec()`
pub const CANONICAL_BLOCK_HASHES: &str = "canonical_block_hashes";

/// Block numbers column family: [`Vec<u8>`] => [`u8;_`]
/// - [`Vec<u8>`] = `block_hash.encode_to_vec()`
/// - [`u8;_`] = `block_number.to_le_bytes()`
pub const BLOCK_NUMBERS: &str = "block_numbers";

/// Block headers column family: [`Vec<u8>`] => [`Vec<u8>`]
/// - [`Vec<u8>`] = `block_hash.encode_to_vec()`
/// - [`Vec<u8>`] = `BlockHeaderRLP::from(block.header.clone()).bytes().clone()`
pub const HEADERS: &str = "headers";

/// Block bodies column family: [`Vec<u8>`] => [`Vec<u8>`]
/// - [`Vec<u8>`] = `block_hash.encode_to_vec();`
/// - [`Vec<u8>`] = `BlockBodyRLP::from(block.body.clone()).bytes().clone()`
pub const BODIES: &str = "bodies";

/// Account codes column family: [`Vec<u8>`] => [`Vec<u8>`]
/// - [`Vec<u8>`] = `code_hash.as_bytes().to_vec()`
/// - [`Vec<u8>`] = `AccountCodeRLP::from(code).bytes().clone()`
pub const ACCOUNT_CODES: &str = "account_codes";

/// Account code metadata column family: [`Vec<u8>`] => [`u8; 8`]
/// - [`Vec<u8>`] = `code_hash.as_bytes().to_vec()`
/// - [`u8; 8`] = `code_length.to_be_bytes()`
pub const ACCOUNT_CODE_METADATA: &str = "account_code_metadata";

/// Receipts column family: [`Vec<u8>`] => [`Vec<u8>`]
/// - [`Vec<u8>`] = `(block_hash, index).encode_to_vec()`
/// - [`Vec<u8>`] = `receipt.encode_to_vec()`
pub const RECEIPTS: &str = "receipts";

/// Transaction locations column family: [`Vec<u8>`] => [`Vec<u8>`]
/// - [`Vec<u8>`] = Composite key
///    ```rust,no_run
///     // let mut composite_key = Vec::with_capacity(64);
///     // composite_key.extend_from_slice(transaction_hash.as_bytes());
///     // composite_key.extend_from_slice(block_hash.as_bytes());
///    ```
/// - [`Vec<u8>`] = `(block_number, block_hash, index).encode_to_vec()`
pub const TRANSACTION_LOCATIONS: &str = "transaction_locations";

/// Chain data column family: [`Vec<u8>`] => [`Vec<u8>`]
/// - [`Vec<u8>`] = `chain_data_key(ChainDataIndex::ChainConfig)`
/// - [`Vec<u8>`] = `serde_json::to_string(chain_config)`
pub const CHAIN_DATA: &str = "chain_data";

/// Snap state column family: [`Vec<u8>`] => [`Vec<u8>`]
/// - [`Vec<u8>`] = `snap_state_key(SnapStateIndex::HeaderDownloadCheckpoint)`
/// - [`Vec<u8>`] = `block_hash.encode_to_vec()`
pub const SNAP_STATE: &str = "snap_state";

/// Account State trie nodes column family: [`Nibbles`] => [`Vec<u8>`]
/// - [`Nibbles`] = `node_hash.as_ref()`
/// - [`Vec<u8>`] = `node_data`
pub const ACCOUNT_TRIE_NODES: &str = "account_trie_nodes";

/// Storage trie nodes column family: [`Nibbles`] => [`Vec<u8>`]
/// - [`Nibbles`] = `node_hash.as_ref()`
/// - [`Vec<u8>`] = `node_data`
pub const STORAGE_TRIE_NODES: &str = "storage_trie_nodes";

/// Pending blocks column family: [`Vec<u8>`] => [`Vec<u8>`]
/// - [`Vec<u8>`] = `BlockHashRLP::from(block.hash()).bytes().clone()`
/// - [`Vec<u8>`] = `BlockRLP::from(block).bytes().clone()`
pub const PENDING_BLOCKS: &str = "pending_blocks";

/// Invalid ancestors column family: [`Vec<u8>`] => [`Vec<u8>`]
/// - [`Vec<u8>`] = `BlockHashRLP::from(bad_block).bytes().clone()`
/// - [`Vec<u8>`] = `BlockHashRLP::from(latest_valid).bytes().clone()`
pub const INVALID_CHAINS: &str = "invalid_ancestors";

/// Block headers downloaded during fullsync column family: [`u8;_`] => [`Vec<u8>`]
/// - [`u8;_`] = `block_number.to_le_bytes()`
/// - [`Vec<u8>`] = `BlockHeaderRLP::from(block.header.clone()).bytes().clone()`
pub const FULLSYNC_HEADERS: &str = "fullsync_headers";

/// Account flat key-value store: MPT nibble-path bytes => RLP-encoded value
///
/// Format: MPT nibble-path bytes. Currently shared across backends via a
/// format discriminator stored in `MISC_VALUES` under key
/// `state_backend_format`. A store opened with a mismatched `BackendKind`
/// will refuse to start.
pub const ACCOUNT_FLATKEYVALUE: &str = "account_flatkeyvalue";

/// Storage slots flat key-value store: MPT nibble-path bytes => RLP-encoded value
///
/// Format: MPT nibble-path bytes. Currently shared across backends via a
/// format discriminator stored in `MISC_VALUES` under key
/// `state_backend_format`. A store opened with a mismatched `BackendKind`
/// will refuse to start.
pub const STORAGE_FLATKEYVALUE: &str = "storage_flatkeyvalue";

pub const MISC_VALUES: &str = "misc_values";

/// Key used in `MISC_VALUES` to store the flat state format discriminator.
///
/// Value encoding: 1 byte.
/// - `0` = MPT nibble-path format (current default).
/// - `1` = Binary trie format (`BackendKind::Binary`).
/// - `2` = Transition format (`BackendKind::Transition`): MPT frozen at switch
///   block, binary overlay active. Decoded on startup to reconstruct
///   `TransitionBackend`. (Phase 6+)
///
/// When a new backend is added it must claim a unique byte value here and
/// document it alongside this constant.
pub const STATE_BACKEND_FORMAT_KEY: &[u8] = b"state_backend_format";

/// Key in `MISC_VALUES` storing the block number at which the MPT→binary
/// transition was activated.
///
/// Value encoding: 8 bytes, big-endian u64.
/// Written atomically alongside `TRANSITION_MPT_FROZEN_ROOT_KEY` and
/// `TRANSITION_BINARY_ROOT_KEY` when `Store::persist_transition_metadata` fires.
pub const TRANSITION_SWITCH_BLOCK_KEY: &[u8] = b"transition_switch_block";

/// Key in `MISC_VALUES` storing the MPT state root at the switch block.
///
/// Value encoding: 32 bytes (raw H256).
/// This is the post-state root of the last block processed entirely by MPT,
/// i.e. `header.state_root` of the block at `switch_block - 1`.
pub const TRANSITION_MPT_FROZEN_ROOT_KEY: &[u8] = b"transition_mpt_frozen_root";

/// Key in `MISC_VALUES` storing the binary trie root at the switch block.
///
/// Value encoding: 32 bytes (raw H256).
/// On activation this is written as `EMPTY_BINARY_ROOT` (`H256([0u8; 32])`).
/// After each block committed in transition mode the storage layer updates
/// this key with the new binary overlay root.
pub const TRANSITION_BINARY_ROOT_KEY: &[u8] = b"transition_binary_root";

/// Binary trie node store column family.
///
/// Key space:
/// - `[u64 LE; 8 bytes]` = serialized binary trie node (InternalNode or StemNode).
/// - `[0xFF, ...]` (any 0xFF-prefixed key) = metadata: `META_ROOT`, `META_NEXT_ID`,
///   `META_BLOCK_KEY`, `META_BASE_HASH_KEY` (see `node_store.rs`).
/// - `[0xFE, stem...; 32 bytes]` = tombstone marker for a SELFDESTRUCTed stem.
///   Value is an empty slice (presence-only). Must be disjoint from valid 8-byte
///   NodeId LE keys and the 0xFF meta-key range — documented here and in `node_store.rs`.
pub const BINARY_TRIE_NODES: &str = "BinaryTrieNodes";

/// Binary trie flat key-value store.
///
/// Stores per-leaf state values for O(1) state reads without trie traversal.
/// Populated inline by `binary_commit_nodes_to_disk` on every commit.
///
/// Key: 32-byte `stem[0..31] || sub_index[0..1]` (canonical binary trie tree key).
/// Value: 32-byte raw leaf value (packed BASIC_DATA for sub-index 0, raw
///        code_hash for sub-index 1, raw U256 BE for storage slots).
///
/// This is a **separate** table from `ACCOUNT_FLATKEYVALUE` / `STORAGE_FLATKEYVALUE`
/// (those are MPT-specific). Using a separate table eliminates backend-discriminator
/// coupling and keeps each backend's FKV loop entirely self-contained.
pub const BINARY_FLATKEYVALUE: &str = "BinaryFlatKeyValue";

/// Binary trie storage-key side-index column family.
///
/// Tracks which storage keys each account has written into the binary trie.
/// Required for SELFDESTRUCT (`removed_storage`) since the binary trie has no
/// prefix-enumeration — all storage keys for an address must be tracked here.
///
/// Key:   20-byte address (`addr.as_bytes()`).
/// Value: packed list of 32-byte storage keys (`N * 32` bytes, where N is the
///        number of distinct storage slots ever written for this address).
///
/// Populated by `BinaryTrieState::flush` (via `prepare_flush`) for dirty entries.
/// Read at `BinaryTrieState::open` time to restore the in-memory side-index.
pub const BINARY_STORAGE_KEYS: &str = "BinaryStorageKeys";

/// Execution witnesses column family: [`Vec<u8>`] => [`Vec<u8>`]
/// - [`Vec<u8>`] = Composite key
///    ```rust,no_run
///     // let mut composite_key = Vec::with_capacity(8 + 32);
///     // composite_key.extend_from_slice(&block_number.to_be_bytes());
///     // composite_key.extend_from_slice(block_hash.as_bytes());
///    ```
/// - [`Vec<u8>`] = `serde_json::to_vec(&witness)`
pub const EXECUTION_WITNESSES: &str = "execution_witnesses";

pub const TABLES: [&str; 22] = [
    CHAIN_DATA,
    ACCOUNT_CODES,
    ACCOUNT_CODE_METADATA,
    BODIES,
    BLOCK_NUMBERS,
    CANONICAL_BLOCK_HASHES,
    HEADERS,
    PENDING_BLOCKS,
    TRANSACTION_LOCATIONS,
    RECEIPTS,
    SNAP_STATE,
    INVALID_CHAINS,
    ACCOUNT_TRIE_NODES,
    STORAGE_TRIE_NODES,
    FULLSYNC_HEADERS,
    ACCOUNT_FLATKEYVALUE,
    STORAGE_FLATKEYVALUE,
    MISC_VALUES,
    EXECUTION_WITNESSES,
    BINARY_TRIE_NODES,
    BINARY_FLATKEYVALUE,
    BINARY_STORAGE_KEYS,
];
