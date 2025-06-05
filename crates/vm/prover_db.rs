use bytes::Bytes;
use ethereum_types::H160;
use ethrex_common::{
    types::{AccountInfo, AccountUpdate, ChainConfig},
    Address, H256, U256,
};
use ethrex_trie::{NodeRLP, Trie, TrieError};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::errors::ProverDBError;

/// In-memory EVM database for single batch execution data.
///
/// This is mainly used to store the relevant state data for executing a single batch and then
/// feeding the DB into a zkVM program to prove the execution.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
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
}

impl ProverDB {
    pub fn get_chain_config(&self) -> ChainConfig {
        self.chain_config
    }

    /// Recreates the state trie and storage tries from the encoded nodes.
    pub fn get_tries(&self) -> Result<(Trie, HashMap<H160, Trie>), ProverDBError> {
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
                // Add or update AccountInfo
                // Fetch current account_info or create a new one to be inserted
                let mut account_info = match self.accounts.get(&update.address) {
                    Some(account_info) => account_info.clone(),
                    None => AccountInfo::default(),
                };
                if let Some(info) = &update.info {
                    account_info.nonce = info.nonce;
                    account_info.balance = info.balance;
                    account_info.code_hash = info.code_hash;

                    // Store updated code
                    if let Some(code) = &update.code {
                        self.code.insert(info.code_hash, code.clone());
                    }
                }
                // Insert new AccountInfo
                self.accounts.insert(update.address, account_info);

                // Store the added storage
                if !update.added_storage.is_empty() {
                    let mut storage = match self.storage.get(&update.address) {
                        Some(storage) => storage.clone(),
                        None => HashMap::default(),
                    };
                    for (storage_key, storage_value) in &update.added_storage {
                        if storage_value.is_zero() {
                            storage.remove(storage_key);
                        } else {
                            storage.insert(*storage_key, *storage_value);
                        }
                    }
                    self.storage.insert(update.address, storage);
                }
            }
        }
    }
}
