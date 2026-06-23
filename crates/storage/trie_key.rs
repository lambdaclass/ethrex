use crate::api::tables::{
    ACCOUNT_FLATKEYVALUE, ACCOUNT_TRIE_NODES, STORAGE_FLATKEYVALUE, STORAGE_TRIE_NODES,
};
use ethrex_common::{Address, H256};
use ethrex_crypto::keccak::keccak_hash;
use ethrex_trie::Nibbles;

/// Nibble value that separates the account prefix from the storage path.
///
/// Value 17 is deliberately outside the valid nibble range (0–15), making it
/// an unambiguous separator in the concatenated key.
pub const SEPARATOR_NIBBLE: u8 = 17;

/// Length (in nibbles) of an account leaf key:
/// 64 nibbles from keccak(address) + 1 leaf-flag nibble.
pub const ACCOUNT_LEAF_LEN: usize = 65;

/// Length (in nibbles) of a storage leaf key:
/// 65 (account prefix with separator) + 64 (keccak(slot)) + 2 (leaf flags) = 131.
pub const STORAGE_LEAF_LEN: usize = 131;

/// A fully-qualified key for the trie/FKV database.
///
/// Wraps a nibble path and provides methods for table routing and
/// conversions. Constructed via typed constructors that handle hashing,
/// nibble expansion, and prefix concatenation internally.
#[derive(Debug, Clone)]
pub struct TrieKey {
    nibbles: Nibbles,
}

impl TrieKey {
    // ── Constructors ─────────────────────────────────────────────

    /// Hash an address and return its nibble path (64 nibbles + leaf flag = 65).
    /// Used to look up an account in the state trie or FKV.
    pub fn from_account_address(address: &Address) -> Self {
        let hash = keccak_hash(address.to_fixed_bytes());
        Self {
            nibbles: Nibbles::from_bytes(&hash),
        }
    }

    /// Build from a pre-computed account hash (H256).
    pub fn from_account_hash(hash: H256) -> Self {
        Self {
            nibbles: Nibbles::from_bytes(hash.as_bytes()),
        }
    }

    /// Build the prefix used for all storage keys of a given account:
    /// `nibbles(keccak(address)) + [17]` (65 nibbles).
    pub fn storage_prefix(address_hash: H256) -> Nibbles {
        Nibbles::from_bytes(address_hash.as_bytes()).append_new(SEPARATOR_NIBBLE)
    }

    /// Build a full storage leaf key:
    /// `nibbles(keccak(address)) + [17] + nibbles(keccak(slot))`.
    pub fn from_storage_slot(address_hash: H256, slot: &H256) -> Self {
        let prefix = Self::storage_prefix(address_hash);
        let slot_hash = keccak_hash(slot.to_fixed_bytes());
        let slot_nibbles = Nibbles::from_bytes(&slot_hash);
        Self {
            nibbles: prefix.concat(&slot_nibbles),
        }
    }

    /// Wrap an existing nibble path as a TrieKey (for internal/intermediate nodes).
    pub fn from_nibbles(nibbles: Nibbles) -> Self {
        Self { nibbles }
    }

    /// Apply an optional account prefix to a raw trie path.
    /// For account tries (prefix=None), returns the path unchanged.
    /// For storage tries (prefix=Some(hash)), prepends the storage prefix.
    pub fn with_prefix(prefix: Option<H256>, path: Nibbles) -> Self {
        match prefix {
            Some(hash) => {
                let prefixed = Self::storage_prefix(hash).concat(&path);
                Self { nibbles: prefixed }
            }
            None => Self { nibbles: path },
        }
    }

    // ── Table routing ────────────────────────────────────────────

    /// Determine which RocksDB column family this key belongs to.
    pub fn table(&self) -> &'static str {
        let len = self.nibbles.len();
        let is_leaf = len == ACCOUNT_LEAF_LEN || len == STORAGE_LEAF_LEN;
        let is_account = len <= ACCOUNT_LEAF_LEN;

        if is_leaf {
            if is_account {
                ACCOUNT_FLATKEYVALUE
            } else {
                STORAGE_FLATKEYVALUE
            }
        } else if is_account {
            ACCOUNT_TRIE_NODES
        } else {
            STORAGE_TRIE_NODES
        }
    }

    /// Whether this key points to a leaf (account or storage value) vs an internal node.
    pub fn is_leaf(&self) -> bool {
        let len = self.nibbles.len();
        len == ACCOUNT_LEAF_LEN || len == STORAGE_LEAF_LEN
    }

    /// Whether this key is in the account trie (vs storage trie).
    pub fn is_account(&self) -> bool {
        self.nibbles.len() <= ACCOUNT_LEAF_LEN
    }

    // ── Conversions ──────────────────────────────────────────────

    /// Get the underlying Nibbles.
    pub fn nibbles(&self) -> &Nibbles {
        &self.nibbles
    }

    /// Consume and return the Nibbles.
    pub fn into_nibbles(self) -> Nibbles {
        self.nibbles
    }

    /// Convert to a byte vector (each nibble as one u8). Used as the RocksDB key.
    pub fn into_vec(self) -> Vec<u8> {
        self.nibbles.into_vec()
    }

    /// Borrow as byte slice (each nibble as one u8). Used for RocksDB lookups.
    pub fn as_bytes(&self) -> &[u8] {
        self.nibbles.as_ref()
    }
}

// ── Standalone hash helpers ──────────────────────────────────────
//
// These are thin wrappers kept for call sites that only need the hash
// (not a full TrieKey).

/// Keccak-256 hash of an Ethereum address. Returns H256.
pub fn hash_address(address: &Address) -> H256 {
    H256(keccak_hash(address.to_fixed_bytes()))
}

/// Keccak-256 hash of a storage key. Returns the raw 32 bytes.
pub fn hash_storage_key(key: &H256) -> [u8; 32] {
    keccak_hash(key.to_fixed_bytes())
}
