use std::collections::HashMap;

use ethereum_types::H160;
use ethrex_core::{
    types::{AccountState, Block, ChainConfig},
    H256,
};
use ethrex_rlp::encode::RLPEncode;
use ethrex_storage::{hash_address, hash_key, AccountUpdate, Store};
use ethrex_trie::{NodeRLP, Trie};
use revm::{
    primitives::{
        AccountInfo as RevmAccountInfo, Address as RevmAddress, Bytecode as RevmBytecode,
        B256 as RevmB256, U256 as RevmU256,
    },
    DatabaseRef,
};
use serde::{Deserialize, Serialize};

use crate::{errors::ExecutionDBError, evm_state, execute_block, get_state_transitions};

/// In-memory EVM database for caching execution data.
///
/// This is mainly used to store the relevant state data for executing a particular block and then
/// feeding the DB into a zkVM program to prove the execution.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ExecutionDB {
    /// indexed by account address
    pub accounts: HashMap<RevmAddress, AccountState>,
    /// indexed by code hash
    pub code: HashMap<RevmB256, RevmBytecode>,
    /// indexed by account address and storage key
    pub storage: HashMap<RevmAddress, HashMap<RevmU256, RevmU256>>,
    /// indexed by block number
    pub block_hashes: HashMap<u64, RevmB256>,
    /// stored chain config
    pub chain_config: ChainConfig,
    /// encoded nodes to reconstruct a state trie, but only including relevant data (pruned).
    /// root node is stored separately from the rest.
    pub pruned_state_trie: (Option<NodeRLP>, Vec<NodeRLP>),
    /// encoded nodes to reconstruct every storage trie, but only including relevant data (pruned)
    /// root nodes are stored separately from the rest.
    pub pruned_storage_tries: HashMap<H160, (Option<NodeRLP>, Vec<NodeRLP>)>,
}

impl ExecutionDB {
    /// Creates a database and returns the ExecutionDB by executing a block,
    /// without performing any validation.
    pub fn from_exec(block: &Block, store: &Store) -> Result<Self, ExecutionDBError> {
        // TODO: perform validation to exit early
        let account_updates = Self::get_account_updates(block, store)?;
        Self::from_account_updates(account_updates, block, store)
    }

    /// Creates a database and returns the ExecutionDB from a Vec<[AccountUpdate]>,
    /// without performing any validation.
    pub fn from_account_updates(
        account_updates: Vec<AccountUpdate>,
        block: &Block,
        store: &Store,
    ) -> Result<Self, ExecutionDBError> {
        // TODO: perform validation to exit early
        let chain_config = store.get_chain_config()?;

        // Store data touched by updates and get all touched storage keys for each account
        let mut accounts = HashMap::new();
        let code = HashMap::new(); // TODO: `code` remains empty for now
        let mut storage = HashMap::new();
        let block_hashes = HashMap::new(); // TODO: `block_hashes` remains empty for now

        let mut address_storage_keys = HashMap::new();

        for account_update in account_updates.iter() {
            let address = RevmAddress::from_slice(account_update.address.as_bytes());
            let account_state = match store.get_account_state_by_hash(
                block.header.parent_hash,
                H160::from_slice(address.as_slice()),
            )? {
                Some(state) => state,
                None => continue,
            };
            accounts.insert(address, account_state);

            let account_storage = account_update
                .added_storage
                .iter()
                .map(|(key, value)| {
                    (
                        RevmU256::from_be_bytes(key.to_fixed_bytes()),
                        RevmU256::from_be_slice(&value.to_big_endian()),
                    )
                })
                .collect();
            storage.insert(address, account_storage);
            address_storage_keys.insert(
                account_update.address,
                account_update
                    .added_storage
                    .keys()
                    .cloned()
                    .collect::<Vec<_>>(),
            );
        }

        // Get pruned state and storage tries. For this we get the "state" (all relevant nodes) of every trie.
        // "Pruned" because we're only getting the nodes that make paths to the relevant
        // key-values.
        let state_trie = store.state_trie(block.header.parent_hash)?.ok_or(
            ExecutionDBError::NewMissingStateTrie(block.header.parent_hash),
        )?;

        // Get pruned state trie
        let state_paths: Vec<_> = address_storage_keys.keys().map(hash_address).collect();
        let pruned_state_trie = state_trie.get_proofs(&state_paths)?;

        // Get pruned storage tries for every account
        let mut pruned_storage_tries = HashMap::new();
        for (address, keys) in address_storage_keys {
            let storage_trie = store
                .storage_trie(block.header.parent_hash, address)?
                .ok_or(ExecutionDBError::NewMissingStorageTrie(
                    block.header.parent_hash,
                    address,
                ))?;
            let storage_paths: Vec<_> = keys.iter().map(hash_key).collect();
            let (storage_trie_root, storage_trie_nodes) =
                storage_trie.get_proofs(&storage_paths)?;
            pruned_storage_tries.insert(address, (storage_trie_root, storage_trie_nodes));
        }

        Ok(Self {
            accounts,
            code,
            storage,
            block_hashes,
            chain_config,
            pruned_state_trie,
            pruned_storage_tries,
        })
    }

    /// Gets the Vec<[AccountUpdate]>/StateTransitions obtained after executing a block.
    pub fn get_account_updates(
        block: &Block,
        store: &Store,
    ) -> Result<Vec<AccountUpdate>, ExecutionDBError> {
        // TODO: perform validation to exit early

        let mut state = evm_state(store.clone(), block.header.parent_hash);

        execute_block(block, &mut state).map_err(Box::new)?;

        let account_updates = get_state_transitions(&mut state);
        Ok(account_updates)
    }

    pub fn get_chain_config(&self) -> ChainConfig {
        self.chain_config
    }

    /// Verifies that all data in [self] is included in the stored tries, and then builds the
    /// pruned tries from the stored nodes.
    pub fn build_tries(&self) -> Result<(Trie, HashMap<H160, Trie>), ExecutionDBError> {
        let (state_trie_root, state_trie_nodes) = &self.pruned_state_trie;
        let state_trie = Trie::from_nodes(state_trie_root.as_ref(), state_trie_nodes)?;
        let mut storage_tries = HashMap::new();

        for (revm_address, account) in &self.accounts {
            let address = H160::from_slice(revm_address.as_slice());

            // check account is in state trie
            if state_trie.get(&hash_address(&address))?.is_none() {
                return Err(ExecutionDBError::MissingAccountInStateTrie(address));
            }

            let (storage_trie_root, storage_trie_nodes) =
                self.pruned_storage_tries
                    .get(&address)
                    .ok_or(ExecutionDBError::MissingStorageTrie(address))?;

            // compare account storage root with storage trie root
            let storage_trie = Trie::from_nodes(storage_trie_root.as_ref(), storage_trie_nodes)?;
            if storage_trie.hash_no_commit() != account.storage_root {
                return Err(ExecutionDBError::InvalidStorageTrieRoot(address));
            }

            // check all storage keys are in storage trie and compare values
            let storage = self
                .storage
                .get(revm_address)
                .ok_or(ExecutionDBError::StorageNotFound(*revm_address))?;
            for (key, value) in storage {
                let key = H256::from_slice(&key.to_be_bytes_vec());
                let value = H256::from_slice(&value.to_be_bytes_vec());
                let retrieved_value = storage_trie
                    .get(&hash_key(&key))?
                    .ok_or(ExecutionDBError::MissingKeyInStorageTrie(address, key))?;
                if value.encode_to_vec() != retrieved_value {
                    return Err(ExecutionDBError::InvalidStorageTrieValue(address, key));
                }
            }

            storage_tries.insert(address, storage_trie);
        }

        Ok((state_trie, storage_tries))
    }
}

impl DatabaseRef for ExecutionDB {
    /// The database error type.
    type Error = ExecutionDBError;

    /// Get basic account information.
    fn basic_ref(&self, address: RevmAddress) -> Result<Option<RevmAccountInfo>, Self::Error> {
        let Some(account_state) = self.accounts.get(&address) else {
            return Ok(None);
        };

        Ok(Some(RevmAccountInfo {
            balance: RevmU256::from_be_bytes(account_state.balance.to_big_endian()),
            nonce: account_state.nonce,
            code_hash: RevmB256::from_slice(account_state.code_hash.as_bytes()),
            code: None,
        }))
    }

    /// Get account code by its hash.
    fn code_by_hash_ref(&self, code_hash: RevmB256) -> Result<RevmBytecode, Self::Error> {
        self.code
            .get(&code_hash)
            .cloned()
            .ok_or(ExecutionDBError::CodeNotFound(code_hash))
    }

    /// Get storage value of address at index.
    fn storage_ref(&self, address: RevmAddress, index: RevmU256) -> Result<RevmU256, Self::Error> {
        self.storage
            .get(&address)
            .ok_or(ExecutionDBError::AccountNotFound(address))?
            .get(&index)
            .cloned()
            .ok_or(ExecutionDBError::StorageValueNotFound(address, index))
    }

    /// Get block hash by block number.
    fn block_hash_ref(&self, number: u64) -> Result<RevmB256, Self::Error> {
        self.block_hashes
            .get(&number)
            .cloned()
            .ok_or(ExecutionDBError::BlockHashNotFound(number))
    }
}

pub mod index_db {
    use std::{cell::RefCell, collections::HashMap};

    use ethrex_core::{types::Block, Address, U256};
    use revm::{inspectors::TracerEip3155, DatabaseCommit, DatabaseRef, Evm};
    use revm_primitives::{
        Account as RevmAccount, Address as RevmAddress, SpecId, B256 as RevmB256, U256 as RevmU256,
    };
    use thiserror::Error;

    use crate::{block_env, tx_env, EvmError};

    #[derive(Debug, Error)]
    pub enum IndexDBError {
        #[error("Runtime borrowing")]
        Borrow,
    }

    /// Dummy DB for indexing read/written state from the execution a block.
    pub struct IndexDB {
        pub written_state: HashMap<RevmAddress, Vec<RevmU256>>,
        pub read_state: HashMap<RevmAddress, Vec<RevmU256>>,
        pub block_numbers: Vec<u64>,
        pub code_hashes: Vec<RevmB256>,
    }

    /// Auxiliary structure for creating a [IndexDB]
    #[derive(Default)]
    struct AuxIndexDB {
        written_state: HashMap<RevmAddress, Vec<RevmU256>>,

        // the following fields need to be mutated whenever a reference is queried from the DB, in
        // which case the DB is immutably borrowed, so they're wrapped in RefCells.
        read_state: RefCell<HashMap<RevmAddress, Vec<RevmU256>>>,
        block_numbers: RefCell<Vec<u64>>,
        code_hashes: RefCell<Vec<RevmB256>>,
    }

    #[allow(unused_variables)]
    impl DatabaseRef for AuxIndexDB {
        type Error = IndexDBError;

        fn basic_ref(
            &self,
            address: RevmAddress,
        ) -> Result<Option<revm_primitives::AccountInfo>, Self::Error> {
            self.read_state
                .try_borrow_mut()
                .map_err(|_| IndexDBError::Borrow)?
                .entry(address)
                .or_default();
            Ok(Some(Default::default()))
        }
        fn storage_ref(
            &self,
            address: RevmAddress,
            index: RevmU256,
        ) -> Result<RevmU256, Self::Error> {
            self.read_state
                .try_borrow_mut()
                .map_err(|_| IndexDBError::Borrow)?
                .entry(address)
                .or_default()
                .push(index);
            Ok(Default::default())
        }
        fn block_hash_ref(&self, number: u64) -> Result<RevmB256, Self::Error> {
            self.block_numbers
                .try_borrow_mut()
                .map_err(|_| IndexDBError::Borrow)?
                .push(number);
            Ok(Default::default())
        }
        fn code_by_hash_ref(
            &self,
            code_hash: RevmB256,
        ) -> Result<revm_primitives::Bytecode, Self::Error> {
            self.code_hashes
                .try_borrow_mut()
                .map_err(|_| IndexDBError::Borrow)?
                .push(code_hash);
            Ok(Default::default())
        }
    }

    impl DatabaseCommit for AuxIndexDB {
        fn commit(&mut self, changes: revm_primitives::HashMap<RevmAddress, RevmAccount>) {
            for (address, account) in changes {
                self.written_state
                    .entry(address)
                    .or_default()
                    .extend(account.storage.keys());
            }
        }
    }

    impl IndexDB {
        /// Get all touched account addresses and storage keys during the execution of a block,
        /// ignoring newly created accounts.
        ///
        /// Generally used for building an [super::ExecutionDB].
        pub fn new(block: &Block, chain_id: u64, spec_id: SpecId) -> Result<Self, EvmError> {
            let block_env = block_env(&block.header);
            let mut db = AuxIndexDB::default();

            for transaction in &block.body.transactions {
                let mut tx_env = tx_env(transaction);

                // disable nonce check (we're executing with empty accounts, nonce 0)
                tx_env.nonce = None;

                // execute tx
                let evm_builder = Evm::builder()
                    .with_block_env(block_env.clone())
                    .with_tx_env(tx_env)
                    .modify_cfg_env(|cfg| {
                        cfg.chain_id = chain_id;
                        // we're executing with empty accounts, balance 0
                        cfg.disable_balance_check = true;
                    })
                    .with_spec_id(spec_id)
                    .with_external_context(
                        TracerEip3155::new(Box::new(std::io::stderr())).without_summary(),
                    );
                let mut evm = evm_builder.with_ref_db(&mut db).build();
                evm.transact_commit()?;
            }

            let mut db = IndexDB {
                written_state: db
                    .written_state
                    .into_iter()
                    .map(|(address, indexes)| {
                        (
                            Address::from_slice(address.as_slice()),
                            indexes
                                .into_iter()
                                .map(|index| U256::from_big_endian(&index.to_be_bytes_vec()))
                                .collect(),
                        )
                    })
                    .collect(),
                read_state: db
                    .read_state
                    .into_inner()
                    .into_iter()
                    .map(|(address, indexes)| {
                        (
                            Address::from_slice(address.as_slice()),
                            indexes
                                .into_iter()
                                .map(|index| U256::from_big_endian(&index.to_be_bytes_vec()))
                                .collect(),
                        )
                    })
                    .collect(),
                block_numbers: db.block_numbers.into_inner(),
                code_hashes: db.code_hashes.into_inner(),
            };

            // add withdrawal accounts
            if let Some(ref withdrawals) = block.body.withdrawals {
                db.written_state
                    .extend(withdrawals.iter().map(|w| (w.address, Vec::new())))
            }

            Ok(db)
        }
    }
}
