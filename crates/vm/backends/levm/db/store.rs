use bytes::Bytes;
use ethrex_common::constants::EMPTY_KECCACK_HASH;
use ethrex_common::types::{AccountState, BlockHash, BlockNumber, ChainConfig};
use ethrex_common::{Address, H256, U256};
use ethrex_levm::db::Database as LevmDatabase;
use ethrex_levm::errors::DatabaseError;
use ethrex_storage::Store;
use std::cmp::Ordering;
use std::collections::HashMap;
use tracing::instrument;

#[derive(Clone)]
pub struct StoreVmDatabase {
    pub store: Store,
    pub block_hash: BlockHash,
    // Used to store known block hashes
    // We use this when executing blocks in batches, as we will only add the blocks at the end
    // And may need to access hashes of blocks previously executed in the batch
    pub block_hash_cache: HashMap<BlockNumber, BlockHash>,
}

impl StoreVmDatabase {
    pub fn new(store: Store, block_hash: BlockHash) -> Self {
        StoreVmDatabase {
            store,
            block_hash,
            block_hash_cache: HashMap::new(),
        }
    }

    pub fn new_with_block_hash_cache(
        store: Store,
        block_hash: BlockHash,
        block_hash_cache: HashMap<BlockNumber, BlockHash>,
    ) -> Self {
        StoreVmDatabase {
            store,
            block_hash,
            block_hash_cache,
        }
    }
}

impl LevmDatabase for StoreVmDatabase {
    #[instrument(level = "trace", name = "Account read", skip_all)]
    fn get_account_state(&self, address: Address) -> Result<AccountState, DatabaseError> {
        self.store
            .get_account_state_by_hash(self.block_hash, address)
            .map(|opt| opt.unwrap_or_default())
            .map_err(|e| DatabaseError::Custom(e.to_string()))
    }

    #[instrument(level = "trace", name = "Storage read", skip_all)]
    fn get_storage_value(&self, address: Address, key: H256) -> Result<U256, DatabaseError> {
        self.store
            .get_storage_at_hash(self.block_hash, address, key)
            .map(|opt| opt.unwrap_or_default())
            .map_err(|e| DatabaseError::Custom(e.to_string()))
    }

    #[instrument(level = "trace", name = "Block hash read", skip_all)]
    fn get_block_hash(&self, block_number: u64) -> Result<H256, DatabaseError> {
        // Check if we have it cached
        if let Some(block_hash) = self.block_hash_cache.get(&block_number) {
            return Ok(*block_hash);
        }
        // First check if our block is canonical, if it is then it's ancestor will also be canonical and we can look it up directly
        if self
            .store
            .is_canonical_sync(self.block_hash)
            .map_err(|err| DatabaseError::Custom(err.to_string()))?
        {
            if let Some(hash) = self
                .store
                .get_canonical_block_hash_sync(block_number)
                .map_err(|err| DatabaseError::Custom(err.to_string()))?
            {
                return Ok(hash);
            }
        // If our block is not canonical then we must look for the target in our block's ancestors
        } else {
            for ancestor_res in self.store.ancestors(self.block_hash) {
                let (hash, ancestor) =
                    ancestor_res.map_err(|e| DatabaseError::Custom(e.to_string()))?;
                match ancestor.number.cmp(&block_number) {
                    Ordering::Greater => continue,
                    Ordering::Equal => return Ok(hash),
                    Ordering::Less => {
                        return Err(DatabaseError::Custom(format!(
                            "Block number requested {block_number} is higher than the current block number {}",
                            ancestor.number
                        )));
                    }
                }
            }
        }
        // Block not found
        Err(DatabaseError::Custom(format!(
            "Block hash not found for block number {block_number}"
        )))
    }

    fn get_chain_config(&self) -> Result<ChainConfig, DatabaseError> {
        self.store
            .get_chain_config()
            .map_err(|e| DatabaseError::Custom(e.to_string()))
    }

    #[instrument(level = "trace", name = "Account code read", skip_all)]
    fn get_account_code(&self, code_hash: H256) -> Result<Bytes, DatabaseError> {
        if code_hash == *EMPTY_KECCACK_HASH {
            return Ok(Bytes::new());
        }
        match self.store.get_account_code(code_hash) {
            Ok(Some(code)) => Ok(code),
            Ok(None) => Err(DatabaseError::Custom(format!(
                "Code not found for hash: {code_hash:?}",
            ))),
            Err(e) => Err(DatabaseError::Custom(e.to_string())),
        }
    }
}
