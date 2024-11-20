use std::collections::HashMap;

use ethereum_types::{Address, H160, U256};
use ethrex_core::{
    types::{AccountState, Block, ChainConfig},
    H256,
};
use ethrex_rlp::encode::RLPEncode;
use ethrex_storage::{hash_address, hash_key, Store};
use ethrex_trie::Trie;
use revm::{
    primitives::{
        AccountInfo as RevmAccountInfo, Address as RevmAddress, Bytecode as RevmBytecode,
        B256 as RevmB256, U256 as RevmU256,
    },
    DatabaseRef,
};
use serde::{Deserialize, Serialize};

use crate::{
    errors::{ExecutionDBError, StateProofsError},
    evm_state, execute_block, get_state_transitions,
};

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
    /// proofs of inclusion of account and storage values of the initial state
    pub initial_proofs: StateProofs,
}

/// Merkle proofs of inclusion of state values.
///
/// Contains Merkle proofs to verfy the inclusion of values in the state and storage tries.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct StateProofs {
    account: HashMap<RevmAddress, Vec<Vec<u8>>>,
    storage: HashMap<RevmAddress, HashMap<RevmU256, Vec<Vec<u8>>>>,
}

impl ExecutionDB {
    /// Creates a database by executing a block, without performing any validation.
    pub fn from_exec(block: &Block, store: &Store) -> Result<Self, ExecutionDBError> {
        // TODO: perform validation to exit early

        // Execute and obtain account updates
        let mut state = evm_state(store.clone(), block.header.parent_hash);
        let chain_config = store.get_chain_config()?;
        execute_block(block, &mut state).map_err(Box::new)?;
        let account_updates = get_state_transitions(&mut state);

        // Store data touched by updates and get all touched storage keys for each account
        let mut accounts = HashMap::new();
        let code = HashMap::new(); // TODO: `code` remains empty for now
        let mut storage = HashMap::new();
        let block_hashes = HashMap::new(); // TODO: `block_hashes` remains empty for now

        let mut address_storage_keys = HashMap::new();

        for account_update in account_updates.iter() {
            let address = RevmAddress::from_slice(account_update.address.as_bytes());
            let account_state = store
                .get_account_state_by_hash(
                    block.header.parent_hash,
                    H160::from_slice(address.as_slice()),
                )?
                .ok_or(ExecutionDBError::NewMissingAccountInfo(address))?;
            accounts.insert(address, account_state);

            let account_storage = account_update
                .added_storage
                .iter()
                .map(|(key, value)| {
                    let mut value_bytes = [0u8; 32];
                    value.to_big_endian(&mut value_bytes);
                    (
                        RevmU256::from_be_bytes(key.to_fixed_bytes()),
                        RevmU256::from_be_slice(&value_bytes),
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

        // Compute Merkle proofs for the initial state values
        let initial_state_trie = store.state_trie(block.header.parent_hash)?.ok_or(
            ExecutionDBError::NewMissingStateTrie(block.header.parent_hash),
        )?;
        let initial_storage_tries = accounts
            .keys()
            .map(|address| {
                Ok((
                    H160::from_slice(address.as_slice()),
                    store
                        .storage_trie(
                            block.header.parent_hash,
                            H160::from_slice(address.as_slice()),
                        )?
                        .ok_or(ExecutionDBError::NewMissingStorageTrie(
                            block.header.parent_hash,
                            *address,
                        ))?,
                ))
            })
            .collect::<Result<HashMap<_, _>, ExecutionDBError>>()?;

        let initial_proofs = StateProofs::new(
            &initial_state_trie,
            &initial_storage_tries,
            &address_storage_keys,
        )?;

        Ok(Self {
            accounts,
            code,
            storage,
            block_hashes,
            chain_config,
            initial_proofs,
        })
    }

    pub fn get_chain_config(&self) -> ChainConfig {
        self.chain_config
    }

    /// Verifies that [self] holds the initial state (prior to block execution) with some root
    /// hash.
    pub fn verify_initial_state(&self, state_root: H256) -> Result<bool, StateProofsError> {
        self.verify_state_proofs(state_root, &self.initial_proofs)
    }

    fn verify_state_proofs(
        &self,
        state_root: H256,
        proofs: &StateProofs,
    ) -> Result<bool, StateProofsError> {
        proofs.verify(state_root, &self.accounts, &self.storage)
    }
}

impl StateProofs {
    fn new(
        state_trie: &Trie,
        storage_tries: &HashMap<Address, Trie>,
        address_storage_keys: &HashMap<Address, Vec<H256>>,
    ) -> Result<Self, StateProofsError> {
        let mut account = HashMap::default();
        let mut storage = HashMap::default();

        for (address, storage_keys) in address_storage_keys {
            let storage_trie = storage_tries
                .get(address)
                .ok_or(StateProofsError::StorageTrieNotFound(*address))?;

            let proof = state_trie.get_proof(&hash_address(address))?;
            let address = RevmAddress::from_slice(address.as_bytes());
            account.insert(address, proof);

            let mut storage_proofs = HashMap::new();
            for key in storage_keys {
                let proof = storage_trie.get_proof(&hash_key(key))?;
                let key = RevmU256::from_be_bytes(key.to_fixed_bytes());
                storage_proofs.insert(key, proof);
            }
            storage.insert(address, storage_proofs);
        }

        Ok(Self { account, storage })
    }

    fn verify(
        &self,
        state_root: H256,
        accounts: &HashMap<RevmAddress, AccountState>,
        storages: &HashMap<RevmAddress, HashMap<RevmU256, RevmU256>>,
    ) -> Result<bool, StateProofsError> {
        // Check accounts inclusion in the state trie
        for (address, account) in accounts {
            let proof = self
                .account
                .get(address)
                .ok_or(StateProofsError::AccountProofNotFound(*address))?;

            let hashed_address = hash_address(&H160::from_slice(address.as_slice()));
            let mut encoded_account = Vec::new();
            account.encode(&mut encoded_account);

            if !Trie::verify_proof(proof, state_root.into(), &hashed_address, &encoded_account)? {
                return Ok(false);
            }
        }
        // so all account storage roots are valid at this point

        // Check storage values inclusion in storage tries
        for (address, storage) in storages {
            let storage_root = accounts
                .get(address)
                .map(|account| account.storage_root)
                .ok_or(StateProofsError::StorageNotFound(*address))?;

            let storage_proofs = self
                .storage
                .get(address)
                .ok_or(StateProofsError::StorageProofsNotFound(*address))?;

            for (key, value) in storage {
                let proof = storage_proofs
                    .get(key)
                    .ok_or(StateProofsError::StorageProofNotFound(*address, *key))?;

                let hashed_key = hash_key(&H256::from_slice(&key.to_be_bytes_vec()));
                let encoded_value = U256::from_big_endian(&value.to_be_bytes_vec()).encode_to_vec();

                if !Trie::verify_proof(proof, storage_root.into(), &hashed_key, &encoded_value)? {
                    return Ok(false);
                }
            }
        }

        Ok(true)
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
            balance: {
                let mut balance_bytes = [0; 32];
                account_state.balance.to_big_endian(&mut balance_bytes);
                RevmU256::from_be_bytes(balance_bytes)
            },
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
            .ok_or(ExecutionDBError::StorageNotFound(address, index))
    }

    /// Get block hash by block number.
    fn block_hash_ref(&self, number: u64) -> Result<RevmB256, Self::Error> {
        self.block_hashes
            .get(&number)
            .cloned()
            .ok_or(ExecutionDBError::BlockHashNotFound(number))
    }
}
