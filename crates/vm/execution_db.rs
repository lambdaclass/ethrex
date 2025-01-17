use std::{
    cell::RefCell,
    collections::{hash_map::Entry, HashMap},
};

use ethereum_types::H160;
use ethrex_core::{
    types::{AccountState, Block, ChainConfig},
    H256, U256,
};
use ethrex_levm::SpecId;
use ethrex_rlp::{decode::RLPDecode, encode::RLPEncode};
use ethrex_storage::{hash_address, hash_key, AccountUpdate, Store};
use ethrex_trie::{NodeRLP, Trie};
use revm::{
    inspectors::TracerEip3155,
    primitives::{
        result::EVMError as RevmError, Account as RevmAccount, AccountInfo as RevmAccountInfo,
        Address as RevmAddress, Bytecode as RevmBytecode, Bytes as RevmBytes, B256 as RevmB256,
        U256 as RevmU256,
    },
    DatabaseCommit, DatabaseRef, Evm,
};
use serde::{Deserialize, Serialize};

use crate::{
    block_env, errors::ExecutionDBError, evm_state, execute_block, get_state_transitions, spec_id,
    tx_env, EvmError,
};

/// In-memory EVM database for caching execution data.
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
    /// encoded nodes to reconstruct a state trie, but only including relevant data (pruned).
    /// root node is stored separately from the rest.
    pub pruned_state_trie: (Option<NodeRLP>, Vec<NodeRLP>),
    /// encoded nodes to reconstruct every storage trie, but only including relevant data (pruned)
    /// root nodes are stored separately from the rest.
    pub pruned_storage_tries: HashMap<H160, (Option<NodeRLP>, Vec<NodeRLP>)>,
}

impl ExecutionDB {
    /// Creates a database and returns the ExecutionDB by "pre-executing" a block,
    /// without performing any validation, and retrieving data from a [Store].
    pub fn from_store(block: &Block, store: &Store) -> Result<Self, ExecutionDBError> {
        let parent_hash = block.header.parent_hash;
        let chain_config = store.get_chain_config()?;

        // pre-execute and get all touched state, block numbers and code hashes
        let pre_exec_db = PreExecDB::exec(
            block,
            chain_config.chain_id,
            spec_id(&chain_config, block.header.timestamp),
        )
        .map_err(|err| Box::new(EvmError::from(err)))?; // TODO: must be a better way

        let read_accounts = pre_exec_db
            .read_accounts
            .into_inner()
            .into_iter()
            .filter_map(|(address, account)| {
                if let Some(account) = account {
                    Some((address, account))
                } else {
                    None
                }
            });
        let accounts = pre_exec_db
            .written_accounts
            .into_iter()
            .chain(read_accounts)
            .collect();
        let code = pre_exec_db.code.into_inner();
        let storage = pre_exec_db.storage.into_inner();
        let block_hashes = pre_exec_db.block_hashes.into_inner();

        Ok(Self {
            accounts,
            code,
            storage,
            block_hashes,
            chain_config,
            pruned_state_trie,
            pruned_storage_tries,
        })
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

    /// Verifies that all data in [self] is included in the stored tries, and then returns the
    /// pruned tries from the stored nodes.
    pub fn build_tries(&self) -> Result<(Trie, HashMap<H160, Trie>), ExecutionDBError> {
        let (state_trie_root, state_trie_nodes) = &self.pruned_state_trie;
        let state_trie = Trie::from_nodes(state_trie_root.as_ref(), state_trie_nodes)?;
        let mut storage_tries = HashMap::new();

        for (revm_address, account) in &self.accounts {
            let address = H160::from_slice(revm_address.as_slice());

            // check account is in state trie
            if state_trie.get(&hash_address(&address))?.is_none() {
                return Err(ExecutionDBError::MissingAccountInStateTrie(address));
            }

            let (storage_trie_root, storage_trie_nodes) =
                self.pruned_storage_tries
                    .get(&address)
                    .ok_or(ExecutionDBError::MissingStorageTrie(address))?;

            // compare account storage root with storage trie root
            let storage_trie = Trie::from_nodes(storage_trie_root.as_ref(), storage_trie_nodes)?;
            if storage_trie.hash_no_commit() != account.storage_root {
                return Err(ExecutionDBError::InvalidStorageTrieRoot(address));
            }

            // check all storage keys are in storage trie and compare values
            let storage = self
                .storage
                .get(revm_address)
                .ok_or(ExecutionDBError::StorageNotFound(*revm_address))?;
            for (key, value) in storage {
                let key = H256::from_slice(&key.to_be_bytes_vec());
                let value = H256::from_slice(&value.to_be_bytes_vec());
                let retrieved_value = storage_trie
                    .get(&hash_key(&key))?
                    .ok_or(ExecutionDBError::MissingKeyInStorageTrie(address, key))?;
                if value.encode_to_vec() != retrieved_value {
                    return Err(ExecutionDBError::InvalidStorageTrieValue(address, key));
                }
            }

            storage_tries.insert(address, storage_trie);
        }

        Ok((state_trie, storage_tries))
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
            balance: RevmU256::from_be_bytes(account_state.balance.to_big_endian()),
            nonce: account_state.nonce,
            code_hash: RevmB256::from_slice(account_state.code_hash.as_bytes()),
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

/// An utility for "pre-executing" a block and retrieving all needed state data from an InnerDB,
/// e.g. a database or an RPC client. The data is finally stored in memory in an [super::ExecutionDB].
#[derive(Default)]
struct PreExecDB<InnerDB: DatabaseRef> {
    written_accounts: HashMap<RevmAddress, RevmAccountInfo>,
    // internal mutability is needed for caching missing values whenever a reference
    // to them is requested, in which case PreExecDB is immutably borrowed
    read_accounts: RefCell<HashMap<RevmAddress, Option<RevmAccountInfo>>>,
    storage: RefCell<HashMap<RevmAddress, HashMap<RevmU256, RevmU256>>>,
    block_hashes: RefCell<HashMap<u64, RevmB256>>,
    code: RefCell<HashMap<RevmB256, RevmBytecode>>,
    db: InnerDB,
}

#[allow(unused_variables)]
impl<InnerDB: DatabaseRef> DatabaseRef for PreExecDB<InnerDB> {
    type Error = InnerDB::Error;

    fn basic_ref(&self, address: RevmAddress) -> Result<Option<RevmAccountInfo>, Self::Error> {
        // WARN: borrot_mut() panics if value is currently borrowed
        match self.read_accounts.borrow_mut().entry(address) {
            Entry::Occupied(entry) => Ok(entry.get().clone()),
            Entry::Vacant(entry) => {
                let account = self.db.basic_ref(address)?;
                entry.insert(account.clone());
                Ok(account)
            }
        }
    }
    fn storage_ref(&self, address: RevmAddress, index: RevmU256) -> Result<RevmU256, Self::Error> {
        // WARN: borrot_mut() panics if value is currently borrowed
        match self.storage.borrow_mut().entry(address) {
            Entry::Occupied(mut account_entry) => match account_entry.get_mut().entry(index) {
                Entry::Occupied(storage_entry) => Ok(storage_entry.get().clone()),
                Entry::Vacant(storage_entry) => {
                    let value = self.db.storage_ref(address, index)?;
                    storage_entry.insert(value);
                    Ok(value)
                }
            },
            Entry::Vacant(account_entry) => {
                let value = self.db.storage_ref(address, index)?;
                account_entry.insert(HashMap::from([(index, value)]));
                Ok(value)
            }
        }
    }
    fn block_hash_ref(&self, number: u64) -> Result<RevmB256, Self::Error> {
        // WARN: borrot_mut() panics if value is currently borrowed
        match self.block_hashes.borrow_mut().entry(number) {
            Entry::Occupied(entry) => Ok(entry.get().clone()),
            Entry::Vacant(entry) => {
                let hash = self.db.block_hash_ref(number)?;
                entry.insert(hash.clone());
                Ok(hash)
            }
        }
    }
    fn code_by_hash_ref(
        &self,
        code_hash: RevmB256,
    ) -> Result<revm_primitives::Bytecode, Self::Error> {
        // WARN: borrot_mut() panics if value is currently borrowed
        match self.code.borrow_mut().entry(code_hash) {
            Entry::Occupied(entry) => Ok(entry.get().clone()),
            Entry::Vacant(entry) => {
                let code = self.db.code_by_hash_ref(code_hash)?;
                entry.insert(code.clone());
                Ok(code)
            }
        }
    }
}

impl<InnerDB: DatabaseRef> DatabaseCommit for PreExecDB<InnerDB> {
    fn commit(&mut self, changes: revm_primitives::HashMap<RevmAddress, RevmAccount>) {
        for (address, account) in changes {
            if !account.is_created() {
                self.written_accounts.entry(address).or_insert(account.info);
            }
        }
    }
}

impl<InnerDB> PreExecDB<InnerDB>
where
    InnerDB: DatabaseRef + Default,
{
    /// Get all account addresses and storage keys during the execution of a block,
    /// ignoring newly created accounts.
    ///
    /// Generally used for building an [super::ExecutionDB].
    /// Executes a block retrieving
    pub fn exec(
        block: &Block,
        chain_id: u64,
        spec_id: SpecId,
    ) -> Result<Self, RevmError<InnerDB::Error>> {
        let block_env = block_env(&block.header);
        let mut db = PreExecDB::default();

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
            let mut evm = evm_builder.with_ref_db(&mut db).build();
            evm.transact_commit()?;
        }

        // add withdrawal accounts
        if let Some(ref withdrawals) = block.body.withdrawals {
            for withdrawal in withdrawals {
                db.basic_ref(RevmAddress::from_slice(withdrawal.address.as_bytes()))
                    .map_err(RevmError::Database)?;
            }
        }

        Ok(db)
    }
}

impl<InnerDB: DatabaseRef> From<PreExecDB<InnerDB>> for ExecutionDB {
    fn from(value: PreExecDB<InnerDB>) -> Self {
        let read_accounts =
            value
                .read_accounts
                .into_inner()
                .into_iter()
                .filter_map(|(address, account)| {
                    if let Some(account) = account {
                        Some((address, account))
                    } else {
                        None
                    }
                });
        let accounts = value
            .written_accounts
            .into_iter()
            .chain(read_accounts)
            .collect();
        let code = value.code.into_inner();
        let storage = value.storage.into_inner();
        let block_hashes = value.block_hashes.into_inner();

        Self {
            accounts,
            code,
            storage,
            block_hashes,
            chain_config,
            pruned_state_trie,
            pruned_storage_tries,
        }
    }
}
