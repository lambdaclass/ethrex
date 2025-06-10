use bytes::Bytes;
use ethereum_types::H160;
use ethrex_common::{
    types::{AccountInfo, AccountUpdate, BlockHeader, ChainConfig, EMPTY_KECCACK_HASH},
    Address, H256, U256,
};
use ethrex_trie::{NodeRLP, Trie, TrieError};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::errors::ProverDBError;
use crate::{EvmError, VmDatabase};

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
    pub block_headers: HashMap<u64, BlockHeader>,
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

    /// Returns Some(block_number) if the hash for block_number is not the parent
    /// hash of block_number + 1. None if there's no such hash.
    ///
    /// Keep in mind that the last block hash (which is a batch's parent hash)
    /// can't be validated against the next header, because it has no successor.
    pub fn get_first_invalid_block_hash(&self) -> Result<Option<u64>, ProverDBError> {
        // Enforces there's at least one block header, so windows() call doesn't panic.
        if self.block_headers.is_empty() {
            return Err(ProverDBError::NoBlockHeaders);
        };

        // Sort in ascending order
        let mut block_headers: Vec<_> = self.block_headers.iter().collect();
        block_headers.sort_by_key(|(number, _)| *number);

        // Validate hashes
        for window in block_headers.windows(2) {
            let (Some((number, header)), Some((next_number, next_header))) =
                (window.first().cloned(), window.get(1).cloned())
            else {
                // windows() returns an empty iterator in this case.
                return Err(ProverDBError::Unreachable(
                    "block header window len is < 2".to_string(),
                ));
            };
            if *next_number != *number + 1 {
                return Err(ProverDBError::NoncontiguousBlockHeaders);
            }
            if next_header.parent_hash != header.hash() {
                return Ok(Some(*number));
            }
        }

        Ok(None)
    }

    pub fn get_last_block_header(&self) -> Result<&BlockHeader, ProverDBError> {
        let latest_block_header = self
            .block_headers
            .keys()
            .max()
            .ok_or(ProverDBError::NoBlockHeaders)?;
        self.block_headers
            .get(latest_block_header)
            .ok_or(ProverDBError::Unreachable(
                "empty block headers after retreiving non-empty keys".to_string(),
            ))
    }
}

impl VmDatabase for ProverDB {
    fn get_account_info(&self, address: Address) -> Result<Option<AccountInfo>, EvmError> {
        Ok(self.accounts.get(&address).cloned())
    }

    fn get_storage_slot(&self, address: Address, key: H256) -> Result<Option<U256>, EvmError> {
        Ok(self
            .storage
            .get(&address)
            .and_then(|storage| storage.get(&key).cloned()))
    }

    fn get_block_hash(&self, block_number: u64) -> Result<H256, EvmError> {
        self.block_headers
            .get(&block_number)
            .map(|header| header.hash())
            .ok_or_else(|| {
                EvmError::DB(format!(
                    "Block hash not found for block number {block_number}"
                ))
            })
    }

    fn get_chain_config(&self) -> Result<ChainConfig, EvmError> {
        Ok(self.get_chain_config())
    }

    fn get_account_code(&self, code_hash: H256) -> Result<Bytes, EvmError> {
        if code_hash == *EMPTY_KECCACK_HASH {
            return Ok(Bytes::new());
        }
        self.code
            .get(&code_hash)
            .cloned()
            .ok_or_else(|| EvmError::DB(format!("Code not found for hash: {:?}", code_hash)))
    }
}
