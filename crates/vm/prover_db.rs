use bytes::Bytes;
use ethrex_common::{
    types::{AccountInfo, AccountState, AccountUpdate, ChainConfig},
    Address, H256, U256,
};
use ethrex_rlp::{decode::RLPDecode, encode::RLPEncode};
use ethrex_trie::Trie;
use serde::{Deserialize, Serialize};
use sha3::{Digest, Keccak256};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use crate::{EvmError, VmDatabase};

/// In-memory EVM database for single batch execution data.
///
/// This is mainly used to store the relevant state data for executing a single batch and then
/// feeding the DB into a zkVM program to prove the execution.
#[derive(Clone, Serialize, Deserialize, Default)]
pub struct ProverDB {
    /// indexed by code hash
    pub code: HashMap<H256, Bytes>,
    /// indexed by block number
    pub block_hashes: HashMap<u64, H256>,
    /// stored chain config
    pub chain_config: ChainConfig,
    #[serde(skip)]
    pub state_trie: Arc<Mutex<Trie>>,
    #[serde(skip)]
    /// indexed by account address
    pub storage_tries: Arc<Mutex<HashMap<Address, Trie>>>,
}

impl ProverDB {
    pub fn get_chain_config(&self) -> ChainConfig {
        self.chain_config
    }

    pub fn apply_account_updates_from_trie(&mut self, account_updates: &[AccountUpdate]) {
        let mut state_trie_lock = self.state_trie.lock().expect("Failed to lock trie");
        let mut storage_tries_lock = self
            .storage_tries
            .lock()
            .expect("Failed to lock storage tries");
        for update in account_updates.iter() {
            let hashed_address = hash_address(&update.address);
            if update.removed {
                // Remove account from trie
                state_trie_lock
                    .remove(hashed_address)
                    .expect("failed to remove from trie");
            } else {
                // Add or update AccountState in the trie
                // Fetch current state or create a new state to be inserted
                let mut account_state = match state_trie_lock
                    .get(&hashed_address)
                    .expect("failed to get account state from trie")
                {
                    Some(encoded_state) => AccountState::decode(&encoded_state)
                        .expect("failed to decode account state"),
                    None => AccountState::default(),
                };
                if let Some(info) = &update.info {
                    account_state.nonce = info.nonce;
                    account_state.balance = info.balance;
                    account_state.code_hash = info.code_hash;
                    // Store updated code in DB
                    if let Some(code) = &update.code {
                        self.code.insert(info.code_hash, code.clone());
                    }
                }
                // Store the added storage in the account's storage trie and compute its new root
                if !update.added_storage.is_empty() {
                    let storage_trie =
                        storage_tries_lock.entry(update.address).or_insert_with(|| {
                            Trie::from_nodes(None, &[]).expect("failed to create empty trie")
                        });

                    for (storage_key, storage_value) in &update.added_storage {
                        let hashed_key = hash_key(storage_key);
                        if storage_value.is_zero() {
                            storage_trie
                                .remove(hashed_key)
                                .expect("failed to remove key");
                        } else {
                            storage_trie
                                .insert(hashed_key, storage_value.encode_to_vec())
                                .expect("failed to insert in trie");
                        }
                    }
                    account_state.storage_root = storage_trie
                        .hash()
                        .expect("failed to calculate storage trie root");
                }
                state_trie_lock
                    .insert(hashed_address, account_state.encode_to_vec())
                    .expect("failed to insert into storage");
            }
        }
    }
}

impl VmDatabase for ProverDB {
    fn get_account_info(&self, address: Address) -> Result<Option<AccountInfo>, EvmError> {
        let state_trie_lock = self
            .state_trie
            .lock()
            .map_err(|_| EvmError::DB("Failed to lock state trie".to_string()))?;
        let hashed_address = hash_address(&address);
        let Ok(Some(encoded_state)) = state_trie_lock.get(&hashed_address) else {
            return Ok(None);
        };
        let state = AccountState::decode(&encoded_state)
            .map_err(|_| EvmError::DB("Failed to get decode account from trie".to_string()))?;

        Ok(Some(AccountInfo {
            balance: state.balance,
            code_hash: state.code_hash,
            nonce: state.nonce,
        }))
    }

    fn get_block_hash(&self, block_number: u64) -> Result<H256, EvmError> {
        self.block_hashes
            .get(&block_number)
            .cloned()
            .ok_or_else(|| {
                EvmError::DB(format!(
                    "Block hash not found for block number {block_number}"
                ))
            })
    }

    fn get_storage_slot(&self, address: Address, key: H256) -> Result<Option<U256>, EvmError> {
        let storage_tries_lock = self
            .storage_tries
            .lock()
            .map_err(|_| EvmError::DB("Failed to lock storage tries".to_string()))?;

        let Some(storage_trie) = storage_tries_lock.get(&address) else {
            return Ok(None);
        };
        let hashed_key = hash_key(&key);
        if let Ok(Some(encoded_key)) = storage_trie.get(&hashed_key) {
            U256::decode(&encoded_key)
                .map_err(|_| EvmError::DB("failed to read storage from trie".to_string()))
                .map(Some)
        } else {
            Ok(None)
        }
    }

    fn get_chain_config(&self) -> Result<ethrex_common::types::ChainConfig, EvmError> {
        Ok(self.get_chain_config())
    }

    fn get_account_code(&self, code_hash: H256) -> Result<bytes::Bytes, EvmError> {
        match self.code.get(&code_hash) {
            Some(code) => Ok(code.clone()),
            None => Err(EvmError::DB(format!(
                "Could not find code for hash {}",
                code_hash
            ))),
        }
    }
}

fn hash_address(address: &Address) -> Vec<u8> {
    Keccak256::new_with_prefix(address.to_fixed_bytes())
        .finalize()
        .to_vec()
}

pub fn hash_key(key: &H256) -> Vec<u8> {
    Keccak256::new_with_prefix(key.to_fixed_bytes())
        .finalize()
        .to_vec()
}
