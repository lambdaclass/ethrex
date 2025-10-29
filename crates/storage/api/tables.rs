//! Table names used by the storage engine.
pub const CHAIN_DATA: &str = "chain_data";
pub const ACCOUNT_CODES: &str = "account_codes";
pub const BODIES: &str = "bodies";
pub const BLOCK_NUMBERS: &str = "block_numbers";
pub const CANONICAL_BLOCK_HASHES: &str = "canonical_block_hashes";
pub const HEADERS: &str = "headers";
pub const PENDING_BLOCKS: &str = "pending_blocks";
pub const TRANSACTION_LOCATIONS: &str = "transaction_locations";
pub const RECEIPTS: &str = "receipts";
pub const SNAP_STATE: &str = "snap_state";
pub const INVALID_CHAINS: &str = "invalid_chains";
pub const TRIE_NODES: &str = "trie_nodes";
pub const FULLSYNC_HEADERS: &str = "fullsync_headers";
pub const FLATKEY_VALUES: &str = "flatkey_values";
pub const MISC_VALUES: &str = "misc_values";

pub const TABLES: [&str; 15] = [
    CHAIN_DATA,
    ACCOUNT_CODES,
    BODIES,
    BLOCK_NUMBERS,
    CANONICAL_BLOCK_HASHES,
    HEADERS,
    PENDING_BLOCKS,
    TRANSACTION_LOCATIONS,
    RECEIPTS,
    SNAP_STATE,
    INVALID_CHAINS,
    TRIE_NODES,
    FULLSYNC_HEADERS,
    FLATKEY_VALUES,
    MISC_VALUES,
];
