use bytes::Bytes;
use ethereum_types::H160;
use ethrex_common::types::BlockHash;
use ethrex_common::{
    types::{AccountInfo, ChainConfig},
    Address, H256, U256,
};
use ethrex_storage::{AccountUpdate, Store};
use ethrex_trie::{NodeRLP, Trie, TrieError};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::errors::ExecutionDBError;

#[derive(Clone)]
pub struct StoreWrapper {
    pub store: Store,
    pub block_hash: BlockHash,
}

/// In-memory EVM database for single batch execution data.
///
/// This is mainly used to store the relevant state data for executing a single batch and then
/// feeding the DB into a zkVM program to prove the execution.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ExecutionDB {
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
}

impl ExecutionDB {
    pub fn get_chain_config(&self) -> ChainConfig {
        self.chain_config
    }

    /// Recreates the state trie and storage tries from the encoded nodes.
    pub fn get_tries(&self) -> Result<(Trie, HashMap<H160, Trie>), ExecutionDBError> {
        let (state_trie_root, state_trie_nodes) = &self.state_proofs;
        let state_trie = Trie::from_nodes(state_trie_root.as_ref(), state_trie_nodes)?;

        let storage_trie = self
            .storage_proofs
            .iter()
            .map(|(address, nodes)| {
                let (storage_trie_root, storage_trie_nodes) = nodes;
                let trie = Trie::from_nodes(storage_trie_root.as_ref(), storage_trie_nodes)?;
                Ok((*address, trie))
            })
            .collect::<Result<_, TrieError>>()?;

        Ok((state_trie, storage_trie))
    }

    pub fn apply_account_updates(&mut self, account_updates: &[AccountUpdate]) {
        for update in account_updates.iter() {
            if update.removed {
                self.accounts.remove(&update.address);
            } else {
                // Check if account info needs to be updated
                // If it is, create new struct
                if let Some(info) = &update.info {
                    // If the account already exists, we can just update its info in the array and avoid cloning it
                    if let Some(account) = self.accounts.get_mut(&update.address) {
                        account.nonce = info.nonce;
                        account.balance = info.balance;
                        account.code_hash = info.code_hash;
                    } else {
                        let account_info = AccountInfo {
                            nonce: info.nonce,
                            balance: info.balance,
                            code_hash: info.code_hash,
                        };

                        //Update the account info
                        self.accounts.insert(update.address, account_info);
                    }

                    if let Some(code) = &update.code {
                        self.code.insert(info.code_hash, code.clone());
                    }
                }

                // Store the added storage
                if !update.added_storage.is_empty() {
                    let update_storage = |storage: &mut HashMap<H256, U256>| {
                        for (storage_key, storage_value) in &update.added_storage {
                            if storage_value.is_zero() {
                                storage.remove(storage_key);
                            } else {
                                storage.insert(*storage_key, *storage_value);
                            }
                        }
                    };
                    if let Some(storage) = self.storage.get_mut(&update.address) {
                        update_storage(storage);
                    } else {
                        let mut storage = HashMap::default();
                        update_storage(&mut storage);
                        self.storage.insert(update.address, storage);
                    };
                }
            }
        }
    }
}
