use std::collections::HashMap;

use crate::{backends::revm::execution_db::ToExecDB, errors::ExecutionDBError};
use bytes::Bytes;
use ethrex_common::{
    types::{AccountInfo, Block, BlockHash, ChainConfig},
    Address, H160, H256, U256,
};
use ethrex_storage::{AccountUpdate, Store};
use ethrex_trie::{Node, NodeRLP, PathRLP, Trie, TrieError};
use serde::{Deserialize, Serialize};
#[cfg(feature = "levm-l2")]
use std::sync::Arc;

#[cfg(not(feature = "levm-l2"))]
use crate::backends::revm::db::evm_state;

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

impl ToExecDB for StoreWrapper {
    fn to_exec_db(&self, block: &Block) -> Result<ExecutionDB, ExecutionDBError> {
        #[cfg(feature = "levm-l2")]
        {
            self.to_exec_db_levm(block)
        }
        #[cfg(not(feature = "levm-l2"))]
        {
            self.to_exec_db_revm(block)
        }
    }
}

/// Get all potential child nodes of a node whose value was deleted.
///
/// After deleting a value from a (partial) trie it's possible that the node containing the value gets
/// replaced by its child, whose prefix is possibly modified by appending some nibbles to it.
/// If we don't have this child node (because we're modifying a partial trie), then we can't
/// perform the deletion. If we have the final proof of exclusion of the deleted value, we can
/// calculate all posible child nodes.
pub fn get_potential_child_nodes(proof: &[NodeRLP], key: &PathRLP) -> Option<Vec<Node>> {
    // TODO: Perhaps it's possible to calculate the child nodes instead of storing all possible ones.
    let trie = Trie::from_nodes(
        proof.first(),
        &proof.iter().skip(1).cloned().collect::<Vec<_>>(),
    )
    .unwrap();

    // return some only if this is a proof of exclusion
    if trie.get(key).unwrap().is_none() {
        let final_node = Node::decode_raw(proof.last().unwrap()).unwrap();
        match final_node {
            Node::Extension(mut node) => {
                let mut variants = Vec::with_capacity(node.prefix.len());
                while {
                    variants.push(Node::from(node.clone()));
                    node.prefix.next().is_some()
                } {}
                Some(variants)
            }
            Node::Leaf(mut node) => {
                let mut variants = Vec::with_capacity(node.partial.len());
                while {
                    variants.push(Node::from(node.clone()));
                    node.partial.next().is_some()
                } {}
                Some(variants)
            }
            _ => None,
        }
    } else {
        None
    }
}
