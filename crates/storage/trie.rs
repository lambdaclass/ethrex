use crate::api::tables::{
    ACCOUNT_FLATKEYVALUE, ACCOUNT_TRIE_NODES, STORAGE_FLATKEYVALUE, STORAGE_TRIE_NODES,
};
use crate::api::{StorageBackend, StorageLockedView};
use crate::error::StoreError;
use crate::fkv_keys::{account_fkv_key, restore_u256, storage_fkv_key};
use crate::layering::apply_prefix;
use ethrex_common::H256;
use ethrex_common::types::AccountStateSlimCodec;
use ethrex_rlp::decode::RLPDecode;
use ethrex_rlp::encode::RLPEncode;
use ethrex_trie::{Nibbles, TrieDB, error::TrieError};
use std::sync::Arc;

/// StorageWriteBatch implementation for the TrieDB trait
/// Wraps a transaction to allow multiple trie operations on the same transaction
pub struct BackendTrieDB {
    /// Reference to the storage backend
    db: Arc<dyn StorageBackend>,
    /// Last flatkeyvalue path already generated
    last_computed_flatkeyvalue: Nibbles,
    nodes_table: &'static str,
    /// Storage trie address prefix (for storage tries)
    /// None for state tries, Some(address) for storage tries
    address_prefix: Option<H256>,
}

impl BackendTrieDB {
    /// Create a new BackendTrieDB for the account trie
    pub fn new_for_accounts(
        db: Arc<dyn StorageBackend>,
        last_written: Vec<u8>,
    ) -> Result<Self, StoreError> {
        let last_computed_flatkeyvalue = Nibbles::from_hex(last_written);
        Ok(Self {
            db,
            last_computed_flatkeyvalue,
            nodes_table: ACCOUNT_TRIE_NODES,
            address_prefix: None,
        })
    }

    /// Create a new BackendTrieDB for the storage tries
    pub fn new_for_storages(
        db: Arc<dyn StorageBackend>,
        last_written: Vec<u8>,
    ) -> Result<Self, StoreError> {
        let last_computed_flatkeyvalue = Nibbles::from_hex(last_written);
        Ok(Self {
            db,
            last_computed_flatkeyvalue,
            nodes_table: STORAGE_TRIE_NODES,
            address_prefix: None,
        })
    }

    /// Create a new BackendTrieDB for a specific storage trie
    pub fn new_for_account_storage(
        db: Arc<dyn StorageBackend>,
        address_prefix: H256,
        last_written: Vec<u8>,
    ) -> Result<Self, StoreError> {
        let last_computed_flatkeyvalue = Nibbles::from_hex(last_written);
        Ok(Self {
            db,
            last_computed_flatkeyvalue,
            nodes_table: STORAGE_TRIE_NODES,
            address_prefix: Some(address_prefix),
        })
    }

    fn make_key(&self, path: Nibbles) -> Vec<u8> {
        apply_prefix(self.address_prefix, path).into_vec()
    }
}

impl TrieDB for BackendTrieDB {
    fn flatkeyvalue_computed(&self, key: Nibbles) -> bool {
        let key = apply_prefix(self.address_prefix, key);
        self.last_computed_flatkeyvalue >= key
    }

    fn get(&self, key: Nibbles) -> Result<Option<Vec<u8>>, TrieError> {
        let prefixed_key = self.make_key(key);

        // Check if this is a leaf-length key (needs FKV lookup with binary key)
        let is_account_leaf = prefixed_key.len() == 65;
        let is_storage_leaf = prefixed_key.len() == 131;

        let tx = self.db.begin_read().map_err(|e| {
            TrieError::DbError(anyhow::anyhow!("Failed to begin read transaction: {}", e))
        })?;

        if is_account_leaf {
            // Convert nibble path to binary FKV key
            let nibbles = Nibbles::from_hex(prefixed_key);
            let account_hash = H256::from_slice(&nibbles.to_bytes());
            let fkv_key = account_fkv_key(&account_hash);

            // Read from FKV table and convert slim format back to original
            let result = tx.get(ACCOUNT_FLATKEYVALUE, &fkv_key).map_err(|e| {
                TrieError::DbError(anyhow::anyhow!("Failed to get from database: {}", e))
            })?;

            match result {
                Some(bytes) => {
                    // Decode slim format and re-encode to original format
                    let slim = AccountStateSlimCodec::decode(&bytes).map_err(|e| {
                        TrieError::DbError(anyhow::anyhow!("Failed to decode slim account: {}", e))
                    })?;
                    Ok(Some(slim.0.encode_to_vec()))
                }
                None => Ok(None),
            }
        } else if is_storage_leaf {
            // Extract address and slot hash from prefixed nibble path
            // Format: [65 nibbles addr][1 nibble separator][65 nibbles slot]
            let addr_nibbles = Nibbles::from_hex(prefixed_key[..65].to_vec());
            let slot_nibbles = Nibbles::from_hex(prefixed_key[66..].to_vec());
            let account_hash = H256::from_slice(&addr_nibbles.to_bytes());
            let slot_hash = H256::from_slice(&slot_nibbles.to_bytes());
            let fkv_key = storage_fkv_key(&account_hash, &slot_hash);

            // Read from FKV table and restore U256 format
            let result = tx.get(STORAGE_FLATKEYVALUE, &fkv_key).map_err(|e| {
                TrieError::DbError(anyhow::anyhow!("Failed to get from database: {}", e))
            })?;

            match result {
                Some(bytes) => {
                    // Restore U256 from stripped bytes and RLP-encode
                    let u256 = restore_u256(&bytes);
                    Ok(Some(u256.encode_to_vec()))
                }
                None => Ok(None),
            }
        } else {
            // Internal node - read from trie nodes table
            tx.get(self.nodes_table, prefixed_key.as_ref())
                .map_err(|e| {
                    TrieError::DbError(anyhow::anyhow!("Failed to get from database: {}", e))
                })
        }
    }

    fn put_batch(&self, key_values: Vec<(Nibbles, Vec<u8>)>) -> Result<(), TrieError> {
        let mut tx = self.db.begin_write().map_err(|e| {
            TrieError::DbError(anyhow::anyhow!("Failed to begin write transaction: {}", e))
        })?;
        for (key, value) in key_values {
            let prefixed_key = self.make_key(key);
            // Always write to trie nodes table - FKV is populated separately
            tx.put_batch(self.nodes_table, vec![(prefixed_key, value)])
                .map_err(|e| TrieError::DbError(anyhow::anyhow!("Failed to write batch: {}", e)))?;
        }
        tx.commit()
            .map_err(|e| TrieError::DbError(anyhow::anyhow!("Failed to write batch: {}", e)))
    }
}

/// Read-only version with persistent locked transaction/snapshot for batch reads
pub struct BackendTrieDBLocked {
    account_trie_tx: Box<dyn StorageLockedView>,
    storage_trie_tx: Box<dyn StorageLockedView>,
    account_fkv_tx: Box<dyn StorageLockedView>,
    storage_fkv_tx: Box<dyn StorageLockedView>,
    /// Last flatkeyvalue path already generated
    last_computed_flatkeyvalue: Nibbles,
}

impl BackendTrieDBLocked {
    pub fn new(engine: &dyn StorageBackend, last_written: Vec<u8>) -> Result<Self, StoreError> {
        let last_computed_flatkeyvalue = Nibbles::from_hex(last_written);
        let account_trie_tx = engine.begin_locked(ACCOUNT_TRIE_NODES)?;
        let storage_trie_tx = engine.begin_locked(STORAGE_TRIE_NODES)?;
        let account_fkv_tx = engine.begin_locked(ACCOUNT_FLATKEYVALUE)?;
        let storage_fkv_tx = engine.begin_locked(STORAGE_FLATKEYVALUE)?;
        Ok(Self {
            account_trie_tx,
            storage_trie_tx,
            account_fkv_tx,
            storage_fkv_tx,
            last_computed_flatkeyvalue,
        })
    }
}

impl TrieDB for BackendTrieDBLocked {
    fn flatkeyvalue_computed(&self, key: Nibbles) -> bool {
        self.last_computed_flatkeyvalue >= key
    }

    fn get(&self, key: Nibbles) -> Result<Option<Vec<u8>>, TrieError> {
        // Check if this is a leaf-length key (needs FKV lookup with binary key)
        let is_account_leaf = key.len() == 65;
        let is_storage_leaf = key.len() == 131;

        if is_account_leaf {
            // Convert nibble path to binary FKV key
            let account_hash = H256::from_slice(&key.to_bytes());
            let fkv_key = account_fkv_key(&account_hash);

            // Read from FKV table and convert slim format back to original
            let result = self.account_fkv_tx.get(&fkv_key).map_err(|e| {
                TrieError::DbError(anyhow::anyhow!("Failed to get from database: {}", e))
            })?;

            match result {
                Some(bytes) => {
                    // Decode slim format and re-encode to original format
                    let slim = AccountStateSlimCodec::decode(&bytes).map_err(|e| {
                        TrieError::DbError(anyhow::anyhow!("Failed to decode slim account: {}", e))
                    })?;
                    Ok(Some(slim.0.encode_to_vec()))
                }
                None => Ok(None),
            }
        } else if is_storage_leaf {
            // Extract address and slot hash from prefixed nibble path
            // Format: [65 nibbles addr][1 nibble separator][65 nibbles slot]
            let key_bytes = key.as_ref();
            let addr_nibbles = Nibbles::from_hex(key_bytes[..65].to_vec());
            let slot_nibbles = Nibbles::from_hex(key_bytes[66..].to_vec());
            let account_hash = H256::from_slice(&addr_nibbles.to_bytes());
            let slot_hash = H256::from_slice(&slot_nibbles.to_bytes());
            let fkv_key = storage_fkv_key(&account_hash, &slot_hash);

            // Read from FKV table and restore U256 format
            let result = self.storage_fkv_tx.get(&fkv_key).map_err(|e| {
                TrieError::DbError(anyhow::anyhow!("Failed to get from database: {}", e))
            })?;

            match result {
                Some(bytes) => {
                    // Restore U256 from stripped bytes and RLP-encode
                    let u256 = restore_u256(&bytes);
                    Ok(Some(u256.encode_to_vec()))
                }
                None => Ok(None),
            }
        } else {
            // Internal node - read from trie nodes table
            let is_account = key.len() <= 65;
            let tx = if is_account {
                &*self.account_trie_tx
            } else {
                &*self.storage_trie_tx
            };
            tx.get(key.as_ref())
                .map_err(|e| TrieError::DbError(anyhow::anyhow!("Failed to get from database: {}", e)))
        }
    }

    fn put_batch(&self, _key_values: Vec<(Nibbles, Vec<u8>)>) -> Result<(), TrieError> {
        // Read-only locked storage, should not be used for puts
        Err(TrieError::DbError(anyhow::anyhow!("trie is read-only")))
    }
}
