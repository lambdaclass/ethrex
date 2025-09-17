use super::StorageBackend;
use crate::v2::{
    backend::StorageError,
    schema::{DBTable, SchemaRegistry},
};
use ethereum_types::H256;
use ethrex_trie::{NodeHash, TrieDB, TrieError};
use std::sync::Arc;

/// TrieDB adapter that wraps a StorageBackend to provide trie functionality
pub struct StorageBackendTrieDB {
    schema: SchemaRegistry,
    namespace: DBTable,
    account_prefix: Option<H256>,
}

impl StorageBackendTrieDB {
    pub fn new_state_trie(backend: Arc<dyn StorageBackend>) -> Result<Self, StorageError> {
        let schema = SchemaRegistry::new(backend)?;

        Ok(Self {
            schema,
            namespace: DBTable::StateTrieNodes,
            account_prefix: None,
        })
    }

    pub fn new_storage_trie(
        backend: Arc<dyn StorageBackend>,
        account_prefix: H256,
    ) -> Result<Self, StorageError> {
        let schema = SchemaRegistry::new(backend)?;

        Ok(Self {
            schema,
            namespace: DBTable::StorageTrieNodes,
            account_prefix: Some(account_prefix),
        })
    }

    fn encode_key(&self, node_hash: NodeHash) -> Vec<u8> {
        match &self.account_prefix {
            Some(prefix) => {
                let mut key = prefix.as_bytes().to_vec();
                key.extend_from_slice(node_hash.as_ref());
                key
            }
            None => {
                // For state tries, use node hash directly
                node_hash.as_ref().to_vec()
            }
        }
    }
}

impl TrieDB for StorageBackendTrieDB {
    fn get(&self, key: NodeHash) -> Result<Option<Vec<u8>>, TrieError> {
        let encoded_key = self.encode_key(key);
        self.schema
            .get_sync(self.namespace, encoded_key)
            .map_err(|_| TrieError::InconsistentTree)
    }

    fn put_batch(&self, key_values: Vec<(NodeHash, Vec<u8>)>) -> Result<(), TrieError> {
        for (key, value) in key_values {
            let encoded_key = self.encode_key(key);
            self.schema
                .put_sync(self.namespace, encoded_key, value)
                .map_err(|_| TrieError::InconsistentTree)?;
        }

        Ok(())
    }
}

pub struct StorageBackendLockedTrieDB {
    schema: SchemaRegistry,
    namespace: DBTable,
    account_prefix: Option<H256>,
}

impl StorageBackendLockedTrieDB {
    pub fn new_state_trie(backend: Arc<dyn StorageBackend>) -> Result<Self, StorageError> {
        let schema = SchemaRegistry::new(backend)?;

        Ok(Self {
            schema,
            namespace: DBTable::StateTrieNodes,
            account_prefix: None,
        })
    }

    pub fn new_storage_trie(
        backend: Arc<dyn StorageBackend>,
        account_prefix: H256,
    ) -> Result<Self, StorageError> {
        let schema = SchemaRegistry::new(backend)?;

        Ok(Self {
            schema,
            namespace: DBTable::StorageTrieNodes,
            account_prefix: Some(account_prefix),
        })
    }

    fn encode_key(&self, node_hash: NodeHash) -> Vec<u8> {
        match &self.account_prefix {
            Some(prefix) => {
                let mut key = prefix.as_bytes().to_vec();
                key.extend_from_slice(node_hash.as_ref());
                key
            }
            None => {
                // For state tries, use node hash directly
                node_hash.as_ref().to_vec()
            }
        }
    }
}

impl TrieDB for StorageBackendLockedTrieDB {
    fn get(&self, key: NodeHash) -> Result<Option<Vec<u8>>, TrieError> {
        let encoded_key = self.encode_key(key);
        self.schema
            .get_sync(self.namespace, encoded_key)
            .map_err(|_| TrieError::InconsistentTree)
    }

    fn put_batch(&self, _key_values: Vec<(NodeHash, Vec<u8>)>) -> Result<(), TrieError> {
        unimplemented!("LockedTrie is read-only")
    }
}
