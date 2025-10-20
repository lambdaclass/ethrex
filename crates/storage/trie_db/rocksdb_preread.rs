use ethrex_common::{H256, types::AccountUpdate};
use ethrex_trie::{Nibbles, TrieDB, error::TrieError};
use rocksdb::{DBWithThreadMode, MultiThreaded};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, RwLock},
};

use crate::{
    apply_prefix, hash_address, hash_key,
    store_db::rocksdb::{CF_FLATKEYVALUE, CF_MISC_VALUES, CF_TRIE_NODES},
    trie_db::layering::TrieLayerCache,
};

/// RocksDB implementation for the TrieDB trait, with pre-fetching of data
pub struct RocksDBPreRead {
    /// RocksDB database
    db: Arc<DBWithThreadMode<MultiThreaded>>,
    /// Pre-read data
    cache: BTreeMap<Vec<u8>, Vec<u8>>,
    last_computed_flatkeyvalue: Vec<u8>,
    tlc: Arc<RwLock<TrieLayerCache>>,
    state_root: H256,
}

pub struct RocksDBPreReadTrieDB {
    inner: Arc<RocksDBPreRead>,
    prefix: Option<H256>,
}

// TODO: tune
const PREFETCH_DEPTH_ACCOUNT: usize = 6;
const PREFETCH_DEPTH_STORAGE: usize = 2;
const PREFETCH_DEPTH_STORAGE_BIG: usize = 4;
const PREFETCH_DEPTH_BIG_COUNT: usize = 5;

fn insert_prefixes(
    tree: &mut BTreeSet<Vec<u8>>,
    value: Nibbles,
    base: Option<Nibbles>,
    depth: usize,
) {
    let mut prefix = match base {
        Some(prefix) => prefix.append_new(17).as_ref().to_vec(),
        None => vec![],
    };
    tree.insert(prefix.clone());
    for nibble in &value.as_ref()[..depth] {
        prefix.push(*nibble);
        tree.insert(prefix.clone());
    }
}

impl RocksDBPreRead {
    pub fn new(
        db: Arc<DBWithThreadMode<MultiThreaded>>,
        updates: &[AccountUpdate],
        tlc: Arc<RwLock<TrieLayerCache>>,
        state_root: H256,
    ) -> Result<Self, TrieError> {
        // Verify column family exists
        let cf_nodes = db
            .cf_handle(CF_TRIE_NODES)
            .ok_or_else(|| TrieError::DbError(anyhow::anyhow!("Column family not found")))?;
        let cf_flatkeyvalue = db
            .cf_handle(CF_FLATKEYVALUE)
            .ok_or_else(|| TrieError::DbError(anyhow::anyhow!("Column family not found")))?;
        let cf_misc = db
            .cf_handle(CF_MISC_VALUES)
            .ok_or_else(|| TrieError::DbError(anyhow::anyhow!("Column family not found")))?;
        let last_computed_flatkeyvalue = db
            .get_cf(&cf_misc, "last_written")
            .map_err(|e| TrieError::DbError(anyhow::anyhow!("Error reading last_written: {e}")))?
            .map(|v| Nibbles::from_hex(v.to_vec()))
            .unwrap_or_default()
            .as_ref()
            .to_vec();

        let mut cache = BTreeMap::new();

        let mut reads = BTreeSet::new();
        let mut fkv_reads = BTreeSet::new();
        for update in updates {
            let addr = hash_address(&update.address);
            let addr_nib = Nibbles::from_bytes(&addr);
            let size_heuristic = if update.added_storage.len() > PREFETCH_DEPTH_BIG_COUNT {
                PREFETCH_DEPTH_STORAGE_BIG
            } else {
                PREFETCH_DEPTH_STORAGE
            };
            for (storage_key, _) in &update.added_storage {
                let key_hash = hash_key(storage_key);
                let key_nib = Nibbles::from_bytes(&key_hash);
                fkv_reads.insert(addr_nib.append_new(17).concat(&key_nib).as_ref().to_vec());
                insert_prefixes(&mut reads, key_nib, Some(addr_nib.clone()), size_heuristic);
            }
            fkv_reads.insert(addr_nib.as_ref().to_vec());
            insert_prefixes(&mut reads, addr_nib, None, PREFETCH_DEPTH_ACCOUNT);
        }

        let account_results = db.batched_multi_get_cf(&cf_nodes, &reads, true);
        println!("prefetched {} nodes", account_results.len());
        for (result, key) in account_results.into_iter().zip(reads) {
            if let Some(value) = result
                .map_err(|e| TrieError::DbError(anyhow::anyhow!("RocksDB get error: {}", e)))?
            {
                cache.insert(key, value.to_vec());
            }
        }

        let snapshot_results = db.batched_multi_get_cf(&cf_flatkeyvalue, &fkv_reads, true);
        for (result, key) in snapshot_results.into_iter().zip(fkv_reads) {
            if let Some(value) = result
                .map_err(|e| TrieError::DbError(anyhow::anyhow!("RocksDB get error: {}", e)))?
            {
                cache.insert(key, value.to_vec());
            }
        }
        drop(cf_misc);
        drop(cf_nodes);
        drop(cf_flatkeyvalue);

        Ok(Self {
            db,
            cache,
            last_computed_flatkeyvalue,
            tlc,
            state_root,
        })
    }

    pub fn state_trie(self: &Arc<RocksDBPreRead>) -> RocksDBPreReadTrieDB {
        RocksDBPreReadTrieDB {
            inner: self.clone(),
            prefix: None,
        }
    }

    pub fn storage_trie(self: &Arc<RocksDBPreRead>, account: H256) -> RocksDBPreReadTrieDB {
        RocksDBPreReadTrieDB {
            inner: self.clone(),
            prefix: Some(account),
        }
    }
}

impl TrieDB for RocksDBPreReadTrieDB {
    fn flatkeyvalue_computed(&self, key: Nibbles) -> bool {
        self.inner.last_computed_flatkeyvalue.as_slice() >= key.as_ref()
    }
    fn get(&self, key: Nibbles) -> Result<Option<Vec<u8>>, TrieError> {
        let key = apply_prefix(self.prefix, key);
        let tlc = self.inner.tlc.read().map_err(|_| TrieError::LockError)?;
        if let Some(value) = tlc.get(self.inner.state_root, key.clone()) {
            return Ok(Some(value));
        }
        if let Some(value) = self.inner.cache.get(key.as_ref()) {
            return Ok(Some(value.clone()));
        }

        let cf = self
            .inner
            .db
            .cf_handle(if key.is_leaf() {
                CF_FLATKEYVALUE
            } else {
                CF_TRIE_NODES
            })
            .ok_or_else(|| TrieError::DbError(anyhow::anyhow!("Column family not found")))?;
        let res = self
            .inner
            .db
            .get_cf(&cf, key.as_ref())
            .map_err(|e| TrieError::DbError(anyhow::anyhow!("RocksDB get error: {}", e)))?;
        Ok(res)
    }

    fn put_batch(&self, _key_values: Vec<(Nibbles, Vec<u8>)>) -> Result<(), TrieError> {
        Err(TrieError::DbError(anyhow::anyhow!("PreRead is read-only")))
    }
}
