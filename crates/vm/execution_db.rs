use std::collections::{hash_map::Entry, HashMap};

use ethereum_types::H160;
use ethrex_core::{
    types::{Block, ChainConfig},
    Address, H256,
};
use ethrex_levm::SpecId;
use ethrex_storage::{error::StoreError, hash_address, hash_key, AccountUpdate, Store};
use ethrex_trie::{NodeRLP, Trie, TrieError};
use revm::{
    db::CacheDB,
    inspectors::TracerEip3155,
    primitives::{
        result::EVMError as RevmError, AccountInfo as RevmAccountInfo, Address as RevmAddress,
        Bytecode as RevmBytecode, B256 as RevmB256, U256 as RevmU256,
    },
    Database, DatabaseRef, Evm,
};
use serde::{Deserialize, Serialize};

use crate::{
    block_env, db::StoreWrapper, errors::ExecutionDBError, evm_state, execute_block,
    get_state_transitions, spec_id, tx_env, EvmError,
};

/// In-memory EVM database for single execution data.
///
/// This is mainly used to store the relevant state data for executing a single block and then
/// feeding the DB into a zkVM program to prove the execution.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ExecutionDB {
    /// indexed by account address
    pub accounts: HashMap<RevmAddress, RevmAccountInfo>,
    /// indexed by code hash
    pub code: HashMap<RevmB256, RevmBytecode>,
    /// indexed by account address and storage key
    pub storage: HashMap<RevmAddress, HashMap<RevmU256, RevmU256>>,
    /// indexed by block number
    pub block_hashes: HashMap<u64, RevmB256>,
    /// stored chain config
    pub chain_config: ChainConfig,
}

impl ExecutionDB {
    /// Creates a database and returns the ExecutionDB by "pre-executing" a block,
    /// without performing any validation, and retrieving data from a [Store].
    pub fn from_store(
        block: &Block,
        store: Store,
    ) -> Result<(Self, ExecutionDBProofs), ExecutionDBError> {
        let parent_hash = block.header.parent_hash;
        let chain_config = store.get_chain_config()?;
        let store_wrapper = StoreWrapper {
            store: store.clone(),
            block_hash: parent_hash,
        };

        // pre-execute and get all state changes
        let cache = Self::pre_execute(
            block,
            chain_config.chain_id,
            spec_id(&chain_config, block.header.timestamp),
            store_wrapper,
        )
        .map_err(|err| Box::new(EvmError::from(err)))?; // TODO: must be a better way
        let store_wrapper = cache.db;

        // fetch initial state
        let already_existing_accounts = cache
            .accounts
            .iter()
            // filter new accounts, we're only interested in already existing accounts.
            // new accounts are storage cleared, self-destructed accounts too but they're marked with "not
            // existing" status instead.
            .filter(|(_, account)| account.account_state.is_storage_cleared());
        let db = Self {
            accounts: already_existing_accounts
                .clone()
                .map(|(address, _)| {
                    // return error if account is missing
                    let account = match store_wrapper.basic_ref(*address) {
                        Ok(None) => Err(ExecutionDBError::NewMissingAccountInfo(
                            Address::from_slice(address.as_slice()),
                        )),
                        Ok(Some(some)) => Ok(some),
                        Err(err) => Err(ExecutionDBError::Store(err)),
                    };
                    Ok((*address, account?))
                })
                .collect::<Result<_, ExecutionDBError>>()?,
            code: already_existing_accounts
                .clone()
                .map(|(_, account)| {
                    // return error if code is missing
                    let hash = account.info.code_hash;
                    Ok((
                        hash,
                        cache
                            .contracts
                            .get(&hash)
                            .cloned()
                            .ok_or(ExecutionDBError::NewMissingCode(hash))?,
                    ))
                })
                .collect::<Result<_, ExecutionDBError>>()?,
            storage: already_existing_accounts
                .map(|(address, account)| {
                    // return error if storage is missing
                    Ok((
                        *address,
                        account
                            .storage
                            .keys()
                            .map(|key| Ok((*key, store_wrapper.storage_ref(*address, *key)?)))
                            .collect::<Result<_, StoreError>>()?,
                    ))
                })
                .collect::<Result<_, StoreError>>()?,
            block_hashes: cache
                .block_hashes
                .into_iter()
                .map(|(num, hash)| (num.try_into().unwrap(), hash))
                .collect(),
            chain_config,
        };

        // get proofs
        let state_trie = store
            .state_trie(parent_hash)?
            .ok_or(ExecutionDBError::NewMissingStateTrie(parent_hash))?;

        let state_proofs = state_trie.get_proofs(
            &db.accounts
                .keys()
                .map(|a| hash_address(&Address::from_slice(a.as_slice())))
                .collect::<Vec<_>>(),
        )?;

        let mut storage_proofs = HashMap::new();
        for (revm_address, storages) in &db.storage {
            let address = Address::from_slice(revm_address.as_slice());

            let storage_trie = store.storage_trie(parent_hash, address)?.ok_or(
                ExecutionDBError::NewMissingStorageTrie(parent_hash, address),
            )?;

            let paths = storages
                .keys()
                .map(|k| hash_key(&H256::from_slice(&k.to_be_bytes_vec())))
                .collect::<Vec<_>>();
            storage_proofs.insert(address, storage_trie.get_proofs(&paths)?);
        }

        let proofs = ExecutionDBProofs {
            state: state_proofs,
            storage: storage_proofs,
        };

        Ok((db, proofs))
    }

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
            let mut tx_env = tx_env(transaction);

            // disable nonce check (we're executing with empty accounts, nonce 0)
            tx_env.nonce = None;

            // execute tx
            let evm_builder = Evm::builder()
                .with_block_env(block_env.clone())
                .with_tx_env(tx_env)
                .modify_cfg_env(|cfg| {
                    cfg.chain_id = chain_id;
                    // we're executing with empty accounts, balance 0
                    cfg.disable_balance_check = true;
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
        let Some(account_state) = self.accounts.get(&address) else {
            return Ok(None);
        };

        Ok(Some(RevmAccountInfo {
            balance: account_state.balance,
            nonce: account_state.nonce,
            code_hash: account_state.code_hash,
            code: None,
        }))
    }

    /// Get account code by its hash.
    fn code_by_hash_ref(&self, code_hash: RevmB256) -> Result<RevmBytecode, Self::Error> {
        self.code
            .get(&code_hash)
            .cloned()
            .ok_or(ExecutionDBError::CodeNotFound(code_hash))
    }

    /// Get storage value of address at index.
    fn storage_ref(&self, address: RevmAddress, index: RevmU256) -> Result<RevmU256, Self::Error> {
        self.storage
            .get(&address)
            .ok_or(ExecutionDBError::AccountNotFound(address))?
            .get(&index)
            .cloned()
            .ok_or(ExecutionDBError::StorageValueNotFound(address, index))
    }

    /// Get block hash by block number.
    fn block_hash_ref(&self, number: u64) -> Result<RevmB256, Self::Error> {
        self.block_hashes
            .get(&number)
            .cloned()
            .ok_or(ExecutionDBError::BlockHashNotFound(number))
    }
}

/// Proofs used for authenticating an [ExecutionDB] and recreating the state and storage tries.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ExecutionDBProofs {
    /// Encoded nodes to reconstruct a state trie, but only including relevant data ("pruned trie").
    ///
    /// Root node is stored separately from the rest as the first tuple member.
    pub state: (Option<NodeRLP>, Vec<NodeRLP>),
    /// Encoded nodes to reconstruct every storage trie, but only including relevant data ("pruned
    /// trie").
    ///
    /// Root node is stored separately from the rest as the first tuple member.
    pub storage: HashMap<H160, (Option<NodeRLP>, Vec<NodeRLP>)>,
}

impl ExecutionDBProofs {
    pub fn get_tries(&self) -> Result<(Trie, HashMap<H160, Trie>), ExecutionDBError> {
        let (state_trie_root, state_trie_nodes) = &self.state;
        let state_trie = Trie::from_nodes(state_trie_root.as_ref(), state_trie_nodes)?;

        let storage_trie = self
            .storage
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
