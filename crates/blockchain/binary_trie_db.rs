use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, RwLock};

use ethrex_binary_trie::state::BinaryTrieState;
use ethrex_common::{
    Address, H256,
    constants::EMPTY_KECCACK_HASH,
    types::{AccountState, BlockNumber, ChainConfig, Code, CodeMetadata},
};
use ethrex_vm::{EvmError, VmDatabase};

/// VmDatabase adapter backed by a BinaryTrieState.
///
/// Unlike StoreVmDatabase, this does not use Store/RocksDB.
/// The trie state is shared via Arc<RwLock> — during execution
/// the VM only reads; writes happen after via apply_account_update.
#[derive(Clone)]
pub struct BinaryTrieVmDb {
    state: Arc<RwLock<BinaryTrieState>>,
    chain_config: ChainConfig,
    block_hashes: Arc<Mutex<BTreeMap<BlockNumber, H256>>>,
}

impl BinaryTrieVmDb {
    pub fn new(state: Arc<RwLock<BinaryTrieState>>, chain_config: ChainConfig) -> Self {
        Self {
            state,
            chain_config,
            block_hashes: Arc::new(Mutex::new(BTreeMap::new())),
        }
    }

    /// Register a block hash for the BLOCKHASH opcode.
    pub fn add_block_hash(&self, number: BlockNumber, hash: H256) {
        self.block_hashes
            .lock()
            .expect("block_hashes mutex poisoned")
            .insert(number, hash);
    }

    /// Register multiple block hashes at once.
    pub fn add_block_hashes(&self, hashes: impl IntoIterator<Item = (BlockNumber, H256)>) {
        let mut cache = self
            .block_hashes
            .lock()
            .expect("block_hashes mutex poisoned");
        for (number, hash) in hashes {
            cache.insert(number, hash);
        }
    }
}

impl VmDatabase for BinaryTrieVmDb {
    fn get_account_state(&self, address: Address) -> Result<Option<AccountState>, EvmError> {
        let state = self
            .state
            .read()
            .map_err(|e| EvmError::DB(format!("lock error: {e}")))?;
        Ok(state.get_account_state(&address))
    }

    fn get_storage_slot(
        &self,
        address: Address,
        key: H256,
    ) -> Result<Option<ethrex_common::U256>, EvmError> {
        let state = self
            .state
            .read()
            .map_err(|e| EvmError::DB(format!("lock error: {e}")))?;
        Ok(state.get_storage_slot(&address, key))
    }

    fn get_block_hash(&self, block_number: u64) -> Result<H256, EvmError> {
        let cache = self
            .block_hashes
            .lock()
            .map_err(|e| EvmError::DB(format!("lock error: {e}")))?;
        cache
            .get(&block_number)
            .copied()
            .ok_or_else(|| EvmError::DB(format!("block hash not found for block {block_number}")))
    }

    fn get_chain_config(&self) -> Result<ChainConfig, EvmError> {
        Ok(self.chain_config)
    }

    fn get_account_code(&self, code_hash: H256) -> Result<Code, EvmError> {
        if code_hash == *EMPTY_KECCACK_HASH {
            return Ok(Code::default());
        }
        let state = self
            .state
            .read()
            .map_err(|e| EvmError::DB(format!("lock error: {e}")))?;
        state
            .get_account_code(&code_hash)
            .map(|bytes| Code::from_bytecode_unchecked(bytes, code_hash))
            .ok_or_else(|| EvmError::DB(format!("code not found for hash {code_hash:?}")))
    }

    fn get_code_metadata(&self, code_hash: H256) -> Result<CodeMetadata, EvmError> {
        if code_hash == *EMPTY_KECCACK_HASH {
            return Ok(CodeMetadata { length: 0 });
        }
        let state = self
            .state
            .read()
            .map_err(|e| EvmError::DB(format!("lock error: {e}")))?;
        state
            .get_account_code(&code_hash)
            .map(|bytes| CodeMetadata {
                length: bytes.len() as u64,
            })
            .ok_or_else(|| EvmError::DB(format!("code metadata not found for hash {code_hash:?}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;
    use ethrex_common::{U256, types::GenesisAccount};
    use std::collections::BTreeMap;

    fn make_address(b: u8) -> Address {
        let mut a = [0u8; 20];
        a[19] = b;
        Address::from(a)
    }

    fn test_chain_config() -> ChainConfig {
        // Minimal chain config for testing.
        ChainConfig::default()
    }

    #[test]
    fn test_vmdb_get_nonexistent_account() {
        let state = Arc::new(RwLock::new(BinaryTrieState::new()));
        let db = BinaryTrieVmDb::new(state, test_chain_config());
        let result = db.get_account_state(make_address(1)).expect("no error");
        assert!(result.is_none());
    }

    #[test]
    fn test_vmdb_get_genesis_account() {
        let mut trie_state = BinaryTrieState::new();
        let addr = make_address(0xAA);
        let mut accounts = BTreeMap::new();
        accounts.insert(
            addr,
            GenesisAccount {
                code: Bytes::new(),
                storage: BTreeMap::new(),
                balance: U256::from(1_000_000u64),
                nonce: 5,
            },
        );
        trie_state.apply_genesis(&accounts).expect("genesis failed");

        let state = Arc::new(RwLock::new(trie_state));
        let db = BinaryTrieVmDb::new(state, test_chain_config());

        let acct = db
            .get_account_state(addr)
            .expect("no error")
            .expect("account exists");
        assert_eq!(acct.balance, U256::from(1_000_000u64));
        assert_eq!(acct.nonce, 5);
    }

    #[test]
    fn test_vmdb_get_storage_slot() {
        let mut trie_state = BinaryTrieState::new();
        let addr = make_address(0xBB);

        let mut storage = BTreeMap::new();
        storage.insert(U256::from(7u64), U256::from(42u64));

        let mut accounts = BTreeMap::new();
        accounts.insert(
            addr,
            GenesisAccount {
                code: Bytes::new(),
                storage,
                balance: U256::from(100u64),
                nonce: 0,
            },
        );
        trie_state.apply_genesis(&accounts).expect("genesis failed");

        let state = Arc::new(RwLock::new(trie_state));
        let db = BinaryTrieVmDb::new(state, test_chain_config());

        let slot_key = H256(U256::from(7u64).to_big_endian());
        let val = db
            .get_storage_slot(addr, slot_key)
            .expect("no error")
            .expect("slot exists");
        assert_eq!(val, U256::from(42u64));
    }

    #[test]
    fn test_vmdb_get_account_code() {
        let mut trie_state = BinaryTrieState::new();
        let addr = make_address(0xCC);
        let bytecode = Bytes::from(vec![0x60u8, 0x00, 0x56]);
        let code_hash = ethrex_common::utils::keccak(bytecode.as_ref());

        let mut accounts = BTreeMap::new();
        accounts.insert(
            addr,
            GenesisAccount {
                code: bytecode.clone(),
                storage: BTreeMap::new(),
                balance: U256::zero(),
                nonce: 1,
            },
        );
        trie_state.apply_genesis(&accounts).expect("genesis failed");

        let state = Arc::new(RwLock::new(trie_state));
        let db = BinaryTrieVmDb::new(state, test_chain_config());

        let code = db.get_account_code(code_hash).expect("no error");
        assert_eq!(code.bytecode, bytecode);
    }

    #[test]
    fn test_vmdb_get_empty_code() {
        let state = Arc::new(RwLock::new(BinaryTrieState::new()));
        let db = BinaryTrieVmDb::new(state, test_chain_config());
        let code = db.get_account_code(*EMPTY_KECCACK_HASH).expect("no error");
        assert!(code.bytecode.is_empty());
    }

    #[test]
    fn test_vmdb_block_hash() {
        let state = Arc::new(RwLock::new(BinaryTrieState::new()));
        let db = BinaryTrieVmDb::new(state, test_chain_config());

        // No hashes registered.
        assert!(db.get_block_hash(100).is_err());

        // Register one.
        let hash = H256::from_low_u64_be(0xDEAD);
        db.add_block_hash(100, hash);
        assert_eq!(db.get_block_hash(100).expect("no error"), hash);
    }

    #[test]
    fn test_vmdb_chain_config() {
        let state = Arc::new(RwLock::new(BinaryTrieState::new()));
        let db = BinaryTrieVmDb::new(state, test_chain_config());
        let config = db.get_chain_config().expect("no error");
        assert_eq!(config, test_chain_config());
    }

    #[test]
    fn test_vmdb_code_metadata() {
        let mut trie_state = BinaryTrieState::new();
        let addr = make_address(0xDD);
        let bytecode = Bytes::from(vec![0x00u8; 100]);
        let code_hash = ethrex_common::utils::keccak(bytecode.as_ref());

        let mut accounts = BTreeMap::new();
        accounts.insert(
            addr,
            GenesisAccount {
                code: bytecode,
                storage: BTreeMap::new(),
                balance: U256::zero(),
                nonce: 1,
            },
        );
        trie_state.apply_genesis(&accounts).expect("genesis failed");

        let state = Arc::new(RwLock::new(trie_state));
        let db = BinaryTrieVmDb::new(state, test_chain_config());

        let meta = db.get_code_metadata(code_hash).expect("no error");
        assert_eq!(meta.length, 100);

        // Empty code metadata.
        let empty_meta = db.get_code_metadata(*EMPTY_KECCACK_HASH).expect("no error");
        assert_eq!(empty_meta.length, 0);
    }
}
