use bytes::Bytes;
use ethrex_common::{
    types::{AccountInfo, AccountState, AccountUpdate, ChainConfig},
    Address, H256, U256,
};
use ethrex_rlp::{decode::RLPDecode, encode::RLPEncode};
use ethrex_storage::{hash_address, hash_key};
use ethrex_trie::{NodeRLP, Trie};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

/// In-memory EVM database for single batch execution data.
///
/// This is mainly used to store the relevant state data for executing a single batch and then
/// feeding the DB into a zkVM program to prove the execution.
#[derive(Clone, Serialize, Deserialize, Default)]
pub struct ProverDB {
    /// indexed by account address
    pub accounts: HashMap<Address, AccountInfo>,
    /// indexed by code hash
    pub code: HashMap<H256, Bytes>,
    /// indexed by account address and storage key
    pub storage: HashMap<Address, HashMap<H256, U256>>,
    /// indexed by block number
    pub block_hashes: HashMap<u64, H256>,
    /// stored chain config
    pub chain_config: ChainConfig,
    /// Encoded nodes to reconstruct a state trie, but only including relevant data ("pruned trie").
    ///
    /// Root node is stored separately from the rest as the first tuple member.
    pub state_proofs: (Option<NodeRLP>, Vec<NodeRLP>),
    /// Encoded nodes to reconstruct every storage trie, but only including relevant data ("pruned
    /// trie").
    ///
    /// Root node is stored separately from the rest as the first tuple member.
    pub storage_proofs: HashMap<Address, (Option<NodeRLP>, Vec<NodeRLP>)>,

    #[serde(skip)]
    pub state_trie: Arc<Mutex<Trie>>,
    #[serde(skip)]
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
