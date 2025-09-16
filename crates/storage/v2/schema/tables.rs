/// Defines all the logical tables needed for Ethereum storage
///
/// These correspond to the different data types that the current StoreEngine manages,
/// but without coupling to any specific database implementation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DBTable {
    // Block data
    Headers,
    Bodies,
    BlockNumbers,
    CanonicalHashes,

    // Transaction data
    TransactionLocations,
    Receipts,

    // Account data
    AccountCodes,

    // Trie data
    StateTrieNodes,
    StorageTrieNodes,

    // Chain metadata
    ChainData,

    // Snap sync data
    SnapState,
    StorageSnapshot,

    // Pending data
    PendingBlocks,

    // Error tracking
    InvalidAncestors,
}

impl DBTable {
    /// Returns the namespace string for this table
    pub fn namespace(&self) -> &'static str {
        match self {
            Self::Headers => "headers",
            Self::Bodies => "bodies",
            Self::BlockNumbers => "block_numbers",
            Self::CanonicalHashes => "canonical_hashes",
            Self::TransactionLocations => "transaction_locations",
            Self::Receipts => "receipts",
            Self::AccountCodes => "account_codes",
            Self::StateTrieNodes => "state_trie_nodes",
            Self::StorageTrieNodes => "storage_trie_nodes",
            Self::ChainData => "chain_data",
            Self::SnapState => "snap_state",
            Self::StorageSnapshot => "storage_snapshot",
            Self::PendingBlocks => "pending_blocks",
            Self::InvalidAncestors => "invalid_ancestors",
        }
    }

    /// Returns all table variants
    pub fn all() -> &'static [DBTable] {
        &[
            Self::Headers,
            Self::Bodies,
            Self::BlockNumbers,
            Self::CanonicalHashes,
            Self::TransactionLocations,
            Self::Receipts,
            Self::AccountCodes,
            Self::StateTrieNodes,
            Self::StorageTrieNodes,
            Self::ChainData,
            Self::SnapState,
            Self::StorageSnapshot,
            Self::PendingBlocks,
            Self::InvalidAncestors,
        ]
    }
}

impl From<&str> for DBTable {
    fn from(value: &str) -> Self {
        match value {
            "headers" => Self::Headers,
            "bodies" => Self::Bodies,
            "block_numbers" => Self::BlockNumbers,
            "canonical_hashes" => Self::CanonicalHashes,
            "transaction_locations" => Self::TransactionLocations,
            "receipts" => Self::Receipts,
            "account_codes" => Self::AccountCodes,
            "state_trie_nodes" => Self::StateTrieNodes,
            "storage_trie_nodes" => Self::StorageTrieNodes,
            "chain_data" => Self::ChainData,
            "snap_state" => Self::SnapState,
            "storage_snapshot" => Self::StorageSnapshot,
            "pending_blocks" => Self::PendingBlocks,
            "invalid_ancestors" => Self::InvalidAncestors,
            _ => panic!("Invalid table name: {}", value),
        }
    }
}
