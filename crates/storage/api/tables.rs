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

/// Account sate flat key-value store: [`Nibbles`] => [`Vec<u8>`]
/// - [`Nibbles`] = `node_hash.as_ref()`
/// - [`Vec<u8>`] = `node_data`
pub const ACCOUNT_FLATKEYVALUE: &str = "account_flatkeyvalue";

/// Storage slots key-value store: [`Nibbles`] => [`Vec<u8>`]
/// - [`Nibbles`] = `node_hash.as_ref()`
/// - [`Vec<u8>`] = `node_data`
pub const STORAGE_FLATKEYVALUE: &str = "storage_flatkeyvalue";

/// Account flat state column family: [`[u8; 32]`] => [`Vec<u8>`]
/// - Key: `keccak(address)` (32 bytes)
/// - Value: RLP-encoded `AccountState`
pub const ACCOUNT_FLAT: &str = "account_flat";

/// Storage flat state column family: [`[u8; 64]`] => [`Vec<u8>`]
/// - Key: `keccak(address) || keccak(storage_key)` (64 bytes)
/// - Value: RLP-encoded `U256`
pub const STORAGE_FLAT: &str = "storage_flat";

pub const MISC_VALUES: &str = "misc_values";

/// Execution witnesses column family: [`Vec<u8>`] => [`Vec<u8>`]
/// - [`Vec<u8>`] = Composite key
///    ```rust,no_run
///     // let mut composite_key = Vec::with_capacity(8 + 32);
///     // composite_key.extend_from_slice(&block_number.to_be_bytes());
///     // composite_key.extend_from_slice(block_hash.as_bytes());
///    ```
/// - [`Vec<u8>`] = `serde_json::to_vec(&witness)`
pub const EXECUTION_WITNESSES: &str = "execution_witnesses";

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
    RECEIPTS,
    SNAP_STATE,
    INVALID_CHAINS,
    ACCOUNT_TRIE_NODES,
    STORAGE_TRIE_NODES,
    FULLSYNC_HEADERS,
    ACCOUNT_FLATKEYVALUE,
    STORAGE_FLATKEYVALUE,
    ACCOUNT_FLAT,
    STORAGE_FLAT,
    MISC_VALUES,
    EXECUTION_WITNESSES,
];
