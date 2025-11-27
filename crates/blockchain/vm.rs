use ethrex_common::{
    Address, H256, U256,
    constants::EMPTY_KECCACK_HASH,
    types::{AccountState, BlockHash, BlockHeader, BlockNumber, ChainConfig, Code},
};
use ethrex_storage::{Store, error::StoreError};
use ethrex_vm::{EvmError, VmDatabase};
use std::{cmp::Ordering, collections::HashMap};
use tracing::instrument;

const MAX_BLOCK_HASH_LOOKUP_DEPTH: u64 = 256;

#[derive(Clone)]
pub struct StoreVmDatabase {
    pub store: Store,
    pub block_hash: BlockHash,
    // The [MAX_BLOCK_HASH_LOOKUP_DEPTH] block hashes before the current block will be pre-loaded to optimize block hash lookup (BLOCKHASH opcode)
    // This will also be loaded with the block hashes for the full block batch in the case of batch execution
    pub block_hash_cache: HashMap<BlockNumber, BlockHash>,
    pub state_root: H256,
}

fn fill_prev_block_hashes(
    block_header: &BlockHeader,
    block_hash_cache: &mut HashMap<BlockNumber, BlockHash>,
    store: Store,
) -> Result<(), StoreError> {
    let current_block = block_header.number;
    let mut current_hash = block_header.hash();
    let oldest_block = current_block.saturating_sub(MAX_BLOCK_HASH_LOOKUP_DEPTH);
    let is_canonic = store
        .get_canonical_block_hash_sync(block_header.number)?
        .is_some_and(|hash| hash == block_header.hash());
    // If the block is canonical, look up hashes directly
    if is_canonic {
        let hashes = store.get_canonical_block_hashes(oldest_block, block_header.number)?;
        current_hash = *hashes.last().unwrap_or(&current_hash);
        block_hash_cache.extend((block_header.number..current_block).zip(hashes));
    }
    // Lookup the rest of the hashes via ancestor lookup
    for ancestor_res in store.ancestors(current_hash) {
        let (hash, ancestor) = ancestor_res?;
        block_hash_cache.insert(ancestor.number, hash);
        match ancestor.number.cmp(&oldest_block) {
            Ordering::Greater => continue,
            _ => break,
        }
    }
    Ok(())
}

impl StoreVmDatabase {
    pub fn new(store: Store, block_header: BlockHeader) -> Result<Self, EvmError> {
        Self::new_with_block_hash_cache(store, block_header, HashMap::new())
    }

    pub fn new_with_block_hash_cache(
        store: Store,
        block_header: BlockHeader,
        mut block_hash_cache: HashMap<BlockNumber, BlockHash>,
    ) -> Result<Self, EvmError> {
        // Fill up block hash cache with prev [MAX_BLOCK_HASH_LOOKUP_DEPTH] block hashes
        fill_prev_block_hashes(&block_header, &mut block_hash_cache, store.clone())
            .map_err(|err| EvmError::DB(err.to_string()))?;
        Ok(StoreVmDatabase {
            store,
            block_hash: block_header.hash(),
            block_hash_cache,
            state_root: block_header.state_root,
        })
    }
}

impl VmDatabase for StoreVmDatabase {
    #[instrument(
        level = "trace",
        name = "Account read",
        skip_all,
        fields(namespace = "block_execution")
    )]
    fn get_account_state(&self, address: Address) -> Result<Option<AccountState>, EvmError> {
        self.store
            .get_account_state_by_root(self.state_root, address)
            .map_err(|e| EvmError::DB(e.to_string()))
    }

    #[instrument(
        level = "trace",
        name = "Storage read",
        skip_all,
        fields(namespace = "block_execution")
    )]
    fn get_storage_slot(&self, address: Address, key: H256) -> Result<Option<U256>, EvmError> {
        self.store
            .get_storage_at_root(self.state_root, address, key)
            .map_err(|e| EvmError::DB(e.to_string()))
    }

    #[instrument(
        level = "trace",
        name = "Block hash read",
        skip_all,
        fields(namespace = "block_execution")
    )]
    fn get_block_hash(&self, block_number: u64) -> Result<H256, EvmError> {
        // We should have already preloaded all available hashes when initializing the DB
        if let Some(block_hash) = self.block_hash_cache.get(&block_number) {
            Ok(*block_hash)
        } else {
            Err(EvmError::DB(format!(
                "Block hash not found for block number {block_number}"
            )))
        }
    }

    fn get_chain_config(&self) -> Result<ChainConfig, EvmError> {
        Ok(self.store.get_chain_config())
    }

    #[instrument(
        level = "trace",
        name = "Account code read",
        skip_all,
        fields(namespace = "block_execution")
    )]
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
