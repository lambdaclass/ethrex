//! Block abstraction for managing state changes.

use std::collections::HashMap;
use ethereum_types::{H256, U256};

use super::world_state::{Account, ReadOnlyWorldState, WorldState};

/// Unique identifier for a block.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct BlockId {
    /// Block number.
    pub number: u64,
    /// Block hash.
    pub hash: H256,
}

impl BlockId {
    /// Creates a new block ID.
    pub fn new(number: u64, hash: H256) -> Self {
        Self { number, hash }
    }
}

/// A block representing uncommitted state changes.
///
/// Blocks form a tree structure where each block has a parent.
/// Multiple blocks can share the same parent (for parallel block creation).
pub struct Block {
    /// Block identifier.
    pub id: BlockId,
    /// Parent block hash.
    pub parent_hash: H256,
    /// Account changes in this block.
    accounts: HashMap<H256, Option<Account>>,
    /// Storage changes in this block (address -> slot -> value).
    storage: HashMap<H256, HashMap<H256, U256>>,
    /// Reference to parent block for lookups.
    parent: Option<Box<Block>>,
}

impl Block {
    /// Creates a new block.
    pub fn new(number: u64, hash: H256, parent_hash: H256) -> Self {
        Self {
            id: BlockId::new(number, hash),
            parent_hash,
            accounts: HashMap::new(),
            storage: HashMap::new(),
            parent: None,
        }
    }

    /// Creates a new block with a parent reference.
    pub fn with_parent(number: u64, hash: H256, parent: Block) -> Self {
        let parent_hash = parent.id.hash;
        Self {
            id: BlockId::new(number, hash),
            parent_hash,
            accounts: HashMap::new(),
            storage: HashMap::new(),
            parent: Some(Box::new(parent)),
        }
    }

    /// Returns the block number.
    pub fn number(&self) -> u64 {
        self.id.number
    }

    /// Returns the block hash.
    pub fn hash(&self) -> H256 {
        self.id.hash
    }

    /// Returns an iterator over all account changes.
    pub fn account_changes(&self) -> impl Iterator<Item = (&H256, &Option<Account>)> {
        self.accounts.iter()
    }

    /// Returns an iterator over all storage changes.
    pub fn storage_changes(&self) -> impl Iterator<Item = (&H256, &HashMap<H256, U256>)> {
        self.storage.iter()
    }

    /// Applies this block's changes to another world state.
    pub fn apply_to<W: WorldState>(&self, state: &mut W) {
        // Apply account changes
        for (address, account) in &self.accounts {
            match account {
                Some(acc) => state.set_account(*address, acc.clone()),
                None => state.delete_account(address),
            }
        }

        // Apply storage changes
        for (address, slots) in &self.storage {
            for (key, value) in slots {
                state.set_storage(*address, *key, *value);
            }
        }
    }

    /// Returns the number of accounts modified.
    pub fn account_count(&self) -> usize {
        self.accounts.len()
    }

    /// Returns the number of storage slots modified.
    pub fn storage_slot_count(&self) -> usize {
        self.storage.values().map(|s| s.len()).sum()
    }
}

impl ReadOnlyWorldState for Block {
    fn get_account(&self, address: &H256) -> Option<Account> {
        // Check local changes first
        if let Some(account) = self.accounts.get(address) {
            return account.clone();
        }

        // Check parent
        if let Some(parent) = &self.parent {
            return parent.get_account(address);
        }

        None
    }

    fn get_storage(&self, address: &H256, key: &H256) -> Option<U256> {
        // Check local storage
        if let Some(slots) = self.storage.get(address) {
            if let Some(value) = slots.get(key) {
                return Some(*value);
            }
        }

        // Check parent
        if let Some(parent) = &self.parent {
            return parent.get_storage(address, key);
        }

        None
    }
}

impl WorldState for Block {
    fn set_account(&mut self, address: H256, account: Account) {
        self.accounts.insert(address, Some(account));
    }

    fn set_storage(&mut self, address: H256, key: H256, value: U256) {
        self.storage
            .entry(address)
            .or_insert_with(HashMap::new)
            .insert(key, value);
    }

    fn delete_account(&mut self, address: &H256) {
        self.accounts.insert(*address, None);
        self.storage.remove(address);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_block_creation() {
        let block = Block::new(1, H256::repeat_byte(0x01), H256::zero());
        assert_eq!(block.number(), 1);
        assert_eq!(block.hash(), H256::repeat_byte(0x01));
    }

    #[test]
    fn test_account_changes() {
        let mut block = Block::new(1, H256::repeat_byte(0x01), H256::zero());

        let address = H256::repeat_byte(0xAB);
        let account = Account::with_balance(U256::from(100));

        block.set_account(address, account.clone());
        assert_eq!(block.get_account(&address), Some(account));
    }

    #[test]
    fn test_storage_changes() {
        let mut block = Block::new(1, H256::repeat_byte(0x01), H256::zero());

        let address = H256::repeat_byte(0xAB);
        let key = H256::repeat_byte(0xCD);
        let value = U256::from(42);

        block.set_storage(address, key, value);
        assert_eq!(block.get_storage(&address, &key), Some(value));
    }

    #[test]
    fn test_parent_lookup() {
        let mut parent = Block::new(1, H256::repeat_byte(0x01), H256::zero());
        let address = H256::repeat_byte(0xAB);
        parent.set_account(address, Account::with_balance(U256::from(100)));

        let child = Block::with_parent(2, H256::repeat_byte(0x02), parent);

        // Should find account in parent
        assert_eq!(
            child.get_account(&address),
            Some(Account::with_balance(U256::from(100)))
        );
    }

    #[test]
    fn test_child_overrides_parent() {
        let mut parent = Block::new(1, H256::repeat_byte(0x01), H256::zero());
        let address = H256::repeat_byte(0xAB);
        parent.set_account(address, Account::with_balance(U256::from(100)));

        let mut child = Block::with_parent(2, H256::repeat_byte(0x02), parent);
        child.set_account(address, Account::with_balance(U256::from(200)));

        // Should find overridden account
        assert_eq!(
            child.get_account(&address),
            Some(Account::with_balance(U256::from(200)))
        );
    }
}
