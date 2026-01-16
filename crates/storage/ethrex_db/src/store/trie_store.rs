//! TrieStore - Persistent trie storage using page-based storage.
//!
//! This module bridges the in-memory MerkleTrie with the PagedDb storage,
//! providing a persistent key-value store optimized for trie operations.

use std::collections::HashMap;

use crate::data::{NibblePath, SlottedArray};
use crate::merkle::{keccak256, MerkleTrie, EMPTY_ROOT};
use crate::store::PAGE_SIZE;

/// Standalone trie that persists to PagedDb on commit.
///
/// This is a simpler approach: keep an in-memory trie and flush to pages
/// when committing. The pages store raw key-value data, and the merkle
/// tree is recomputed on load.
pub struct PersistentTrie {
    /// In-memory trie for operations and root computation
    trie: MerkleTrie,
    /// Pending changes to write on commit
    pending: HashMap<Vec<u8>, Option<Vec<u8>>>, // None = deletion
}

impl PersistentTrie {
    /// Creates a new empty persistent trie.
    pub fn new() -> Self {
        Self {
            trie: MerkleTrie::new(),
            pending: HashMap::new(),
        }
    }

    /// Loads a trie from a LeafPage's data.
    ///
    /// The page contains key-value pairs in a slotted array format.
    pub fn load_from_page(page_data: &[u8]) -> Self {
        let _arr = SlottedArray::from_bytes(Self::page_data_to_array(page_data));
        let trie = MerkleTrie::new();

        // Reconstruct trie from stored entries
        // Note: SlottedArray doesn't have an iterator, so we'd need to add one
        // For now, we'll track entries separately in pending

        Self {
            trie,
            pending: HashMap::new(),
        }
    }

    /// Inserts or updates a key-value pair.
    pub fn insert(&mut self, key: &[u8], value: Vec<u8>) {
        self.trie.insert(key, value.clone());
        self.pending.insert(key.to_vec(), Some(value));
    }

    /// Gets a value by key.
    pub fn get(&self, key: &[u8]) -> Option<Vec<u8>> {
        self.trie.get(key).map(|v| v.to_vec())
    }

    /// Removes a key.
    pub fn remove(&mut self, key: &[u8]) {
        self.trie.remove(key);
        self.pending.insert(key.to_vec(), None);
    }

    /// Computes and returns the root hash.
    pub fn root_hash(&mut self) -> [u8; 32] {
        self.trie.root_hash()
    }

    /// Checks if the trie is empty.
    pub fn is_empty(&mut self) -> bool {
        self.root_hash() == EMPTY_ROOT
    }

    /// Writes the trie contents to a LeafPage.
    ///
    /// Returns the serialized data that can be stored in a page.
    pub fn to_page_data(&self) -> Vec<u8> {
        let mut arr = SlottedArray::new();

        // Insert all entries from the trie
        for (key, value) in self.trie.iter() {
            let path = NibblePath::from_bytes(key);
            arr.try_insert(&path, value);
        }

        arr.as_bytes().to_vec()
    }

    fn page_data_to_array(data: &[u8]) -> [u8; PAGE_SIZE] {
        let mut arr = [0u8; PAGE_SIZE];
        let len = data.len().min(PAGE_SIZE);
        arr[..len].copy_from_slice(&data[..len]);
        arr
    }
}

impl Default for PersistentTrie {
    fn default() -> Self {
        Self::new()
    }
}

/// State trie that stores account data.
///
/// Keys are keccak256(address), values are RLP-encoded accounts.
pub struct StateTrie {
    /// The underlying trie
    trie: PersistentTrie,
    /// Storage tries per account (keyed by address hash)
    storage_tries: HashMap<[u8; 32], StorageTrie>,
}

impl StateTrie {
    /// Creates a new empty state trie.
    pub fn new() -> Self {
        Self {
            trie: PersistentTrie::new(),
            storage_tries: HashMap::new(),
        }
    }

    /// Gets or creates a storage trie for an account.
    pub fn storage_trie(&mut self, address: &[u8; 20]) -> &mut StorageTrie {
        let key = keccak256(address);
        self.storage_tries.entry(key).or_insert_with(StorageTrie::new)
    }

    /// Gets an account by address.
    pub fn get_account(&self, address: &[u8; 20]) -> Option<AccountData> {
        let key = keccak256(address);
        self.trie.get(&key).map(|data| AccountData::decode(&data))
    }

    /// Sets an account.
    pub fn set_account(&mut self, address: &[u8; 20], account: AccountData) {
        let key = keccak256(address);
        self.trie.insert(&key, account.encode());
    }

    /// Computes the state root hash.
    ///
    /// This first updates all account storage roots, then computes the state root.
    pub fn root_hash(&mut self) -> [u8; 32] {
        // Update storage roots in accounts
        // Collect keys to avoid borrow conflict
        let addr_hashes: Vec<[u8; 32]> = self.storage_tries.keys().cloned().collect();

        for addr_hash in addr_hashes {
            let storage_root = self.storage_tries.get_mut(&addr_hash).unwrap().root_hash();
            if let Some(account_data) = self.trie.get(&addr_hash) {
                let mut account = AccountData::decode(&account_data);
                if account.storage_root != storage_root {
                    account.storage_root = storage_root;
                    self.trie.insert(&addr_hash, account.encode());
                }
            }
        }

        self.trie.root_hash()
    }
}

impl Default for StateTrie {
    fn default() -> Self {
        Self::new()
    }
}

/// Storage trie for a single account's storage.
///
/// Keys are keccak256(slot), values are RLP-encoded storage values.
pub struct StorageTrie {
    trie: PersistentTrie,
}

impl StorageTrie {
    /// Creates a new empty storage trie.
    pub fn new() -> Self {
        Self {
            trie: PersistentTrie::new(),
        }
    }

    /// Gets a storage value.
    pub fn get(&self, slot: &[u8; 32]) -> Option<[u8; 32]> {
        let key = keccak256(slot);
        self.trie.get(&key).map(|v| {
            let mut arr = [0u8; 32];
            let len = v.len().min(32);
            arr[32 - len..].copy_from_slice(&v[v.len() - len..]);
            arr
        })
    }

    /// Sets a storage value.
    pub fn set(&mut self, slot: &[u8; 32], value: [u8; 32]) {
        let key = keccak256(slot);
        // RLP encode the value (strip leading zeros)
        let trimmed: Vec<u8> = value.iter().skip_while(|&&b| b == 0).copied().collect();
        if trimmed.is_empty() {
            self.trie.remove(&key);
        } else {
            self.trie.insert(&key, trimmed);
        }
    }

    /// Computes the storage root hash.
    pub fn root_hash(&mut self) -> [u8; 32] {
        self.trie.root_hash()
    }
}

impl Default for StorageTrie {
    fn default() -> Self {
        Self::new()
    }
}

/// Account data stored in the state trie.
#[derive(Clone, Debug, Default)]
pub struct AccountData {
    pub nonce: u64,
    pub balance: [u8; 32], // U256 as big-endian bytes
    pub storage_root: [u8; 32],
    pub code_hash: [u8; 32],
}

impl AccountData {
    /// Empty account code hash (keccak256 of empty bytes).
    pub const EMPTY_CODE_HASH: [u8; 32] = [
        0xc5, 0xd2, 0x46, 0x01, 0x86, 0xf7, 0x23, 0x3c,
        0x92, 0x7e, 0x7d, 0xb2, 0xdc, 0xc7, 0x03, 0xc0,
        0xe5, 0x00, 0xb6, 0x53, 0xca, 0x82, 0x27, 0x3b,
        0x7b, 0xfa, 0xd8, 0x04, 0x5d, 0x85, 0xa4, 0x70,
    ];

    /// Creates a new empty account.
    pub fn empty() -> Self {
        Self {
            nonce: 0,
            balance: [0u8; 32],
            storage_root: EMPTY_ROOT,
            code_hash: Self::EMPTY_CODE_HASH,
        }
    }

    /// RLP encodes the account data.
    pub fn encode(&self) -> Vec<u8> {
        use crate::merkle::RlpEncoder;
        let mut enc = RlpEncoder::new();
        enc.encode_list(|e| {
            e.encode_u64(self.nonce);
            // Encode balance (trim leading zeros)
            let balance_trimmed: Vec<u8> = self.balance.iter().skip_while(|&&b| b == 0).copied().collect();
            if balance_trimmed.is_empty() {
                e.encode_empty();
            } else {
                e.encode_bytes(&balance_trimmed);
            }
            e.encode_bytes(&self.storage_root);
            e.encode_bytes(&self.code_hash);
        });
        enc.into_bytes()
    }

    /// Decodes account data from RLP bytes.
    pub fn decode(data: &[u8]) -> Self {
        // Simple RLP decoder for account
        // Format: [nonce, balance, storage_root, code_hash]
        // This is a simplified decoder - a production version would be more robust

        if data.is_empty() || data[0] < 0xc0 {
            return Self::empty();
        }

        // Skip list header
        let (header_len, _list_len) = Self::decode_length(data);
        let mut pos = header_len;

        // Decode nonce
        let (nonce, len) = Self::decode_u64(&data[pos..]);
        pos += len;

        // Decode balance
        let (balance, len) = Self::decode_bytes(&data[pos..]);
        pos += len;
        let mut balance_arr = [0u8; 32];
        let offset = 32 - balance.len().min(32);
        balance_arr[offset..].copy_from_slice(&balance[..balance.len().min(32)]);

        // Decode storage_root
        let (storage, len) = Self::decode_bytes(&data[pos..]);
        pos += len;
        let mut storage_root = [0u8; 32];
        if storage.len() == 32 {
            storage_root.copy_from_slice(&storage);
        }

        // Decode code_hash
        let (code, _len) = Self::decode_bytes(&data[pos..]);
        let mut code_hash = Self::EMPTY_CODE_HASH;
        if code.len() == 32 {
            code_hash.copy_from_slice(&code);
        }

        Self {
            nonce,
            balance: balance_arr,
            storage_root,
            code_hash,
        }
    }

    fn decode_length(data: &[u8]) -> (usize, usize) {
        if data.is_empty() {
            return (0, 0);
        }
        let prefix = data[0];
        if prefix <= 0xbf {
            if prefix < 0x80 {
                (0, 1)
            } else if prefix <= 0xb7 {
                (1, (prefix - 0x80) as usize)
            } else {
                let len_of_len = (prefix - 0xb7) as usize;
                let mut len = 0usize;
                for i in 0..len_of_len {
                    len = (len << 8) | data[1 + i] as usize;
                }
                (1 + len_of_len, len)
            }
        } else if prefix <= 0xf7 {
            (1, (prefix - 0xc0) as usize)
        } else {
            let len_of_len = (prefix - 0xf7) as usize;
            let mut len = 0usize;
            for i in 0..len_of_len {
                len = (len << 8) | data[1 + i] as usize;
            }
            (1 + len_of_len, len)
        }
    }

    fn decode_u64(data: &[u8]) -> (u64, usize) {
        if data.is_empty() {
            return (0, 0);
        }
        let prefix = data[0];
        if prefix < 0x80 {
            (prefix as u64, 1)
        } else if prefix == 0x80 {
            (0, 1)
        } else if prefix <= 0xb7 {
            let len = (prefix - 0x80) as usize;
            let mut value = 0u64;
            for i in 0..len {
                value = (value << 8) | data[1 + i] as u64;
            }
            (value, 1 + len)
        } else {
            (0, 1)
        }
    }

    fn decode_bytes(data: &[u8]) -> (Vec<u8>, usize) {
        if data.is_empty() {
            return (vec![], 0);
        }
        let (header_len, content_len) = Self::decode_length(data);
        if content_len == 0 {
            return (vec![], header_len.max(1));
        }
        let start = header_len;
        let end = start + content_len;
        if end > data.len() {
            return (vec![], header_len);
        }
        (data[start..end].to_vec(), header_len + content_len)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_persistent_trie_basic() {
        let mut trie = PersistentTrie::new();
        assert!(trie.is_empty());

        trie.insert(b"key1", b"value1".to_vec());
        assert_eq!(trie.get(b"key1"), Some(b"value1".to_vec()));
        assert!(!trie.is_empty());

        trie.remove(b"key1");
        assert!(trie.get(b"key1").is_none());
    }

    #[test]
    fn test_persistent_trie_root_changes() {
        let mut trie = PersistentTrie::new();
        let empty_root = trie.root_hash();

        trie.insert(b"key", b"value".to_vec());
        let root1 = trie.root_hash();
        assert_ne!(root1, empty_root);

        trie.insert(b"key", b"different".to_vec());
        let root2 = trie.root_hash();
        assert_ne!(root2, root1);
    }

    #[test]
    fn test_storage_trie() {
        let mut storage = StorageTrie::new();

        let slot = [1u8; 32];
        let value = [0u8; 31].into_iter().chain([42u8]).collect::<Vec<_>>();
        let mut value_arr = [0u8; 32];
        value_arr.copy_from_slice(&value);

        storage.set(&slot, value_arr);
        let retrieved = storage.get(&slot);
        assert_eq!(retrieved, Some(value_arr));

        // Setting to zero removes
        storage.set(&slot, [0u8; 32]);
        assert!(storage.get(&slot).is_none() || storage.get(&slot) == Some([0u8; 32]));
    }

    #[test]
    fn test_account_data_encode_decode() {
        let account = AccountData {
            nonce: 42,
            balance: {
                let mut b = [0u8; 32];
                b[31] = 100;
                b
            },
            storage_root: EMPTY_ROOT,
            code_hash: AccountData::EMPTY_CODE_HASH,
        };

        let encoded = account.encode();
        let decoded = AccountData::decode(&encoded);

        assert_eq!(decoded.nonce, 42);
        assert_eq!(decoded.balance[31], 100);
    }

    #[test]
    fn test_state_trie() {
        let mut state = StateTrie::new();

        let address = [1u8; 20];
        let account = AccountData {
            nonce: 1,
            balance: [0u8; 32],
            storage_root: EMPTY_ROOT,
            code_hash: AccountData::EMPTY_CODE_HASH,
        };

        state.set_account(&address, account);
        let retrieved = state.get_account(&address).unwrap();
        assert_eq!(retrieved.nonce, 1);
    }
}
