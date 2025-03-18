use std::collections::HashMap;

use crate::errors::ExecutionDBError;
use bytes::Bytes;
use ethrex_common::{
    types::{AccountInfo, Block, BlockHash, ChainConfig},
    Address, H160, H256, U256,
};
use ethrex_storage::{AccountUpdate, Store};
use ethrex_trie::{NodeRLP, Trie, TrieError};
use serde::{Deserialize, Serialize};

#[cfg(not(feature = "levm-l2"))]
use crate::backends::revm::db::evm_state;
#[cfg(feature = "levm-l2")]
use std::sync::Arc;

#[derive(Clone)]
pub struct StoreWrapper {
    pub store: Store,
    pub block_hash: BlockHash,
}

/// In-memory EVM database for single execution data.
///
/// This is mainly used to store the relevant state data for executing a single block and then
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
    /// Gets the Vec<[AccountUpdate]>/StateTransitions obtained after executing a block.
    pub fn get_account_updates(
        block: &Block,
        store: &Store,
    ) -> Result<Vec<AccountUpdate>, ExecutionDBError> {
        // TODO: perform validation to exit early

        #[cfg(feature = "levm-l2")]
        {
            let store_wrapper = StoreWrapper {
                store: store.clone(),
                block_hash: block.header.parent_hash,
            };
            let result = crate::backends::levm::LEVM::execute_block(block, Arc::new(store_wrapper))
                .map_err(Box::new)?;
            Ok(result.account_updates)
        }
        #[cfg(not(feature = "levm-l2"))]
        {
            let mut state = evm_state(store.clone(), block.header.parent_hash);

            let result =
                crate::backends::revm::REVM::execute_block(block, &mut state).map_err(Box::new)?;
            Ok(result.account_updates)
        }
    }

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
}
