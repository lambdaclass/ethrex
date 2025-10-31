use ethrex_common::{
    Address, H256, U256,
    constants::EMPTY_KECCACK_HASH,
    types::{AccountState, BlockHeader, ChainConfig, Code},
};
use ethrex_rlp::decode::RLPDecode;
use ethrex_trie::{Nibbles, TrieError};
use ethrex_vm::{EvmError, VmDatabase};
use std::sync::{Arc, OnceLock};
use tracing::instrument;

use crate::{
    api::StoreEngine,
    apply_prefix,
    error::StoreError,
    hash_address, hash_key,
    store_db::rocksdb::{CF_CHAIN_DATA, CF_FLATKEYVALUE, CF_MISC_VALUES, Store},
    trie_db::layering::TrieLayerCache,
    utils::ChainDataIndex,
};

#[derive(Clone)]
pub struct RocksDBVM {
    store: Store,
    header: BlockHeader,
    chain_config_cache: OnceLock<ChainConfig>,
    last_computed_flatkeyvalue: Nibbles,
    trie_cache: Arc<TrieLayerCache>,
}

impl RocksDBVM {
    pub fn new(store: Store, header: BlockHeader) -> Result<Self, StoreError> {
        let cf_misc = store
            .db
            .cf_handle(CF_MISC_VALUES)
            .ok_or_else(|| TrieError::DbError(anyhow::anyhow!("Column family not found")))?;
        let last_computed_flatkeyvalue = store
            .db
            .get_cf(&cf_misc, "last_written")
            .map_err(|e| TrieError::DbError(anyhow::anyhow!("Error reading last_written: {e}")))?
            .map(|v| Nibbles::from_hex(v.to_vec()))
            .unwrap_or_default();
        drop(cf_misc);

        let trie_cache = store
            .trie_cache
            .lock()
            .map_err(|_| TrieError::LockError)?
            .clone();
        Ok(Self {
            store,
            header,
            chain_config_cache: OnceLock::new(),
            last_computed_flatkeyvalue,
            trie_cache,
        })
    }
    fn get_fkv<T: RLPDecode>(&self, key: Nibbles) -> Result<Option<T>, TrieError> {
        if let Some(value) = self.trie_cache.get(self.header.state_root, key.clone()) {
            if value.is_empty() {
                return Ok(None);
            }
            return Ok(Some(T::decode(&value)?));
        }
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
        let hashed_account = nibbles_for_account(address);
        // deoptimized path
        if hashed_account > self.last_computed_flatkeyvalue {
            let trie = self
                .store
                .open_state_trie(self.header.state_root)
                .map_err(|e| EvmError::DB(e.to_string()))?;
            let Some(accountrlp) = trie
                .get(&hashed_account.to_bytes())
                .map_err(|e| EvmError::DB(e.to_string()))?
            else {
                return Ok(None);
            };
            return Ok(Some(
                AccountState::decode(&accountrlp).map_err(|e| EvmError::DB(e.to_string()))?,
            ));
        }
        self.get_fkv(hashed_account)
            .map_err(|e| EvmError::DB(e.to_string()))
    }

    #[instrument(level = "trace", name = "Storage read", skip_all)]
    fn get_storage_slot(&self, address: Address, key: H256) -> Result<Option<U256>, EvmError> {
        let hashed_storage = nibbles_for_slot(address, key);
        // deoptimized path
        if hashed_storage > self.last_computed_flatkeyvalue {
            let Some(account) = self.get_account_state(address)? else {
                return Ok(None);
            };
            let trie = self
                .store
                .open_storage_trie(
                    H256::from_slice(&hash_address(&address)),
                    account.storage_root,
                    self.header.state_root,
                )
                .map_err(|e| EvmError::DB(e.to_string()))?;
            let Some(storagerlp) = trie
                .get(&hash_key(&key))
                .map_err(|e| EvmError::DB(e.to_string()))?
            else {
                return Ok(None);
            };
            return Ok(Some(
                U256::decode(&storagerlp).map_err(|e| EvmError::DB(e.to_string()))?,
            ));
        }
        self.get_fkv(hashed_storage)
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
                return Err(EvmError::DB("Block hash in the future".to_string()));
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
            let _ = self.chain_config_cache.set(chain_config);
            return Ok(chain_config);
        }
        Err(EvmError::DB("missing chain config".to_string()))
    }

    #[instrument(level = "trace", name = "Account code read", skip_all)]
    fn get_account_code(&self, code_hash: H256) -> Result<Code, EvmError> {
        if code_hash == *EMPTY_KECCACK_HASH {
            return Ok(Code::default());
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
