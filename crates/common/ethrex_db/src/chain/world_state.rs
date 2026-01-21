//! World state abstraction for Ethereum accounts and storage.

use primitive_types::{H256, U256};

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

    /// Encodes the account for storage using RLP encoding.
    ///
    /// This MUST match the legacy AccountState RLP encoding byte-for-byte
    /// to ensure state root hashes match between ethrex_db and legacy trie systems.
    ///
    /// Format: RLP([nonce, balance, storage_root, code_hash])
    pub fn encode(&self) -> Vec<u8> {
        use ethereum_types::{H256 as EthH256, U256 as EthU256};
        use ethrex_rlp::structs::Encoder;

        // Convert primitive_types to ethereum_types for RLP encoding
        let balance_eth = EthU256::from_big_endian(&self.balance.to_big_endian());
        let storage_root_eth = EthH256::from(self.storage_root.0);
        let code_hash_eth = EthH256::from(self.code_hash.0);

        // Encode using same RLP encoder as AccountState
        let mut buf = Vec::with_capacity(128);
        Encoder::new(&mut buf)
            .encode_field(&self.nonce)
            .encode_field(&balance_eth)
            .encode_field(&storage_root_eth)
            .encode_field(&code_hash_eth)
            .finish();
        buf
    }

    /// Decodes an account from RLP-encoded bytes.
    ///
    /// This MUST match the legacy AccountState RLP decoding to ensure
    /// compatibility between ethrex_db and legacy trie systems.
    pub fn decode(data: &[u8]) -> Option<Self> {
        use ethereum_types::{H256 as EthH256, U256 as EthU256};
        use ethrex_rlp::structs::Decoder;

        // Decode RLP list: [nonce, balance, storage_root, code_hash]
        let decoder = Decoder::new(data).ok()?;

        // Decode fields (each decode_field returns (value, updated_decoder))
        let (nonce, decoder): (u64, _) = decoder.decode_field("nonce").ok()?;
        let (balance_eth, decoder): (EthU256, _) = decoder.decode_field("balance").ok()?;
        let (storage_root_eth, decoder): (EthH256, _) =
            decoder.decode_field("storage_root").ok()?;
        let (code_hash_eth, _): (EthH256, _) = decoder.decode_field("code_hash").ok()?;

        // Convert ethereum_types back to primitive_types
        let balance = U256::from_big_endian(&balance_eth.to_big_endian());
        let storage_root = H256(storage_root_eth.0);
        let code_hash = H256(code_hash_eth.0);

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
