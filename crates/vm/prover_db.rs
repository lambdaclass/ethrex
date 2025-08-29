use bytes::Bytes;
use ethereum_types::H160;
use ethrex_common::types::BlockHeader;
use ethrex_common::{
    Address, H256, U256,
    constants::EMPTY_KECCACK_HASH,
    types::{AccountInfo, AccountUpdate, ChainConfig},
};
use ethrex_trie::{NodeHash, NodeRLP, Trie, TrieError};
use rkyv::{Archive, Deserialize as RDeserialize, Serialize as RSerialize};
use serde::{Deserialize, Serialize};
use sha3::{Digest, Keccak256};
use std::collections::HashMap;

use crate::errors::ProverDBError;
use crate::{EvmError, VmDatabase};
use ethrex_common::rkyv_utils::{
    AccountInfoWrapper, BytesWrapper, EncodedTrieWrapper, H160Wrapper, H256Wrapper, U256Wrapper,
};
use ethrex_common::types::block_execution_witness::ExecutionWitnessResult;

#[derive(Serialize, Deserialize, RSerialize, RDeserialize, Archive)]
pub enum PreExecutionState {
    DB(Box<ProverDB>),
    Witness(Box<ExecutionWitnessResult>),
}

impl PreExecutionState {
    pub fn chain_id(&self) -> u64 {
        match self {
            PreExecutionState::DB(db) => db.chain_config.chain_id,
            PreExecutionState::Witness(witness) => witness.chain_config.chain_id,
        }
    }
}

/// In-memory EVM database for single batch execution data.
///
/// This is mainly used to store the relevant state data for executing a single batch and then
/// feeding the DB into a zkVM program to prove the execution.
#[derive(Debug, Clone, Serialize, Deserialize, Default, RSerialize, RDeserialize, Archive)]
pub struct ProverDB {
    /// indexed by account address
    #[rkyv(with=rkyv::with::MapKV<H160Wrapper, AccountInfoWrapper>)]
    pub accounts: HashMap<Address, AccountInfo>,
    /// indexed by code hash
    #[rkyv(with=rkyv::with::MapKV<H256Wrapper, BytesWrapper>)]
    pub code: HashMap<H256, Bytes>,
    /// indexed by account address and storage key
    #[rkyv(with=rkyv::with::MapKV<H160Wrapper, rkyv::with::MapKV<H256Wrapper, U256Wrapper>>)]
    pub storage: HashMap<Address, HashMap<H256, U256>>,
    /// indexed by block number
    pub block_headers: HashMap<u64, BlockHeader>,
    /// stored chain config
    pub chain_config: ChainConfig,
    /// Encoded nodes to reconstruct a state trie, but only including relevant data ("pruned trie").
    ///
    /// Root node is stored separately from the rest as the first tuple member.
    #[rkyv(with=EncodedTrieWrapper)]
    pub state_proofs: (Option<NodeRLP>, Vec<NodeRLP>),
    /// Encoded nodes to reconstruct every storage trie, but only including relevant data ("pruned
    /// trie").
    ///
    /// Root node is stored separately from the rest as the first tuple member.
    #[rkyv(with=rkyv::with::MapKV<H160Wrapper, EncodedTrieWrapper>)]
    pub storage_proofs: HashMap<Address, (Option<NodeRLP>, Vec<NodeRLP>)>,
}

impl ProverDB {
    pub fn get_chain_config(&self) -> ChainConfig {
        self.chain_config
    }

    /// Recreates the state trie and storage tries from the encoded nodes.
    pub fn get_tries(&self) -> Result<(Trie, HashMap<H160, Trie>), ProverDBError> {
        let (state_trie_root, state_trie_nodes) = &self.state_proofs;
        let mut state_nodes = HashMap::new();
        for node in state_trie_nodes.iter() {
            let hash = Keccak256::digest(node);
            state_nodes.insert(NodeHash::Hashed(H256::from_slice(&hash)), node.clone());
        }
        let state_trie = Trie::from_nodes(state_trie_root.as_ref(), state_nodes)?;

        let storage_trie = self
            .storage_proofs
            .iter()
            .map(|(address, (storage_trie_root, storage_trie_nodes))| {
                let mut nodes = HashMap::new();
                for node in storage_trie_nodes.iter() {
                    let hash = Keccak256::digest(node);
                    nodes.insert(NodeHash::Hashed(H256::from_slice(&hash)), node.clone());
                }

                let trie = Trie::from_nodes(storage_trie_root.as_ref(), nodes)?;
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
