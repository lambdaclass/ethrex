use ethrex_common::{
    Address, H256, U256,
    constants::EMPTY_KECCACK_HASH,
    types::{
        AccountState, BlockHash, BlockHeader, BlockNumber, ChainConfig, Code, Transaction, TxKind,
    },
};
use ethrex_storage::Store;
use ethrex_vm::{EvmError, VmDatabase};
use std::{
    cmp::Ordering,
    collections::{HashMap, HashSet},
    str::FromStr,
    sync::OnceLock,
};
use tracing::instrument;

#[derive(Clone)]
pub struct StoreVmDatabase {
    pub store: Store,
    pub block_hash: BlockHash,
    // Used to store known block hashes
    // We use this when executing blocks in batches, as we will only add the blocks at the end
    // And may need to access hashes of blocks previously executed in the batch
    pub block_hash_cache: HashMap<BlockNumber, BlockHash>,
    pub state_root: H256,

    pub account_cache: HashMap<Address, Option<AccountState>>,
    pub storage_cache: HashMap<(Address, H256), Option<U256>>,
}

static COMMON_SLOTS: OnceLock<Vec<H256>> = OnceLock::new();

fn h256_str(str: &str) -> Result<H256, EvmError> {
    H256::from_str(str).map_err(|e| EvmError::Custom(format!("decode error: {e}")))
}

impl StoreVmDatabase {
    pub fn new(store: Store, block_header: BlockHeader) -> Self {
        StoreVmDatabase {
            store,
            block_hash: block_header.hash(),
            block_hash_cache: HashMap::new(),
            state_root: block_header.state_root,
            account_cache: Default::default(),
            storage_cache: Default::default(),
        }
    }

    pub fn new_with_block_hash_cache(
        store: Store,
        block_header: BlockHeader,
        block_hash_cache: HashMap<BlockNumber, BlockHash>,
    ) -> Self {
        StoreVmDatabase {
            store,
            block_hash: block_header.hash(),
            block_hash_cache,
            state_root: block_header.state_root,
            account_cache: Default::default(),
            storage_cache: Default::default(),
        }
    }

    pub fn warm(&mut self, txns: &Vec<(&Transaction, Address)>) -> Result<(), EvmError> {
        let common_slot = match COMMON_SLOTS.get() {
            Some(val) => val,
            None => {
                let mut slots = vec![
                    // eip1967.proxy.implementation
                    h256_str("75b20eef8615de99c108b05f0dbda081c91897128caa336d75dffb97c4132b4d")?,
                    // eip1967.proxy.admin
                    h256_str("b53127684a568b3173ae13b9f8a6016e243e63b6e8ee1178d6a717850b5d6103")?,
                ];
                for i in 0..20 {
                    slots.push(H256::from_slice(&U256::from(i).to_big_endian()));
                }
                COMMON_SLOTS.get_or_init(|| slots)
            }
        };

        let mut to_fetch: HashMap<Address, HashSet<H256>> = Default::default();
        for (tx, sender) in txns {
            to_fetch.entry(*sender).or_default();
            match tx.to() {
                TxKind::Call(to) => {
                    to_fetch.entry(to).or_default();
                }
                TxKind::Create => {}
            }
            for (addr, keys) in tx.access_list() {
                to_fetch
                    .entry(*addr)
                    .or_insert_with(|| HashSet::from_iter(common_slot.clone()))
                    .extend(keys);
            }
        }
        let (account_cache, storage_cache) = self
            .store
            .fetch_bulk(self.state_root, to_fetch)
            .map_err(|e| EvmError::DB(e.to_string()))?;
        self.account_cache = account_cache;
        self.storage_cache = storage_cache;
        Ok(())
    }
}

impl VmDatabase for StoreVmDatabase {
    #[instrument(level = "trace", name = "Account read", skip_all)]
    fn get_account_state(&self, address: Address) -> Result<Option<AccountState>, EvmError> {
        if let Some(state) = self.account_cache.get(&address) {
            return Ok(state.clone());
        }
        self.store
            .get_account_state_by_root(self.state_root, address)
            .map_err(|e| EvmError::DB(e.to_string()))
    }

    #[instrument(level = "trace", name = "Storage read", skip_all)]
    fn get_storage_slot(&self, address: Address, key: H256) -> Result<Option<U256>, EvmError> {
        if let Some(value) = self.storage_cache.get(&(address, key)) {
            return Ok(value.clone());
        }
        self.store
            .get_storage_at_root(self.state_root, address, key)
            .map_err(|e| EvmError::DB(e.to_string()))
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
        Ok(self.store.get_chain_config())
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
