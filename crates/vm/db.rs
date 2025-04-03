use bytes::Bytes;
use ethereum_types::H160;
use ethrex_common::types::BlockHash;
use ethrex_common::{
    types::{AccountInfo, Block, ChainConfig},
    Address, H256, U256,
};
use ethrex_storage::{AccountUpdate, Store};
use ethrex_trie::{NodeRLP, Trie, TrieError};
use revm::{
    db::CacheDB,
    inspectors::TracerEip3155,
    primitives::{result::EVMError as RevmError, Address as RevmAddress},
    Database, DatabaseRef, Evm,
};
use revm_primitives::SpecId;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::backends::revm::{block_env, db::evm_state, tx_env};
use crate::{
    backends::{self},
    errors::ExecutionDBError,
};

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

        let mut state = evm_state(store.clone(), block.header.parent_hash);

        let result = backends::revm::REVM::execute_block(block, &mut state).map_err(Box::new)?;
        Ok(result.account_updates)
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
        // this code was copied from the L1
        // TODO: if we change EvmState so that it accepts a CacheDB<RpcDB> then we can
        // simply call execute_block().

        let mut db = CacheDB::new(db);

        // beacon root call
        #[cfg(not(feature = "l2"))]
        {
            use lazy_static::lazy_static;
            use revm::DatabaseCommit;
            use revm_primitives::{TxEnv, TxKind as RevmTxKind, U256 as RevmU256};

            lazy_static! {
                static ref SYSTEM_ADDRESS: RevmAddress = RevmAddress::from_slice(
                    &hex::decode("fffffffffffffffffffffffffffffffffffffffe").unwrap()
                );
                static ref CONTRACT_ADDRESS: RevmAddress = RevmAddress::from_slice(
                    &hex::decode("000F3df6D732807Ef1319fB7B8bB8522d0Beac02").unwrap(),
                );
            };
            let beacon_root = match block.header.parent_beacon_block_root {
                None => {
                    return Err(RevmError::Custom(
                        "parent_beacon_block_root field is missing".to_string(),
                    ))
                }
                Some(beacon_root) => beacon_root,
            };

            let tx_env = TxEnv {
                caller: *SYSTEM_ADDRESS,
                transact_to: RevmTxKind::Call(*CONTRACT_ADDRESS),
                gas_limit: 30_000_000,
                data: revm::primitives::Bytes::copy_from_slice(beacon_root.as_bytes()),
                ..Default::default()
            };
            let mut block_env = block_env(&block.header, spec_id);
            block_env.basefee = RevmU256::ZERO;
            block_env.gas_limit = RevmU256::from(30_000_000);

            let mut evm = Evm::builder()
                .with_db(&mut db)
                .with_block_env(block_env)
                .with_tx_env(tx_env)
                .with_spec_id(spec_id)
                .build();

            let transaction_result = evm.transact()?;

            let mut result_state = transaction_result.state;
            result_state.remove(&*SYSTEM_ADDRESS);
            result_state.remove(&evm.block().coinbase);

            evm.context.evm.db.commit(result_state);
        }

        // execute block
        let block_env = block_env(&block.header, spec_id);

        for transaction in &block.body.transactions {
            let tx_env = tx_env(transaction, transaction.sender());

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

/// Creates an [ExecutionDB] from an initial database and a block to execute, usually via
/// pre-execution.
pub trait ToExecDB {
    fn to_exec_db(&self, block: &Block) -> Result<ExecutionDB, ExecutionDBError>;
}
