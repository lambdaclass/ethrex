//! World state abstraction for Ethereum accounts and storage.

use ethereum_types::{H256, U256};

/// An Ethereum account.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Account {
    /// Account nonce.
    pub nonce: u64,
    /// Account balance.
    pub balance: U256,
    /// Code hash (keccak256 of code, or empty hash if no code).
    pub code_hash: H256,
    /// Storage root hash (computed from storage trie).
    pub storage_root: H256,
}

impl Account {
    /// Creates a new empty account.
    pub fn new() -> Self {
        Self {
            nonce: 0,
            balance: U256::zero(),
            code_hash: H256::zero(),
            storage_root: H256::zero(),
        }
    }

    /// Creates an account with the given balance.
    pub fn with_balance(balance: U256) -> Self {
        Self {
            balance,
            ..Default::default()
        }
    }

    /// Returns true if this is an empty account.
    pub fn is_empty(&self) -> bool {
        self.nonce == 0 && self.balance.is_zero() && self.code_hash == H256::zero()
    }

    /// Encodes the account for storage.
    ///
    /// Format: [nonce (8)] [balance (32)] [code_hash (32)] [storage_root (32)]
    pub fn encode(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(104);
        buf.extend_from_slice(&self.nonce.to_le_bytes());

        // Use big-endian for balance (standard Ethereum encoding)
        let balance_bytes = self.balance.to_big_endian();
        buf.extend_from_slice(&balance_bytes);

        buf.extend_from_slice(self.code_hash.as_bytes());
        buf.extend_from_slice(self.storage_root.as_bytes());
        buf
    }

    /// Decodes an account from bytes.
    pub fn decode(data: &[u8]) -> Option<Self> {
        if data.len() < 104 {
            return None;
        }

        let nonce = u64::from_le_bytes(data[0..8].try_into().ok()?);
        let balance = U256::from_big_endian(&data[8..40]);
        let code_hash = H256::from_slice(&data[40..72]);
        let storage_root = H256::from_slice(&data[72..104]);

        Some(Self {
            nonce,
            balance,
            code_hash,
            storage_root,
        })
    }
}

/// Read-only access to world state.
pub trait ReadOnlyWorldState {
    /// Gets an account by address.
    fn get_account(&self, address: &H256) -> Option<Account>;

    /// Gets a storage value.
    fn get_storage(&self, address: &H256, key: &H256) -> Option<U256>;

    /// Checks if an account exists.
    fn account_exists(&self, address: &H256) -> bool {
        self.get_account(address).is_some()
    }

    /// Gets the account balance.
    fn get_balance(&self, address: &H256) -> U256 {
        self.get_account(address)
            .map(|a| a.balance)
            .unwrap_or_default()
    }

    /// Gets the account nonce.
    fn get_nonce(&self, address: &H256) -> u64 {
        self.get_account(address)
            .map(|a| a.nonce)
            .unwrap_or_default()
    }
}

/// Mutable world state access.
pub trait WorldState: ReadOnlyWorldState {
    /// Sets an account.
    fn set_account(&mut self, address: H256, account: Account);

    /// Sets a storage value.
    fn set_storage(&mut self, address: H256, key: H256, value: U256);

    /// Deletes an account.
    fn delete_account(&mut self, address: &H256);

    /// Increments the nonce for an account.
    fn increment_nonce(&mut self, address: &H256) {
        if let Some(mut account) = self.get_account(address) {
            account.nonce += 1;
            self.set_account(*address, account);
        }
    }

    /// Adds to an account's balance.
    fn add_balance(&mut self, address: &H256, amount: U256) {
        let mut account = self.get_account(address).unwrap_or_default();
        account.balance = account.balance.saturating_add(amount);
        self.set_account(*address, account);
    }

    /// Subtracts from an account's balance.
    fn sub_balance(&mut self, address: &H256, amount: U256) -> bool {
        if let Some(mut account) = self.get_account(address) {
            if account.balance >= amount {
                account.balance = account.balance - amount;
                self.set_account(*address, account);
                return true;
            }
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_account_encode_decode() {
        let account = Account {
            nonce: 42,
            balance: U256::from(1000),
            code_hash: H256::repeat_byte(0xAB),
            storage_root: H256::repeat_byte(0xCD),
        };

        let encoded = account.encode();
        let decoded = Account::decode(&encoded).unwrap();

        assert_eq!(decoded, account);
    }

    #[test]
    fn test_empty_account() {
        let account = Account::new();
        assert!(account.is_empty());

        let account = Account::with_balance(U256::from(1));
        assert!(!account.is_empty());
    }
}
