use bytes::Bytes;
use ethrex_common::{
    Address, H256, U256,
    constants::EMPTY_KECCACK_HASH,
    types::{AccountState, Block, BlockHeader, ChainConfig, TxKind},
};
use ethrex_rlp::decode::RLPDecode;
use ethrex_trie::{Nibbles, TrieError};
use ethrex_vm::{EvmError, VmDatabase};
use rayon::iter::{IntoParallelRefIterator, ParallelIterator};
use std::{collections::HashMap, sync::OnceLock};
use tracing::instrument;

use crate::{
    api::StoreEngine,
    apply_prefix,
    error::StoreError,
    hash_address, hash_key,
    store_db::rocksdb::{CF_CHAIN_DATA, CF_FLATKEYVALUE, CF_TRIE_NODES, Store},
    utils::ChainDataIndex,
};

#[derive(Clone)]
pub struct RocksDBVM {
    store: Store,
    header: BlockHeader,
    chain_config_cache: OnceLock<ChainConfig>,
    cache: HashMap<Address, Option<AccountState>>,
}

impl RocksDBVM {
    pub fn new(store: Store, header: BlockHeader, block: &Block) -> Result<Self, StoreError> {
        let mut vm = Self {
            store,
            header,
            chain_config_cache: OnceLock::new(),
            cache: HashMap::new(),
        };
        vm.warm(block)?;
        Ok(vm)
    }
    #[instrument(level = "trace", name = "Account prefetch", skip_all)]
    fn warm(&mut self, block: &Block) -> Result<(), StoreError> {
        let tlc_lock = self
            .store
            .trie_cache
            .write()
            .map_err(|_| StoreError::LockError)?;
        let accounts: Vec<_> = block
            .body
            .transactions
            .par_iter()
            .flat_map(|tx| {
                let mut addresses: Vec<_> = tx
                    .access_list()
                    .into_iter()
                    .map(|(k, _)| k.clone())
                    .collect();
                if let TxKind::Call(to) = tx.to() {
                    addresses.push(to);
                }
                if let Ok(sender) = tx.sender() {
                    addresses.push(sender);
                }
                addresses
            })
            .map(|address| {
                let key = nibbles_for_account(address);
                (address, tlc_lock.get(self.header.state_root, key))
            })
            .collect();
        drop(tlc_lock);
        let mut fkv_reads = Vec::with_capacity(accounts.len());
        for (account, value) in accounts {
            match value {
                Some(value) => {
                    let decoded = if value.is_empty() {
                        None
                    } else {
                        Some(AccountState::decode(&value)?)
                    };
                    self.cache.insert(account, decoded);
                }
                None => fkv_reads.push(account),
            }
        }
        fkv_reads.sort();
        let fkv_reads_nibs: Vec<_> = fkv_reads
            .iter()
            .map(|address| nibbles_for_account(*address))
            .collect();
        let cf: std::sync::Arc<rocksdb::BoundColumnFamily<'_>> = self
            .store
            .db
            .cf_handle(CF_FLATKEYVALUE)
            .ok_or_else(|| TrieError::DbError(anyhow::anyhow!("Column family not found")))?;
        let fkv_results = self
            .store
            .db
            .batched_multi_get_cf(&cf, &fkv_reads_nibs, true);
        for (result, account) in fkv_results.into_iter().zip(fkv_reads) {
            if let Some(value) = result? {
                let decoded = AccountState::decode(&value)?;
                self.cache.insert(account, Some(decoded));
            } else {
                self.cache.insert(account, None);
            }
        }
        Ok(())
    }
    fn get_fkv<T: RLPDecode>(&self, key: Nibbles) -> Result<Option<T>, TrieError> {
        let tlc = self
            .store
            .trie_cache
            .read()
            .map_err(|_| TrieError::LockError)?;
        if let Some(value) = tlc.get(self.header.state_root, key.clone()) {
            if value.is_empty() {
                return Ok(None);
            }
            return Ok(Some(T::decode(&value)?));
        }
        drop(tlc);
        let cf = self
            .store
            .db
            .cf_handle(CF_FLATKEYVALUE)
            .ok_or_else(|| TrieError::DbError(anyhow::anyhow!("Column family not found")))?;
        if let Some(value) = self
            .store
            .db
            .get_pinned_cf(&cf, key.as_ref())
            .map_err(|e| TrieError::DbError(anyhow::anyhow!("RocksDB get error: {}", e)))?
        {
            return Ok(Some(T::decode(&value)?));
        }
        Ok(None)
    }
}

fn nibbles_for_account(address: Address) -> Nibbles {
    let hash = hash_address(&address);
    Nibbles::from_bytes(&hash)
}

fn nibbles_for_slot(address: Address, key: H256) -> Nibbles {
    let hash_address = hash_address(&address);
    let hash_key = hash_key(&key);
    apply_prefix(
        Some(H256::from_slice(&hash_address)),
        Nibbles::from_bytes(&hash_key),
    )
}

impl VmDatabase for RocksDBVM {
    #[instrument(level = "trace", name = "Account read", skip_all)]
    fn get_account_state(&self, address: Address) -> Result<Option<AccountState>, EvmError> {
        if let Some(value) = self.cache.get(&address) {
            return Ok(value.clone());
        }
        self.get_fkv(nibbles_for_account(address))
            .map_err(|e| EvmError::DB(e.to_string()))
    }

    #[instrument(level = "trace", name = "Storage read", skip_all)]
    fn get_storage_slot(&self, address: Address, key: H256) -> Result<Option<U256>, EvmError> {
        self.get_fkv(nibbles_for_slot(address, key))
            .map_err(|e| EvmError::DB(e.to_string()))
    }

    #[instrument(level = "trace", name = "Block hash read", skip_all)]
    fn get_block_hash(&self, block_number: u64) -> Result<H256, EvmError> {
        if let Some(hash) = self
            .store
            .get_canonical_block_hash_sync(block_number)
            .map_err(|e| EvmError::DB(e.to_string()))?
        {
            return Ok(hash);
        }
        let mut current = self.header.hash();
        loop {
            let Some(header) = self
                .store
                .get_block_header_by_hash(current)
                .map_err(|e| EvmError::DB(e.to_string()))?
            else {
                break;
            };
            if block_number > header.number {
                return Err(EvmError::DB(format!("Block hash in the future")));
            }
            if header.number == block_number {
                return Ok(current);
            }
            current = header.parent_hash;
        }
        // Block not found
        Err(EvmError::DB(format!(
            "Block hash not found for block number {block_number}"
        )))
    }

    fn get_chain_config(&self) -> Result<ChainConfig, EvmError> {
        if let Some(chain_config) = self.chain_config_cache.get() {
            return Ok(*chain_config);
        }
        let key = Store::chain_data_key(ChainDataIndex::ChainConfig);
        let cf = self
            .store
            .db
            .cf_handle(CF_CHAIN_DATA)
            .ok_or_else(|| EvmError::DB("Column family CF_CHAIN_DATA not found".to_string()))?;
        if let Some(value) = self
            .store
            .db
            .get_pinned_cf(&cf, key)
            .map_err(|e| EvmError::DB(e.to_string()))?
        {
            let chain_config: ChainConfig = serde_json::from_slice(&value)
                .map_err(|e| EvmError::DB(format!("chain data invalid: {e}")))?;
            let _ = self.chain_config_cache.set(chain_config.clone());
            return Ok(chain_config);
        }
        Err(EvmError::DB("missing chain config".to_string()))
    }

    #[instrument(level = "trace", name = "Account code read", skip_all)]
    fn get_account_code(&self, code_hash: H256) -> Result<Bytes, EvmError> {
        if code_hash == *EMPTY_KECCACK_HASH {
            return Ok(Bytes::new());
        }
        match self.store.get_account_code(code_hash) {
            Ok(Some(code)) => Ok(code),
            Ok(None) => Err(EvmError::DB(format!(
                "Code not found for hash: {code_hash:?}",
            ))),
            Err(e) => Err(EvmError::DB(e.to_string())),
        }
    }
}
