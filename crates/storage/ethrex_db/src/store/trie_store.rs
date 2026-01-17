//! TrieStore - Persistent trie storage using page-based storage.
//!
//! This module bridges the in-memory MerkleTrie with the PagedDb storage,
//! providing a persistent key-value store optimized for trie operations.

use std::collections::HashMap;

use crate::data::{NibblePath, SlottedArray, PAGE_SIZE};
use crate::merkle::{keccak256, MerkleTrie, EMPTY_ROOT};
use crate::store::{DbAddress, PageType, LeafPage, DataPage, BatchContext, PagedDb, DbError};

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
        let arr = SlottedArray::from_bytes(Self::page_data_to_array(page_data));
        let mut trie = MerkleTrie::new();

        // Reconstruct trie from stored entries using the new iterator
        for (path, value) in arr.iter() {
            // Convert NibblePath back to bytes
            let key = Self::nibble_path_to_bytes(&path);
            trie.insert(&key, value);
        }

        Self {
            trie,
            pending: HashMap::new(),
        }
    }

    /// Converts a NibblePath to its original byte representation.
    fn nibble_path_to_bytes(path: &NibblePath) -> Vec<u8> {
        let mut bytes = Vec::with_capacity((path.len() + 1) / 2);
        let mut i = 0;
        while i < path.len() {
            let high = path.get(i);
            let low = if i + 1 < path.len() { path.get(i + 1) } else { 0 };
            bytes.push((high << 4) | low);
            i += 2;
        }
        bytes
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

    /// Returns the number of entries.
    pub fn len(&self) -> usize {
        self.trie.len()
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

    /// Writes the trie to multiple pages if needed.
    ///
    /// Returns a list of (key_prefix, page_data) pairs for entries that didn't fit.
    pub fn to_pages(&self) -> Vec<Vec<u8>> {
        let mut pages = Vec::new();
        let mut current_arr = SlottedArray::new();

        for (key, value) in self.trie.iter() {
            let path = NibblePath::from_bytes(key);
            if !current_arr.try_insert(&path, value) {
                // Page is full, start a new one
                pages.push(current_arr.as_bytes().to_vec());
                current_arr = SlottedArray::new();
                current_arr.try_insert(&path, value);
            }
        }

        // Don't forget the last page
        if current_arr.live_count() > 0 {
            pages.push(current_arr.as_bytes().to_vec());
        }

        pages
    }

    fn page_data_to_array(data: &[u8]) -> [u8; PAGE_SIZE] {
        let mut arr = [0u8; PAGE_SIZE];
        let len = data.len().min(PAGE_SIZE);
        arr[..len].copy_from_slice(&data[..len]);
        arr
    }

    /// Returns an iterator over all key-value pairs.
    pub fn iter(&self) -> impl Iterator<Item = (&[u8], &[u8])> {
        self.trie.iter()
    }

    /// Clears pending changes after commit.
    pub fn clear_pending(&mut self) {
        self.pending.clear();
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

// ============================================================================
// PagedStateTrie - Full integration with PagedDb
// ============================================================================

/// A state trie that persists to PagedDb.
///
/// Uses DataPage for fanout navigation and LeafPage for actual data storage.
/// Supports full load/save cycles with the database.
pub struct PagedStateTrie {
    /// The in-memory state trie
    state: StateTrie,
    /// Address of the root page in PagedDb (if loaded from db)
    root_addr: Option<DbAddress>,
}

impl PagedStateTrie {
    /// Creates a new empty paged state trie.
    pub fn new() -> Self {
        Self {
            state: StateTrie::new(),
            root_addr: None,
        }
    }

    /// Loads a state trie from PagedDb.
    ///
    /// Reads the state from the database starting at the given root address.
    pub fn load(db: &PagedDb, root_addr: DbAddress) -> Result<Self, DbError> {
        if root_addr.is_null() {
            return Ok(Self::new());
        }

        let mut state = StateTrie::new();

        // Load the root page
        let root_page = db.get_page(root_addr)?;
        let page_type = root_page.header().get_page_type();

        match page_type {
            Some(PageType::Leaf) => {
                // Simple case: all data in a single leaf page
                let leaf = LeafPage::wrap(root_page);
                let arr = SlottedArray::from_bytes(Self::payload_to_array(leaf.data()));

                for (path, value) in arr.iter() {
                    let key = PersistentTrie::nibble_path_to_bytes(&path);
                    // Decode as account data
                    let _account = AccountData::decode(&value);
                    // Convert key back to address (it's keccak256 of address, but we store the hash)
                    let mut addr_hash = [0u8; 32];
                    let copy_len = key.len().min(32);
                    addr_hash[..copy_len].copy_from_slice(&key[..copy_len]);
                    // Insert directly into the underlying trie
                    state.trie.insert(&addr_hash, value);
                }
            }
            Some(PageType::Data) => {
                // Complex case: fanout structure
                Self::load_from_data_page(db, &root_page, &mut state)?;
            }
            Some(PageType::StateRoot) => {
                // StateRoot page contains references to account and storage tries
                Self::load_from_state_root_page(db, &root_page, &mut state)?;
            }
            _ => {
                // Unknown or empty page type
            }
        }

        Ok(Self {
            state,
            root_addr: Some(root_addr),
        })
    }

    fn load_from_data_page(db: &PagedDb, page: &crate::store::Page, state: &mut StateTrie) -> Result<(), DbError> {
        let data_page = DataPage::wrap(page.clone());

        // Iterate through all buckets
        for i in 0..256 {
            let child_addr = data_page.get_bucket(i);
            if !child_addr.is_null() {
                let child_page = db.get_page(child_addr)?;
                let child_type = child_page.header().get_page_type();

                match child_type {
                    Some(PageType::Leaf) => {
                        let leaf = LeafPage::wrap(child_page);
                        let arr = SlottedArray::from_bytes(Self::payload_to_array(leaf.data()));

                        for (path, value) in arr.iter() {
                            let key = PersistentTrie::nibble_path_to_bytes(&path);
                            let mut addr_hash = [0u8; 32];
                            let copy_len = key.len().min(32);
                            addr_hash[..copy_len].copy_from_slice(&key[..copy_len]);
                            state.trie.insert(&addr_hash, value);
                        }
                    }
                    Some(PageType::Data) => {
                        // Recursive fanout
                        Self::load_from_data_page(db, &child_page, state)?;
                    }
                    _ => {}
                }
            }
        }

        Ok(())
    }

    fn load_from_state_root_page(db: &PagedDb, page: &crate::store::Page, state: &mut StateTrie) -> Result<(), DbError> {
        // StateRoot page layout:
        // - First 4 bytes: address of accounts trie root
        // - Remaining: list of (address_hash, storage_trie_addr) pairs
        let data = page.payload();

        if data.len() < 4 {
            return Ok(());
        }

        // Read accounts trie address
        let accounts_addr = DbAddress::read(data);
        if !accounts_addr.is_null() {
            let accounts_page = db.get_page(accounts_addr)?;
            let page_type = accounts_page.header().get_page_type();

            match page_type {
                Some(PageType::Leaf) => {
                    let leaf = LeafPage::wrap(accounts_page);
                    let arr = SlottedArray::from_bytes(Self::payload_to_array(leaf.data()));

                    for (path, value) in arr.iter() {
                        let key = PersistentTrie::nibble_path_to_bytes(&path);
                        let mut addr_hash = [0u8; 32];
                        let copy_len = key.len().min(32);
                        addr_hash[..copy_len].copy_from_slice(&key[..copy_len]);
                        state.trie.insert(&addr_hash, value);
                    }
                }
                Some(PageType::Data) => {
                    Self::load_from_data_page(db, &accounts_page, state)?;
                }
                _ => {}
            }
        }

        // TODO: Load storage tries from the page as well

        Ok(())
    }

    fn payload_to_array(data: &[u8]) -> [u8; PAGE_SIZE] {
        let mut arr = [0u8; PAGE_SIZE];
        let len = data.len().min(PAGE_SIZE);
        arr[..len].copy_from_slice(&data[..len]);
        arr
    }

    /// Saves the state trie to PagedDb.
    ///
    /// Returns the root address that can be stored in the RootPage.
    pub fn save(&mut self, batch: &mut BatchContext) -> Result<DbAddress, DbError> {
        // Compute root hash first (this updates storage roots in accounts)
        let _root_hash = self.state.root_hash();

        // Collect all account entries (clone to avoid borrow issues)
        let entries: Vec<(Vec<u8>, Vec<u8>)> = self.state.trie.iter()
            .map(|(k, v)| (k.to_vec(), v.to_vec()))
            .collect();

        if entries.is_empty() {
            return Ok(DbAddress::NULL);
        }

        // Try to fit everything in a single leaf page
        let mut arr = SlottedArray::new();
        let mut all_fit = true;

        for (key, value) in &entries {
            let path = NibblePath::from_bytes(key);
            if !arr.try_insert(&path, value) {
                all_fit = false;
                break;
            }
        }

        if all_fit {
            // Everything fits in one page
            let (addr, _page) = batch.allocate_page(PageType::Leaf, 0)?;
            let arr_bytes = arr.as_bytes();

            // Get a mutable copy and write
            let mut leaf_page = batch.get_writable_copy(addr)?;
            let payload = leaf_page.payload_mut();
            let copy_len = payload.len().min(arr_bytes.len());
            payload[..copy_len].copy_from_slice(&arr_bytes[..copy_len]);
            batch.mark_dirty(addr, leaf_page);

            self.root_addr = Some(addr);
            return Ok(addr);
        }

        // Need to use fanout structure
        Self::save_with_fanout_static(&mut self.root_addr, batch, &entries)
    }

    fn save_with_fanout_static(root_addr: &mut Option<DbAddress>, batch: &mut BatchContext, entries: &[(Vec<u8>, Vec<u8>)]) -> Result<DbAddress, DbError> {
        // Group entries by first byte (256 buckets)
        let mut buckets: Vec<Vec<(&[u8], &[u8])>> = vec![Vec::new(); 256];

        for (key, value) in entries {
            if !key.is_empty() {
                let bucket_idx = key[0] as usize;
                buckets[bucket_idx].push((key.as_slice(), value.as_slice()));
            }
        }

        // Allocate root data page
        let (addr, _) = batch.allocate_page(PageType::Data, 0)?;
        let root_page = batch.get_writable_copy(addr)?;
        let mut data_page = DataPage::wrap(root_page);

        // Process each bucket
        for (i, bucket_entries) in buckets.iter().enumerate() {
            if bucket_entries.is_empty() {
                continue;
            }

            // Try to fit bucket in a leaf page
            let mut arr = SlottedArray::new();
            let mut all_fit = true;

            for (key, value) in bucket_entries {
                // Skip first byte since it's encoded in the bucket index
                let remaining = if key.len() > 1 { &key[1..] } else { &[] };
                let path = NibblePath::from_bytes(remaining);
                if !arr.try_insert(&path, value) {
                    all_fit = false;
                    break;
                }
            }

            if all_fit {
                // Create leaf page for this bucket
                let (leaf_addr, _) = batch.allocate_page(PageType::Leaf, 1)?;
                let mut leaf_page = batch.get_writable_copy(leaf_addr)?;
                let arr_bytes = arr.as_bytes();
                let payload = leaf_page.payload_mut();
                let copy_len = payload.len().min(arr_bytes.len());
                payload[..copy_len].copy_from_slice(&arr_bytes[..copy_len]);
                batch.mark_dirty(leaf_addr, leaf_page);

                data_page.set_bucket(i, leaf_addr);
            } else {
                // Need recursive fanout (for very large buckets)
                // For now, just split across multiple leaf pages
                let first_leaf = Self::save_bucket_multi_page_static(batch, bucket_entries)?;
                data_page.set_bucket(i, first_leaf);
            }
        }

        batch.mark_dirty(addr, data_page.into_page());
        *root_addr = Some(addr);
        Ok(addr)
    }

    fn save_bucket_multi_page_static(batch: &mut BatchContext, entries: &[(&[u8], &[u8])]) -> Result<DbAddress, DbError> {
        // Simple approach: just create multiple leaf pages and link them
        // In a production implementation, this would use a proper tree structure
        let mut first_addr = DbAddress::NULL;
        let mut arr = SlottedArray::new();

        for (key, value) in entries {
            let remaining = if key.len() > 1 { &key[1..] } else { &[] };
            let path = NibblePath::from_bytes(remaining);

            if !arr.try_insert(&path, value) {
                // Save current page and start new one
                let (addr, _) = batch.allocate_page(PageType::Leaf, 1)?;
                let mut page = batch.get_writable_copy(addr)?;
                let arr_bytes = arr.as_bytes();
                let payload = page.payload_mut();
                let copy_len = payload.len().min(arr_bytes.len());
                payload[..copy_len].copy_from_slice(&arr_bytes[..copy_len]);
                batch.mark_dirty(addr, page);

                if first_addr.is_null() {
                    first_addr = addr;
                }

                arr = SlottedArray::new();
                arr.try_insert(&path, value);
            }
        }

        // Save last page if non-empty
        if arr.live_count() > 0 {
            let (addr, _) = batch.allocate_page(PageType::Leaf, 1)?;
            let mut page = batch.get_writable_copy(addr)?;
            let arr_bytes = arr.as_bytes();
            let payload = page.payload_mut();
            let copy_len = payload.len().min(arr_bytes.len());
            payload[..copy_len].copy_from_slice(&arr_bytes[..copy_len]);
            batch.mark_dirty(addr, page);

            if first_addr.is_null() {
                first_addr = addr;
            }
        }

        Ok(first_addr)
    }

    /// Gets an account by address.
    pub fn get_account(&self, address: &[u8; 20]) -> Option<AccountData> {
        self.state.get_account(address)
    }

    /// Sets an account.
    pub fn set_account(&mut self, address: &[u8; 20], account: AccountData) {
        self.state.set_account(address, account);
    }

    /// Gets the storage trie for an account.
    pub fn storage_trie(&mut self, address: &[u8; 20]) -> &mut StorageTrie {
        self.state.storage_trie(address)
    }

    /// Computes the state root hash.
    pub fn root_hash(&mut self) -> [u8; 32] {
        self.state.root_hash()
    }

    /// Returns the root address in PagedDb.
    pub fn root_addr(&self) -> Option<DbAddress> {
        self.root_addr
    }

    /// Returns the number of accounts.
    pub fn account_count(&self) -> usize {
        self.state.trie.len()
    }
}

impl Default for PagedStateTrie {
    fn default() -> Self {
        Self::new()
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

    #[test]
    fn test_paged_state_trie_basic() {
        use crate::store::{PagedDb, CommitOptions};

        let mut db = PagedDb::in_memory(1000).unwrap();
        let mut trie = PagedStateTrie::new();

        // Add some accounts
        let address1 = [1u8; 20];
        let account1 = AccountData {
            nonce: 10,
            balance: {
                let mut b = [0u8; 32];
                b[31] = 100;
                b
            },
            storage_root: EMPTY_ROOT,
            code_hash: AccountData::EMPTY_CODE_HASH,
        };
        trie.set_account(&address1, account1);

        let address2 = [2u8; 20];
        let account2 = AccountData {
            nonce: 20,
            balance: {
                let mut b = [0u8; 32];
                b[31] = 200;
                b
            },
            storage_root: EMPTY_ROOT,
            code_hash: AccountData::EMPTY_CODE_HASH,
        };
        trie.set_account(&address2, account2);

        // Save to database
        let mut batch = db.begin_batch();
        let root_addr = trie.save(&mut batch).unwrap();
        batch.set_state_root(root_addr);
        batch.commit(CommitOptions::DangerNoFlush).unwrap();

        assert!(!root_addr.is_null());
        assert_eq!(trie.account_count(), 2);
    }

    #[test]
    fn test_paged_state_trie_save_load() {
        use crate::store::{PagedDb, CommitOptions};

        let mut db = PagedDb::in_memory(1000).unwrap();
        let root_addr;

        // Create and save a trie
        {
            let mut trie = PagedStateTrie::new();

            for i in 0..5u8 {
                let mut address = [0u8; 20];
                address[0] = i;
                let account = AccountData {
                    nonce: i as u64,
                    balance: {
                        let mut b = [0u8; 32];
                        b[31] = i * 10;
                        b
                    },
                    storage_root: EMPTY_ROOT,
                    code_hash: AccountData::EMPTY_CODE_HASH,
                };
                trie.set_account(&address, account);
            }

            let _root_hash_original = trie.root_hash();

            let mut batch = db.begin_batch();
            root_addr = trie.save(&mut batch).unwrap();
            batch.set_state_root(root_addr);
            batch.commit(CommitOptions::DangerNoFlush).unwrap();
        }

        // Load the trie back
        {
            let loaded = PagedStateTrie::load(&db, root_addr).unwrap();
            assert_eq!(loaded.account_count(), 5);

            // Verify accounts
            for i in 0..5u8 {
                let mut address = [0u8; 20];
                address[0] = i;
                let account = loaded.get_account(&address);
                assert!(account.is_some(), "Account {} not found", i);
                assert_eq!(account.unwrap().nonce, i as u64);
            }
        }
    }

    #[test]
    fn test_paged_state_trie_root_hash() {
        let mut trie = PagedStateTrie::new();

        let empty_root = trie.root_hash();
        assert_eq!(empty_root, EMPTY_ROOT);

        let address = [42u8; 20];
        let account = AccountData {
            nonce: 1,
            balance: [0u8; 32],
            storage_root: EMPTY_ROOT,
            code_hash: AccountData::EMPTY_CODE_HASH,
        };
        trie.set_account(&address, account);

        let new_root = trie.root_hash();
        assert_ne!(new_root, EMPTY_ROOT);
    }

    #[test]
    fn test_persistent_trie_page_roundtrip() {
        let mut trie = PersistentTrie::new();

        // Insert some data
        trie.insert(b"key1", b"value1".to_vec());
        trie.insert(b"key2", b"value2".to_vec());
        trie.insert(b"another_key", b"another_value".to_vec());

        let _original_root = trie.root_hash();

        // Convert to page data
        let page_data = trie.to_page_data();

        // Load from page data
        let loaded = PersistentTrie::load_from_page(&page_data);

        // Verify data
        assert_eq!(loaded.get(b"key1"), Some(b"value1".to_vec()));
        assert_eq!(loaded.get(b"key2"), Some(b"value2".to_vec()));
        assert_eq!(loaded.get(b"another_key"), Some(b"another_value".to_vec()));
    }
}
