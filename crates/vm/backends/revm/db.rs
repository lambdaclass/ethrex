use ethrex_common::{types::ChainConfig, Address as CoreAddress, H256 as CoreH256};
use ethrex_trie::{Node, NodeRLP, PathRLP, Trie};
use revm::{
    primitives::{
        AccountInfo as RevmAccountInfo, Address as RevmAddress, Bytecode as RevmBytecode,
        Bytes as RevmBytes, B256 as RevmB256, U256 as RevmU256,
    },
    DatabaseRef,
};

use crate::errors::{EvmError, ProverDBError};
use crate::{db::DynVmDatabase, prover_db::ProverDB};

/// State used when running the EVM. The state can be represented with a [VmDbWrapper] database, or
/// with a [ProverDB] in case we only want to store the necessary data for some particular
/// execution, for example when proving in L2 mode.
///
/// Encapsulates state behaviour to be agnostic to the evm implementation for crate users.
pub enum EvmState {
    Store(revm::db::State<DynVmDatabase>),
    Execution(Box<revm::db::CacheDB<ProverDB>>),
}

// Needed because revm::db::State is not cloneable and we need to
// restore the previous EVM state after executing a transaction in L2 mode whose resulting state diff doesn't fit in a blob.
impl Clone for EvmState {
    fn clone(&self) -> Self {
        match self {
            EvmState::Store(state) => EvmState::Store(revm::db::State::<DynVmDatabase> {
                cache: state.cache.clone(),
                database: state.database.clone(),
                transition_state: state.transition_state.clone(),
                bundle_state: state.bundle_state.clone(),
                use_preloaded_bundle: state.use_preloaded_bundle,
                block_hashes: state.block_hashes.clone(),
            }),
            EvmState::Execution(execution) => {
                EvmState::Execution(Box::new(Into::<revm::db::CacheDB<ProverDB>>::into(
                    *execution.clone(),
                )))
            }
        }
    }
}

impl EvmState {
    /// Gets the stored chain config
    pub fn chain_config(&self) -> Result<ChainConfig, EvmError> {
        match self {
            EvmState::Store(db) => Ok(db.database.0.get_chain_config()),
            EvmState::Execution(db) => Ok(db.db.get_chain_config()),
        }
    }
}

/// Builds EvmState from a Store
pub fn evm_state(db: DynVmDatabase) -> EvmState {
    EvmState::Store(
        revm::db::State::builder()
            .with_database(db)
            .with_bundle_update()
            .without_state_clear()
            .build(),
    )
}

impl From<ProverDB> for EvmState {
    fn from(value: ProverDB) -> Self {
        EvmState::Execution(Box::new(revm::db::CacheDB::new(value)))
    }
}

impl DatabaseRef for ProverDB {
    /// The database error type.
    type Error = ProverDBError;

    /// Get basic account information.
    fn basic_ref(&self, address: RevmAddress) -> Result<Option<RevmAccountInfo>, Self::Error> {
        let Some(account_info) = self.accounts.get(&CoreAddress::from(address.0.as_ref())) else {
            return Ok(None);
        };

        Ok(Some(RevmAccountInfo {
            balance: RevmU256::from_limbs(account_info.balance.0),
            nonce: account_info.nonce,
            code_hash: RevmB256::from_slice(&account_info.code_hash.0),
            code: None,
        }))
    }

    /// Get account code by its hash.
    fn code_by_hash_ref(&self, code_hash: RevmB256) -> Result<RevmBytecode, Self::Error> {
        self.code
            .get(&CoreH256::from(code_hash.as_ref()))
            .map(|b| RevmBytecode::new_raw(RevmBytes(b.clone())))
            .ok_or(ProverDBError::CodeNotFound(code_hash))
    }

    /// Get storage value of address at index.
    fn storage_ref(&self, address: RevmAddress, index: RevmU256) -> Result<RevmU256, Self::Error> {
        self.storage
            .get(&CoreAddress::from(address.0.as_ref()))
            .ok_or(ProverDBError::AccountNotFound(address))?
            .get(&CoreH256::from(index.to_be_bytes()))
            .map(|v| RevmU256::from_limbs(v.0))
            .ok_or(ProverDBError::StorageValueNotFound(address, index))
    }

    /// Get block hash by block number.
    fn block_hash_ref(&self, number: u64) -> Result<RevmB256, Self::Error> {
        self.block_hashes
            .get(&number)
            .map(|h| RevmB256::from_slice(&h.0))
            .ok_or(ProverDBError::BlockHashNotFound(number))
    }
}

impl revm::Database for DynVmDatabase {
    type Error = EvmError;

    fn basic(&mut self, address: RevmAddress) -> Result<Option<RevmAccountInfo>, Self::Error> {
        let acc_info = match self
            .0
            .get_account_info(CoreAddress::from(address.0.as_ref()))?
        {
            None => return Ok(None),
            Some(acc_info) => acc_info,
        };
        let code = self
            .0
            .get_account_code(acc_info.code_hash)?
            .map(|b| RevmBytecode::new_raw(RevmBytes(b)));

        Ok(Some(RevmAccountInfo {
            balance: RevmU256::from_limbs(acc_info.balance.0),
            nonce: acc_info.nonce,
            code_hash: RevmB256::from(acc_info.code_hash.0),
            code,
        }))
    }

    fn code_by_hash(&mut self, code_hash: RevmB256) -> Result<RevmBytecode, Self::Error> {
        self.0
            .get_account_code(CoreH256::from(code_hash.as_ref()))?
            .map(|b| RevmBytecode::new_raw(RevmBytes(b)))
            .ok_or_else(|| EvmError::DB(format!("No code for hash {code_hash}")))
    }

    fn storage(&mut self, address: RevmAddress, index: RevmU256) -> Result<RevmU256, Self::Error> {
        Ok(self
            .0
            .get_storage_slot(
                CoreAddress::from(address.0.as_ref()),
                CoreH256::from(index.to_be_bytes()),
            )?
            .map(|value| RevmU256::from_limbs(value.0))
            .unwrap_or_else(|| RevmU256::ZERO))
    }

    fn block_hash(&mut self, number: u64) -> Result<RevmB256, Self::Error> {
        self.0
            .get_block_hash(number)?
            .map(|hash| RevmB256::from_slice(&hash.0))
            .ok_or_else(|| EvmError::DB(format!("Block {number} not found")))
    }
}

impl revm::DatabaseRef for DynVmDatabase {
    type Error = EvmError;

    fn basic_ref(&self, address: RevmAddress) -> Result<Option<RevmAccountInfo>, Self::Error> {
        let acc_info = match self
            .0
            .get_account_info(CoreAddress::from(address.0.as_ref()))?
        {
            None => return Ok(None),
            Some(acc_info) => acc_info,
        };
        let code = self
            .0
            .get_account_code(acc_info.code_hash)?
            .map(|b| RevmBytecode::new_raw(RevmBytes(b)));

        Ok(Some(RevmAccountInfo {
            balance: RevmU256::from_limbs(acc_info.balance.0),
            nonce: acc_info.nonce,
            code_hash: RevmB256::from(acc_info.code_hash.0),
            code,
        }))
    }

    fn code_by_hash_ref(&self, code_hash: RevmB256) -> Result<RevmBytecode, Self::Error> {
        self.0
            .get_account_code(CoreH256::from(code_hash.as_ref()))?
            .map(|b| RevmBytecode::new_raw(RevmBytes(b)))
            .ok_or_else(|| EvmError::DB(format!("No code for hash {code_hash}")))
    }

    fn storage_ref(&self, address: RevmAddress, index: RevmU256) -> Result<RevmU256, Self::Error> {
        Ok(self
            .0
            .get_storage_slot(
                CoreAddress::from(address.0.as_ref()),
                CoreH256::from(index.to_be_bytes()),
            )?
            .map(|value| RevmU256::from_limbs(value.0))
            .unwrap_or_else(|| RevmU256::ZERO))
    }

    fn block_hash_ref(&self, number: u64) -> Result<RevmB256, Self::Error> {
        self.0
            .get_block_hash(number)?
            .map(|hash| RevmB256::from_slice(&hash.0))
            .ok_or_else(|| EvmError::DB(format!("Block {number} not found")))
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
