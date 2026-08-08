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
/// - Value: `receipt.encode_storage()` (internal storage codec; NOT the
///   wire/consensus format — byte-identical to `encode_to_vec()` for
///   non-frame receipts, full-fidelity layout for frame receipts)
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

/// EIP-8297 binary trie nodes column family: [`Vec<u8>`] => [`Vec<u8>`]
/// - [`Vec<u8>`] = `BitPath::to_db_key()` — the node's **bit path from the
///   root**, not a node hash: a four-byte big-endian bit count followed by the
///   path's bits packed MSB-first. The count is what makes the key injective;
///   without it `[1]` and `[1, 0]` pack to identical bytes.
/// - [`Vec<u8>`] = the node's stored encoding (leaf or branch), which is also
///   its hashing preimage.
///
/// An **empty value is a tombstone**, not a stored node: it means the node at
/// that path left the tree and the key must be deleted, so a later read answers
/// `None`. No node encodes to zero bytes — every encoding starts with a tag —
/// so the two cannot be confused. See `BackendBinaryTrieDB` in
/// `crates/storage/binary_trie.rs`.
///
/// Single-version and path-keyed, the same storage model the MPT node tables
/// use: a node that changes overwrites itself in place.
pub const BINARY_TRIE_NODES: &str = "binary_trie_nodes";

/// EIP-8297 binary trie flat key-value mirror: [`Vec<u8>`] => [`Vec<u8>`]
/// - [`Vec<u8>`] = the leaf's **tree key**, exactly as the embedding derives it:
///   34 bytes in the account and code zones, 66 in the storage zone. Not a bit
///   path and not a hash — the raw key, so that the column family's bytewise
///   order *is* the tree's leaf order. That holds because keys expand to bits
///   MSB-first and the key set is prefix-free; see the binary-trie crate's
///   `key_ordering` tests, which exist to keep it true.
/// - [`Vec<u8>`] = the leaf's 32-byte value, raw. No tag and no length prefix:
///   every leaf value in this tree is exactly 32 bytes, so there is nothing to
///   delimit.
///
/// An **empty value is a tombstone**, not a stored leaf: it means the key left
/// the tree and the row must be deleted, so a later read answers `None`. That
/// is the same convention [`BINARY_TRIE_NODES`] uses, and it is *not* the same
/// as a value of 32 zero bytes — which must never be written at all. The state
/// embedding resolves a zero-valued leaf to a removal ("zero means absent"), so
/// a 32-zero-byte row would claim a key the trie's root does not commit to, and
/// a range served from this table and proved against that root would fail on
/// it. `value.is_empty()` is the tombstone test; `value == [0u8; 32]` is an
/// invariant violation. See `BackendBinaryFlatDB` in
/// `crates/storage/binary_trie.rs`.
///
/// **Derived, not authoritative.** The trie is the state; this table is a
/// mirror of its leaves that exists so a leaf can be read in one lookup instead
/// of a ~256-bit descent, and so leaves can be enumerated in key order, which
/// the node table cannot do at any price — [`BINARY_TRIE_NODES`] is keyed by
/// bit path behind a bit *count*, so it sorts breadth-first. Every row here is
/// produced by a change the trie was already told about, and the whole table
/// can be dropped and rebuilt from the trie.
///
/// Single-version, like the trie it mirrors: one row per key, overwritten in
/// place, no history.
pub const BINARY_FLATKEYVALUE: &str = "binary_flatkeyvalue";

/// EIP-8297 binary-trie root by block hash: [`Vec<u8>`] => [`Vec<u8>`]
/// - [`Vec<u8>`] = `block_hash.as_bytes()`
/// - [`Vec<u8>`] = the block's binary-trie root, 32 raw bytes.
///
/// **Scope: block import only, and only while the commitment is scheduled but
/// not yet active.** During that window a header commits the *MPT* root, so
/// nothing in a header names the binary root a block must extend from; this
/// table is how a block finds its parent's. It is consulted by exactly one
/// caller — [`Store::advance_binary_trie_for_block`] — and by no read path:
/// nothing resolving state from a header ever looks here. Once headers carry
/// the binary root (activation), a post-flip header addresses the binary trie
/// the same way a pre-flip header addresses the MPT and this table becomes
/// redundant.
///
/// Deliberately *not* the `mpt_lookup_roots` registry the earlier in-memory
/// branch needed: that one had to be swept through fork choice, `newPayload`,
/// `eth_syncing`, sync resume points, tracing and the L2 committer, because a
/// post-flip header no longer named an MPT root. With the binary trie
/// persisted and path-keyed, `header.state_root` keeps resolving on its own.
///
/// [`Store::advance_binary_trie_for_block`]: crate::Store::advance_binary_trie_for_block
pub const BINARY_TRIE_ROOTS: &str = "binary_trie_roots";

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

/// Account state flat key-value mirror of the MPT: [`Vec<u8>`] => [`Vec<u8>`]
/// - [`Vec<u8>`] = the account's **leaf path** in the state trie, one byte per
///   nibble, 65 bytes long. Not a node hash: this table is keyed by where the
///   leaf sits, so a lookup answers without descending. `classify_trie_key`
///   (`crates/storage/trie.rs`) dispatches on exactly that length.
/// - [`Vec<u8>`] = the leaf's own value, the RLP-encoded `AccountState`.
///
/// An empty value is a tombstone: the row is deleted and a read answers `None`
/// (`Trie::get`, `crates/common/trie/trie.rs`). Coverage is progressive — see
/// `TrieDB::flatkeyvalue_computed` and the `last_written` frontier in
/// [`MISC_VALUES`] — so a miss is authoritative absence only for keys the
/// frontier already covers.
pub const ACCOUNT_FLATKEYVALUE: &str = "account_flatkeyvalue";

/// Storage slot flat key-value mirror of the MPT: [`Vec<u8>`] => [`Vec<u8>`]
/// - [`Vec<u8>`] = the account's address prefix followed by the slot's leaf
///   path in that account's storage trie, 131 bytes long. The length is what
///   tells this table's keys apart from [`ACCOUNT_FLATKEYVALUE`]'s.
/// - [`Vec<u8>`] = the leaf's own value, the RLP-encoded slot value.
///
/// Same tombstone and progressive-coverage rules as [`ACCOUNT_FLATKEYVALUE`].
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

/// Bad blocks column family: single-keyed list of the most recent bad blocks
/// seen by the client, served by `debug_getBadBlocks`.
/// - [`Vec<u8>`] = [`BAD_BLOCKS_KEY`]
/// - [`Vec<u8>`] = RLP-encoded `Vec<Block>` (sorted by descending block number)
pub const BAD_BLOCKS: &str = "bad_blocks";

pub const TABLES: [&str; 25] = [
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
    BINARY_TRIE_NODES,
    BINARY_FLATKEYVALUE,
    BINARY_TRIE_ROOTS,
    FULLSYNC_HEADERS,
    ACCOUNT_FLATKEYVALUE,
    STORAGE_FLATKEYVALUE,
    MISC_VALUES,
    EXECUTION_WITNESSES,
    BLOCK_ACCESS_LISTS,
    STATE_HISTORY,
    BAD_BLOCKS,
];
