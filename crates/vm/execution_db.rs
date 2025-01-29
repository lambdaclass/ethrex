use std::collections::HashMap;

use bytes::Bytes;
use ethereum_types::H160;
use ethrex_core::{
    types::{AccountInfo, Block, ChainConfig},
    Address, H256, U256,
};
<<<<<<< HEAD
use ethrex_storage::{AccountUpdate, Store};
=======
use ethrex_storage::{hash_address, hash_key, AccountUpdate, Store};
>>>>>>> main
use ethrex_trie::{NodeRLP, Trie, TrieError};
use revm::{
    db::CacheDB,
    inspectors::TracerEip3155,
    primitives::{
        result::EVMError as RevmError, AccountInfo as RevmAccountInfo, Address as RevmAddress,
        Bytecode as RevmBytecode, Bytes as RevmBytes, B256 as RevmB256, U256 as RevmU256,
    },
    Database, DatabaseRef, Evm,
};
use revm_primitives::SpecId;
use serde::{Deserialize, Serialize};

use crate::{
    block_env, errors::ExecutionDBError, evm_state, execute_block, get_state_transitions, tx_env,
};

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

        let mut state = evm_state(store.clone(), block.header.parent_hash);

        execute_block(block, &mut state).map_err(Box::new)?;

        let account_updates = get_state_transitions(&mut state);
        Ok(account_updates)
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

    /// Execute a block and cache all state changes, returns the cache
    pub fn pre_execute<ExtDB: DatabaseRef>(
        block: &Block,
        chain_id: u64,
        spec_id: SpecId,
        db: ExtDB,
    ) -> Result<CacheDB<ExtDB>, RevmError<ExtDB::Error>> {
        let block_env = block_env(&block.header);
        let mut db = CacheDB::new(db);

        for transaction in &block.body.transactions {
            let tx_env = tx_env(transaction);

            // execute tx
            let evm_builder = Evm::builder()
                .with_block_env(block_env.clone())
                .with_tx_env(tx_env)
                .modify_cfg_env(|cfg| {
                    cfg.chain_id = chain_id;
                })
                .with_spec_id(spec_id)
                .with_external_context(
                    TracerEip3155::new(Box::new(std::io::stderr())).without_summary(),
                );
            let mut evm = evm_builder.with_db(&mut db).build();
            evm.transact_commit()?;
        }

        // add withdrawal accounts
        if let Some(ref withdrawals) = block.body.withdrawals {
            for withdrawal in withdrawals {
                db.basic(RevmAddress::from_slice(withdrawal.address.as_bytes()))
                    .map_err(RevmError::Database)?;
            }
        }

        Ok(db)
    }
}

impl DatabaseRef for ExecutionDB {
    /// The database error type.
    type Error = ExecutionDBError;

    /// Get basic account information.
    fn basic_ref(&self, address: RevmAddress) -> Result<Option<RevmAccountInfo>, Self::Error> {
        let Some(account_info) = self.accounts.get(&Address::from(address.0.as_ref())) else {
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
            .get(&H256::from(code_hash.as_ref()))
            .map(|b| RevmBytecode::new_raw(RevmBytes(b.clone())))
            .ok_or(ExecutionDBError::CodeNotFound(code_hash))
    }

    /// Get storage value of address at index.
    fn storage_ref(&self, address: RevmAddress, index: RevmU256) -> Result<RevmU256, Self::Error> {
        self.storage
            .get(&Address::from(address.0.as_ref()))
            .ok_or(ExecutionDBError::AccountNotFound(address))?
            .get(&H256::from(index.to_be_bytes()))
            .map(|v| RevmU256::from_limbs(v.0))
            .ok_or(ExecutionDBError::StorageValueNotFound(address, index))
    }

    /// Get block hash by block number.
    fn block_hash_ref(&self, number: u64) -> Result<RevmB256, Self::Error> {
        self.block_hashes
            .get(&number)
            .map(|h| RevmB256::from_slice(&h.0))
            .ok_or(ExecutionDBError::BlockHashNotFound(number))
    }
}

/// Creates an [ExecutionDB] from an initial database and a block to execute, usually via
/// pre-execution.
pub trait ToExecDB {
    fn to_exec_db(&self, block: &Block) -> Result<ExecutionDB, ExecutionDBError>;
}
