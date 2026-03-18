/// Sub-index for the basic_data leaf (version, nonce, balance, code_size).
pub const BASIC_DATA_LEAF_KEY: u8 = 0;

/// Sub-index for the code_hash leaf (keccak256 of the account's code).
pub const CODE_HASH_LEAF_KEY: u8 = 1;

/// Offset in the stem subtree where header storage slots begin (slots 0–63).
pub const HEADER_STORAGE_OFFSET: u64 = 64;

/// Offset in the stem subtree where code chunks begin.
pub const CODE_OFFSET: u64 = 128;

/// Number of leaf slots per stem subtree (one per sub-index byte value).
pub const STEM_SUBTREE_WIDTH: u64 = 256;

// MAIN_STORAGE_OFFSET = 2^248 (= 256^31)
// This requires a U256 value and is reserved for Phase 2 key-mapping implementation.
