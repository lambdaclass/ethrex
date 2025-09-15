use bytes::Bytes;
use ethrex_common::{
    Address, H256, U256,
    constants::EMPTY_KECCACK_HASH,
    types::{AccountInfo, BlockHash, BlockNumber, ChainConfig},
};
use ethrex_storage::Store;
use ethrex_vm::{EvmError, VmDatabase};
use std::{cmp::Ordering, collections::HashMap};
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

impl VmDatabase for StoreVmDatabase {
    #[instrument(level = "trace", name = "Account read", skip_all)]
    fn get_account_info(&self, address: Address) -> Result<Option<AccountInfo>, EvmError> {
        let result = self
            .store
            .get_account_info_by_hash(self.block_hash, address)
            .map_err(|e| EvmError::DB(e.to_string()));

        #[cfg(feature = "replay")]
        if result.is_err() {
            return Ok(None);
        };

        result
    }

    #[instrument(level = "trace", name = "Storage read", skip_all)]
    fn get_storage_slot(&self, address: Address, key: H256) -> Result<Option<U256>, EvmError> {
        let result = self
            .store
            .get_storage_at_hash(self.block_hash, address, key)
            .map_err(|e| EvmError::DB(e.to_string()));

        #[cfg(feature = "replay")]
        if result.is_err() {
            return Ok(None);
        };

        result
    }

    #[instrument(level = "trace", name = "Block hash read", skip_all)]
    fn get_block_hash(&self, block_number: u64) -> Result<H256, EvmError> {
        // Check if we have it cached
        if let Some(block_hash) = self.block_hash_cache.get(&block_number) {
            return Ok(*block_hash);
        }
        // First check if our block is canonical, if it is then it's ancestor will also be canonical and we can look it up directly
        if self
            .store
            .is_canonical_sync(self.block_hash)
            .map_err(|err| EvmError::DB(err.to_string()))?
        {
            if let Some(hash) = self
                .store
                .get_canonical_block_hash_sync(block_number)
                .map_err(|err| EvmError::DB(err.to_string()))?
            {
                return Ok(hash);
            }
        // If our block is not canonical then we must look for the target in our block's ancestors
        } else {
            for ancestor_res in self.store.ancestors(self.block_hash) {
                let (hash, ancestor) = ancestor_res.map_err(|e| EvmError::DB(e.to_string()))?;
                match ancestor.number.cmp(&block_number) {
                    Ordering::Greater => continue,
                    Ordering::Equal => return Ok(hash),
                    Ordering::Less => {
                        return Err(EvmError::DB(format!(
                            "Block number requested {block_number} is higher than the current block number {}",
                            ancestor.number
                        )));
                    }
                }
            }
        }
        // Block not found
        Err(EvmError::DB(format!(
            "Block hash not found for block number {block_number}"
        )))
    }

    fn get_chain_config(&self) -> Result<ChainConfig, EvmError> {
        self.store
            .get_chain_config()
            .map_err(|e| EvmError::DB(e.to_string()))
    }

    #[instrument(level = "trace", name = "Account code read", skip_all)]
    fn get_account_code(&self, code_hash: H256) -> Result<Bytes, EvmError> {
        if code_hash == *EMPTY_KECCACK_HASH {
            return Ok(Bytes::new());
        }

        let result = self.store.get_account_code(code_hash);

        #[cfg(feature = "replay")]
        if result.is_err() || matches!(result, Ok(None)) {
            return Ok(Bytes::new());
        }

        match result {
            Ok(Some(code)) => Ok(code),
            Ok(None) => Err(EvmError::DB(format!(
                "Code not found for hash: {code_hash:?}",
            ))),
            Err(e) => Err(EvmError::DB(e.to_string())),
        }
    }
}
