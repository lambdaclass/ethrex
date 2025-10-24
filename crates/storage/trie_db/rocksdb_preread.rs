use ethrex_common::{H256, types::AccountUpdate};
use ethrex_trie::{Nibbles, TrieDB, error::TrieError};
use rayon::iter::{IntoParallelIterator, IntoParallelRefIterator, ParallelIterator};
use rocksdb::{DBWithThreadMode, MultiThreaded};
use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex},
};

use crate::{
    hash_address, hash_key,
    store_db::rocksdb::{CF_FLATKEYVALUE, CF_MISC_VALUES, CF_TRIE_NODES},
    trie_db::layering::TrieLayerCache,
};

/// RocksDB implementation for the TrieDB trait, with pre-fetching of data
pub struct RocksDBPreRead {
    /// RocksDB database
    db: Arc<DBWithThreadMode<MultiThreaded>>,
    /// Pre-read data
    cache: HashMap<Vec<u8>, Vec<u8>>,
    last_computed_flatkeyvalue: Vec<u8>,
    tlc: Arc<Mutex<Arc<TrieLayerCache>>>,
    state_root: H256,
}

pub struct RocksDBPreReadTrieDB {
    inner: Arc<RocksDBPreRead>,
    prefix: Option<Nibbles>,
}

// TODO: tune
const PREFETCH_DEPTH_ACCOUNT: usize = 10;
const PREFETCH_DEPTH_STORAGE: usize = 4;
const PREFETCH_DEPTH_STORAGE_BIG: usize = 7;
const PREFETCH_DEPTH_BIG_COUNT: usize = 2;

fn insert_prefixes(
    tree: &mut HashSet<Vec<u8>>,
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
        tlc: Arc<Mutex<Arc<TrieLayerCache>>>,
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

        let mut fkv_reads: Vec<_> = updates
            .par_iter()
            .map(|update| {
                let addr = hash_address(&update.address);
                let addr_nib = Nibbles::from_bytes(&addr);
                addr_nib.as_ref().to_vec()
            })
            .collect();
        let mut reads: Vec<_> = updates
            .par_iter()
            .flat_map_iter(|update| {
                let mut prefixes = HashSet::new();
                let addr = hash_address(&update.address);
                let addr_nib = Nibbles::from_bytes(&addr);
                let size_heuristic = if update.added_storage.len() > PREFETCH_DEPTH_BIG_COUNT {
                    PREFETCH_DEPTH_STORAGE_BIG
                } else {
                    PREFETCH_DEPTH_STORAGE
                };
                for storage_key in update.added_storage.keys() {
                    let key_hash = hash_key(storage_key);
                    let key_nib = Nibbles::from_bytes(&key_hash);
                    insert_prefixes(
                        &mut prefixes,
                        key_nib,
                        Some(addr_nib.clone()),
                        size_heuristic,
                    );
                }
                insert_prefixes(&mut prefixes, addr_nib, None, PREFETCH_DEPTH_ACCOUNT);
                prefixes.into_iter()
            })
            .collect();
        fkv_reads.sort();
        reads.sort();

        let account_results = db.batched_multi_get_cf(&cf_nodes, &reads, true);
        let fkv_results = db.batched_multi_get_cf(&cf_flatkeyvalue, &fkv_reads, true);
        let mut results = Vec::with_capacity(account_results.len());
        for (result, key) in account_results.into_iter().zip(reads) {
            if let Some(value) = result
                .map_err(|e| TrieError::DbError(anyhow::anyhow!("RocksDB get error: {}", e)))?
            {
                results.push((key, value.to_vec()));
            }
        }
        for (result, key) in fkv_results.into_iter().zip(fkv_reads) {
            if let Some(value) = result
                .map_err(|e| TrieError::DbError(anyhow::anyhow!("RocksDB get error: {}", e)))?
            {
                results.push((key, value.to_vec()));
            } else {
                results.push((key, vec![]));
            }
        }
        let tlc_lock = tlc.lock().map_err(|_| TrieError::LockError)?;
        let cache: HashMap<_, _> = results
            .into_par_iter()
            .map(|(key, value)| {
                if let Some(value) = tlc_lock.get(state_root, &key) {
                    (key, value)
                } else {
                    (key, value)
                }
            })
            .collect();
        drop(tlc_lock);

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
            prefix: Some(Nibbles::from_bytes(account.as_bytes()).append_new(17)),
        }
    }
}

impl TrieDB for RocksDBPreReadTrieDB {
    fn flatkeyvalue_computed(&self, key: Nibbles) -> bool {
        self.inner.last_computed_flatkeyvalue.as_slice() >= key.as_ref()
    }
    fn get(&self, key: Nibbles) -> Result<Option<Vec<u8>>, TrieError> {
        let key = match &self.prefix {
            Some(prefix) => prefix.concat(&key),
            None => key,
        };
        if let Some(value) = self.inner.cache.get(key.as_ref()) {
            return Ok(Some(value.clone()));
        }
        let tlc = self.inner.tlc.lock().map_err(|_| TrieError::LockError)?;
        if let Some(value) = tlc.get(self.inner.state_root, key.as_ref()) {
            return Ok(Some(value));
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
        self.inner
            .db
            .get_cf(&cf, key.as_ref())
            .map_err(|e| TrieError::DbError(anyhow::anyhow!("RocksDB get error: {}", e)))
    }

    fn put_batch(&self, _key_values: Vec<(Nibbles, Vec<u8>)>) -> Result<(), TrieError> {
        Err(TrieError::DbError(anyhow::anyhow!("PreRead is read-only")))
    }
}
