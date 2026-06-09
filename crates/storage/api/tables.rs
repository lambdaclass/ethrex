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

/// Receipts column family (legacy, pre-v2): [`Vec<u8>`] => [`Vec<u8>`]
/// Used only for migration reads (v1→v2). Not listed in `TABLES`, so
/// `drop_obsolete_cfs()` removes it right after migration completes
/// (same startup).
pub const RECEIPTS: &str = "receipts";

/// Receipts v2 column family: [`Vec<u8>`] => [`Vec<u8>`]
/// - Key: `block_hash (32B) || index (8B big-endian u64)` — fixed-width raw key
///   enabling cursor-based prefix iteration by block hash.
/// - Value: `receipt.encode_to_vec()`
pub const RECEIPTS_V2: &str = "receipts_v2";

/// Transaction locations column family: [`Vec<u8>`] => [`Vec<u8>`]
/// - Key: `transaction_hash.as_bytes()` (32 bytes)
/// - Value: `Vec<(block_number, block_hash, index)>.encode_to_vec()`
///
/// The value is a list because, in the rare case of a reorg, the same
/// transaction may appear in multiple blocks. Readers must filter by the
/// canonical chain to pick the right `(block_number, block_hash, index)`.
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

/// Account sate flat key-value store: [`Nibbles`] => [`Vec<u8>`]
/// - [`Nibbles`] = `node_hash.as_ref()`
/// - [`Vec<u8>`] = `node_data`
pub const ACCOUNT_FLATKEYVALUE: &str = "account_flatkeyvalue";

/// Storage slots key-value store: [`Nibbles`] => [`Vec<u8>`]
/// - [`Nibbles`] = `node_hash.as_ref()`
/// - [`Vec<u8>`] = `node_data`
pub const STORAGE_FLATKEYVALUE: &str = "storage_flatkeyvalue";

pub const MISC_VALUES: &str = "misc_values";

/// State-history journal column family: [`u8; 8`] => [`Vec<u8>`]
/// - [`u8; 8`] = `block_number.to_be_bytes()` (big-endian so lex order == numeric order)
/// - [`Vec<u8>`] = `JournalEntry::encode()`
///
/// Stores one reverse-diff entry per committed block, enabling reorgs deeper
/// than the in-memory `TrieLayerCache`. Pruned at finality.
pub const STATE_HISTORY: &str = "state_history";

/// Execution witnesses column family: [`Vec<u8>`] => [`Vec<u8>`]
/// - [`Vec<u8>`] = Composite key
///    ```rust,no_run
///     // let mut composite_key = Vec::with_capacity(8 + 32);
///     // composite_key.extend_from_slice(&block_number.to_be_bytes());
///     // composite_key.extend_from_slice(block_hash.as_bytes());
///    ```
/// - [`Vec<u8>`] = `serde_json::to_vec(&witness)`
pub const EXECUTION_WITNESSES: &str = "execution_witnesses";

/// Block access lists column family: [`Vec<u8>`] => [`Vec<u8>`]
/// - [`Vec<u8>`] = `block_hash.as_bytes().to_vec()`
/// - [`Vec<u8>`] = RLP-encoded `BlockAccessList`
pub const BLOCK_ACCESS_LISTS: &str = "block_access_lists";

pub const TABLES: [&str; 21] = [
    CHAIN_DATA,
    ACCOUNT_CODES,
    ACCOUNT_CODE_METADATA,
    BODIES,
    BLOCK_NUMBERS,
    CANONICAL_BLOCK_HASHES,
    HEADERS,
    PENDING_BLOCKS,
    TRANSACTION_LOCATIONS,
    RECEIPTS_V2,
    SNAP_STATE,
    INVALID_CHAINS,
    ACCOUNT_TRIE_NODES,
    STORAGE_TRIE_NODES,
    FULLSYNC_HEADERS,
    ACCOUNT_FLATKEYVALUE,
    STORAGE_FLATKEYVALUE,
    MISC_VALUES,
    EXECUTION_WITNESSES,
    BLOCK_ACCESS_LISTS,
    STATE_HISTORY,
];
