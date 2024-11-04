use self::engines::in_memory::Store as InMemoryStore;
#[cfg(feature = "libmdbx")]
use self::engines::libmdbx::Store as LibmdbxStore;
use self::error::StoreError;
use bytes::Bytes;
use engines::api::StoreEngine;
use ethereum_rust_core::types::{
    code_hash, AccountInfo, AccountState, BlobsBundle, Block, BlockBody, BlockHash, BlockHeader,
    BlockNumber, ChainConfig, Genesis, GenesisAccount, Index, MempoolTransaction, Receipt,
    Transaction, TxType, EMPTY_TRIE_HASH,
};
use ethereum_rust_rlp::decode::RLPDecode;
use ethereum_rust_rlp::encode::RLPEncode;
use ethereum_rust_trie::Trie;
use ethereum_types::{Address, H256, U256};
use sha3::{Digest as _, Keccak256};
use std::collections::HashMap;
use std::fmt::Debug;
use std::sync::{Arc, Mutex};
use tracing::info;

mod engines;
pub mod error;
mod rlp;

#[derive(Debug, Clone)]
pub struct Store {
    // TODO: Check if we can remove this mutex and move it to the in_memory::Store struct
    engine: Arc<dyn StoreEngine>,
    pub mempool: Arc<Mutex<HashMap<H256, MempoolTransaction>>>,
    pub blobs_bundle_pool: Arc<Mutex<HashMap<H256, BlobsBundle>>>,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Copy)]
pub enum EngineType {
    InMemory,
    #[cfg(feature = "libmdbx")]
    Libmdbx,
}

#[derive(Default, Debug)]
pub struct AccountUpdate {
    pub address: Address,
    pub removed: bool,
    pub info: Option<AccountInfo>,
    pub code: Option<Bytes>,
    pub added_storage: HashMap<H256, U256>,
    // Matches TODO in code
    // removed_storage_keys: Vec<H256>,
}

impl AccountUpdate {
    /// Creates new empty update for the given account
    pub fn new(address: Address) -> AccountUpdate {
        AccountUpdate {
            address,
            ..Default::default()
        }
    }

    /// Creates new update representing an account removal
    pub fn removed(address: Address) -> AccountUpdate {
        AccountUpdate {
            address,
            removed: true,
            ..Default::default()
        }
    }
}

impl Store {
    pub fn new(path: &str, engine_type: EngineType) -> Result<Self, StoreError> {
        info!("Starting storage engine ({engine_type:?})");
        let store = match engine_type {
            #[cfg(feature = "libmdbx")]
            EngineType::Libmdbx => Self {
                engine: Arc::new(LibmdbxStore::new(path)?),
                mempool: Arc::new(Mutex::new(HashMap::new())),
                blobs_bundle_pool: Arc::new(Mutex::new(HashMap::new())),
            },
            EngineType::InMemory => Self {
                engine: Arc::new(InMemoryStore::new()),
                mempool: Arc::new(Mutex::new(HashMap::new())),
                blobs_bundle_pool: Arc::new(Mutex::new(HashMap::new())),
            },
        };
        info!("Started store engine");
        Ok(store)
    }

    pub fn get_account_info(
        &self,
        block_number: BlockNumber,
        address: Address,
    ) -> Result<Option<AccountInfo>, StoreError> {
        match self.get_canonical_block_hash(block_number)? {
            Some(block_hash) => self.get_account_info_by_hash(block_hash, address),
            None => Ok(None),
        }
    }

    pub fn get_account_info_by_hash(
        &self,
        block_hash: BlockHash,
        address: Address,
    ) -> Result<Option<AccountInfo>, StoreError> {
        let Some(state_trie) = self.state_trie(block_hash)? else {
            return Ok(None);
        };
        let hashed_address = hash_address(&address);
        let Some(encoded_state) = state_trie.get(&hashed_address)? else {
            return Ok(None);
        };
        let account_state = AccountState::decode(&encoded_state)?;
        Ok(Some(AccountInfo {
            code_hash: account_state.code_hash,
            balance: account_state.balance,
            nonce: account_state.nonce,
        }))
    }

    pub fn add_block_header(
        &self,
        block_hash: BlockHash,
        block_header: BlockHeader,
    ) -> Result<(), StoreError> {
        self.engine.add_block_header(block_hash, block_header)
    }

    pub fn get_block_header(
        &self,
        block_number: BlockNumber,
    ) -> Result<Option<BlockHeader>, StoreError> {
        self.engine.get_block_header(block_number)
    }

    pub fn get_block_header_by_hash(
        &self,
        block_hash: BlockHash,
    ) -> Result<Option<BlockHeader>, StoreError> {
        self.engine.get_block_header_by_hash(block_hash)
    }

    pub fn get_block_body_by_hash(
        &self,
        block_hash: BlockHash,
    ) -> Result<Option<BlockBody>, StoreError> {
        self.engine.get_block_body_by_hash(block_hash)
    }

    pub fn add_block_body(
        &self,
        block_hash: BlockHash,
        block_body: BlockBody,
    ) -> Result<(), StoreError> {
        self.engine.add_block_body(block_hash, block_body)
    }

    pub fn get_block_body(
        &self,
        block_number: BlockNumber,
    ) -> Result<Option<BlockBody>, StoreError> {
        self.engine.get_block_body(block_number)
    }

    pub fn add_pending_block(&self, block: Block) -> Result<(), StoreError> {
        info!(
            "Adding block to pending: {}",
            block.header.compute_block_hash()
        );
        self.engine.add_pending_block(block)
    }

    pub fn get_pending_block(&self, block_hash: BlockHash) -> Result<Option<Block>, StoreError> {
        info!("get pending: {}", block_hash);
        self.engine.get_pending_block(block_hash)
    }

    pub fn add_block_number(
        &self,
        block_hash: BlockHash,
        block_number: BlockNumber,
    ) -> Result<(), StoreError> {
        self.engine
            .clone()
            .add_block_number(block_hash, block_number)
    }

    pub fn get_block_number(
        &self,
        block_hash: BlockHash,
    ) -> Result<Option<BlockNumber>, StoreError> {
        self.engine.get_block_number(block_hash)
    }

    pub fn add_block_total_difficulty(
        &self,
        block_hash: BlockHash,
        block_difficulty: U256,
    ) -> Result<(), StoreError> {
        self.engine
            .add_block_total_difficulty(block_hash, block_difficulty)
    }

    pub fn get_block_total_difficulty(
        &self,
        block_hash: BlockHash,
    ) -> Result<Option<U256>, StoreError> {
        self.engine.get_block_total_difficulty(block_hash)
    }

    pub fn add_transaction_location(
        &self,
        transaction_hash: H256,
        block_number: BlockNumber,
        block_hash: BlockHash,
        index: Index,
    ) -> Result<(), StoreError> {
        self.engine
            .add_transaction_location(transaction_hash, block_number, block_hash, index)
    }

    pub fn get_transaction_location(
        &self,
        transaction_hash: H256,
    ) -> Result<Option<(BlockNumber, BlockHash, Index)>, StoreError> {
        self.engine.get_transaction_location(transaction_hash)
    }

    /// Add transaction to the pool
    pub fn add_transaction_to_pool(
        &self,
        hash: H256,
        transaction: MempoolTransaction,
    ) -> Result<(), StoreError> {
        let mut mempool = self
            .mempool
            .lock()
            .map_err(|error| StoreError::Custom(error.to_string()))?;
        mempool.insert(hash, transaction);

        Ok(())
    }

    /// Add a blobs bundle to the pool by its blob transaction hash
    pub fn add_blobs_bundle_to_pool(
        &self,
        tx_hash: H256,
        blobs_bundle: BlobsBundle,
    ) -> Result<(), StoreError> {
        self.blobs_bundle_pool
            .lock()
            .map_err(|error| StoreError::Custom(error.to_string()))?
            .insert(tx_hash, blobs_bundle);
        Ok(())
    }

    /// Get a blobs bundle to the pool given its blob transaction hash
    pub fn get_blobs_bundle_from_pool(
        &self,
        tx_hash: H256,
    ) -> Result<Option<BlobsBundle>, StoreError> {
        Ok(self
            .blobs_bundle_pool
            .lock()
            .map_err(|error| StoreError::Custom(error.to_string()))?
            .get(&tx_hash)
            .cloned())
    }

    /// Remove a transaction from the pool
    pub fn remove_transaction_from_pool(&self, hash: &H256) -> Result<(), StoreError> {
        let mut mempool = self
            .mempool
            .lock()
            .map_err(|error| StoreError::Custom(error.to_string()))?;
        if let Some(tx) = mempool.get(hash) {
            if matches!(tx.tx_type(), TxType::EIP4844) {
                self.blobs_bundle_pool
                    .lock()
                    .map_err(|error| StoreError::Custom(error.to_string()))?
                    .remove(&tx.compute_hash());
            }

            mempool.remove(hash);
        };

        Ok(())
    }

    /// Applies the filter and returns a set of suitable transactions from the mempool.
    /// These transactions will be grouped by sender and sorted by nonce
    pub fn filter_pool_transactions(
        &self,
        filter: &dyn Fn(&Transaction) -> bool,
    ) -> Result<HashMap<Address, Vec<MempoolTransaction>>, StoreError> {
        let mut txs_by_sender: HashMap<Address, Vec<MempoolTransaction>> = HashMap::new();
        let mempool = self
            .mempool
            .lock()
            .map_err(|error| StoreError::Custom(error.to_string()))?;

        for (_, tx) in mempool.iter() {
            if filter(tx) {
                txs_by_sender
                    .entry(tx.sender())
                    .or_default()
                    .push(tx.clone())
            }
        }

        txs_by_sender.iter_mut().for_each(|(_, txs)| txs.sort());
        Ok(txs_by_sender)
    }

    fn add_account_code(&self, code_hash: H256, code: Bytes) -> Result<(), StoreError> {
        self.engine.add_account_code(code_hash, code)
    }

    pub fn get_account_code(&self, code_hash: H256) -> Result<Option<Bytes>, StoreError> {
        self.engine.get_account_code(code_hash)
    }

    pub fn get_code_by_account_address(
        &self,
        block_number: BlockNumber,
        address: Address,
    ) -> Result<Option<Bytes>, StoreError> {
        let Some(block_hash) = self.engine.get_canonical_block_hash(block_number)? else {
            return Ok(None);
        };
        let Some(state_trie) = self.state_trie(block_hash)? else {
            return Ok(None);
        };
        let hashed_address = hash_address(&address);
        let Some(encoded_state) = state_trie.get(&hashed_address)? else {
            return Ok(None);
        };
        let account_state = AccountState::decode(&encoded_state)?;
        self.get_account_code(account_state.code_hash)
    }
    pub fn get_nonce_by_account_address(
        &self,
        block_number: BlockNumber,
        address: Address,
    ) -> Result<Option<u64>, StoreError> {
        let Some(block_hash) = self.engine.get_canonical_block_hash(block_number)? else {
            return Ok(None);
        };
        let Some(state_trie) = self.state_trie(block_hash)? else {
            return Ok(None);
        };
        let hashed_address = hash_address(&address);
        let Some(encoded_state) = state_trie.get(&hashed_address)? else {
            return Ok(None);
        };
        let account_state = AccountState::decode(&encoded_state)?;
        Ok(Some(account_state.nonce))
    }

    /// Applies account updates based on the block's latest storage state
    /// and returns the new state root after the updates have been applied.
    pub fn apply_account_updates(
        &self,
        block_hash: BlockHash,
        account_updates: &[AccountUpdate],
    ) -> Result<Option<H256>, StoreError> {
        let Some(mut state_trie) = self.state_trie(block_hash)? else {
            return Ok(None);
        };
        for update in account_updates.iter() {
            let hashed_address = hash_address(&update.address);
            if update.removed {
                // Remove account from trie
                state_trie.remove(hashed_address)?;
            } else {
                // Add or update AccountState in the trie
                // Fetch current state or create a new state to be inserted
                let mut account_state = match state_trie.get(&hashed_address)? {
                    Some(encoded_state) => AccountState::decode(&encoded_state)?,
                    None => AccountState::default(),
                };
                if let Some(info) = &update.info {
                    account_state.nonce = info.nonce;
                    account_state.balance = info.balance;
                    account_state.code_hash = info.code_hash;
                    // Store updated code in DB
                    if let Some(code) = &update.code {
                        self.add_account_code(info.code_hash, code.clone())?;
                    }
                }
                // Store the added storage in the account's storage trie and compute its new root
                if !update.added_storage.is_empty() {
                    let mut storage_trie = self
                        .engine
                        .open_storage_trie(update.address, account_state.storage_root);
                    for (storage_key, storage_value) in &update.added_storage {
                        let hashed_key = hash_key(storage_key);
                        if storage_value.is_zero() {
                            storage_trie.remove(hashed_key)?;
                        } else {
                            storage_trie.insert(hashed_key, storage_value.encode_to_vec())?;
                        }
                    }
                    account_state.storage_root = storage_trie.hash()?;
                }
                state_trie.insert(hashed_address, account_state.encode_to_vec())?;
            }
        }
        Ok(Some(state_trie.hash()?))
    }

    /// Adds all genesis accounts and returns the genesis block's state_root
    pub fn setup_genesis_state_trie(
        &self,
        genesis_accounts: HashMap<Address, GenesisAccount>,
    ) -> Result<H256, StoreError> {
        let mut genesis_state_trie = self.engine.open_state_trie(*EMPTY_TRIE_HASH);
        for (address, account) in genesis_accounts {
            // Store account code (as this won't be stored in the trie)
            let code_hash = code_hash(&account.code);
            self.add_account_code(code_hash, account.code)?;
            // Store the account's storage in a clean storage trie and compute its root
            let mut storage_trie = self.engine.open_storage_trie(address, *EMPTY_TRIE_HASH);
            for (storage_key, storage_value) in account.storage {
                if !storage_value.is_zero() {
                    let hashed_key = hash_key(&storage_key);
                    storage_trie.insert(hashed_key, storage_value.encode_to_vec())?;
                }
            }
            let storage_root = storage_trie.hash()?;
            // Add account to trie
            let account_state = AccountState {
                nonce: account.nonce,
                balance: account.balance,
                storage_root,
                code_hash,
            };
            let hashed_address = hash_address(&address);
            genesis_state_trie.insert(hashed_address, account_state.encode_to_vec())?;
        }
        Ok(genesis_state_trie.hash()?)
    }

    pub fn add_receipt(
        &self,
        block_hash: BlockHash,
        index: Index,
        receipt: Receipt,
    ) -> Result<(), StoreError> {
        self.engine.add_receipt(block_hash, index, receipt)
    }

    pub fn get_receipt(
        &self,
        block_number: BlockNumber,
        index: Index,
    ) -> Result<Option<Receipt>, StoreError> {
        self.engine.get_receipt(block_number, index)
    }

    pub fn add_block(&self, block: Block) -> Result<(), StoreError> {
        // TODO Maybe add both in a single tx?
        let header = block.header;
        let number = header.number;
        let latest_total_difficulty = self.get_latest_total_difficulty()?;
        let block_total_difficulty =
            latest_total_difficulty.unwrap_or(U256::zero()) + header.difficulty;
        let hash = header.compute_block_hash();
        self.add_transaction_locations(&block.body.transactions, number, hash)?;
        self.add_block_body(hash, block.body)?;
        self.add_block_header(hash, header)?;
        self.add_block_number(hash, number)?;
        self.add_block_total_difficulty(hash, block_total_difficulty)?;
        self.update_latest_total_difficulty(block_total_difficulty)
    }

    fn add_transaction_locations(
        &self,
        transactions: &[Transaction],
        block_number: BlockNumber,
        block_hash: BlockHash,
    ) -> Result<(), StoreError> {
        for (index, transaction) in transactions.iter().enumerate() {
            self.add_transaction_location(
                transaction.compute_hash(),
                block_number,
                block_hash,
                index as Index,
            )?;
        }
        Ok(())
    }

    pub fn add_initial_state(&self, genesis: Genesis) -> Result<(), StoreError> {
        info!("Storing initial state from genesis");

        // Obtain genesis block
        let genesis_block = genesis.get_block();
        let genesis_block_number = genesis_block.header.number;

        let genesis_hash = genesis_block.hash();

        if let Some(header) = self.get_block_header(genesis_block_number)? {
            if header.compute_block_hash() == genesis_hash {
                info!("Received genesis file matching a previously stored one, nothing to do");
                return Ok(());
            } else {
                panic!("tried to run genesis twice with different blocks");
            }
        }
        // Store genesis accounts
        // TODO: Should we use this root instead of computing it before the block hash check?
        let genesis_state_root = self.setup_genesis_state_trie(genesis.alloc)?;
        debug_assert_eq!(genesis_state_root, genesis_block.header.state_root);

        // Store genesis block
        info!(
            "Storing genesis block with number {} and hash {}",
            genesis_block_number, genesis_hash
        );

        self.add_block(genesis_block)?;
        self.update_earliest_block_number(genesis_block_number)?;
        self.update_latest_block_number(genesis_block_number)?;
        self.set_canonical_block(genesis_block_number, genesis_hash)?;

        // Set chain config
        self.set_chain_config(&genesis.config)
    }

    pub fn get_transaction_by_hash(
        &self,
        transaction_hash: H256,
    ) -> Result<Option<Transaction>, StoreError> {
        self.engine.get_transaction_by_hash(transaction_hash)
    }

    pub fn get_transaction_by_location(
        &self,
        block_hash: BlockHash,
        index: u64,
    ) -> Result<Option<Transaction>, StoreError> {
        self.engine.get_transaction_by_location(block_hash, index)
    }

    pub fn get_block_by_hash(&self, block_hash: H256) -> Result<Option<Block>, StoreError> {
        self.engine.get_block_by_hash(block_hash)
    }

    pub fn get_storage_at(
        &self,
        block_number: BlockNumber,
        address: Address,
        storage_key: H256,
    ) -> Result<Option<U256>, StoreError> {
        match self.get_canonical_block_hash(block_number)? {
            Some(block_hash) => self.get_storage_at_hash(block_hash, address, storage_key),
            None => Ok(None),
        }
    }

    pub fn get_storage_at_hash(
        &self,
        block_hash: BlockHash,
        address: Address,
        storage_key: H256,
    ) -> Result<Option<U256>, StoreError> {
        let Some(storage_trie) = self.storage_trie(block_hash, address)? else {
            return Ok(None);
        };
        let hashed_key = hash_key(&storage_key);
        storage_trie
            .get(&hashed_key)?
            .map(|rlp| U256::decode(&rlp).map_err(StoreError::RLPDecode))
            .transpose()
    }

    pub fn set_chain_config(&self, chain_config: &ChainConfig) -> Result<(), StoreError> {
        self.engine.set_chain_config(chain_config)
    }

    pub fn get_chain_config(&self) -> Result<ChainConfig, StoreError> {
        self.engine.get_chain_config()
    }

    pub fn update_earliest_block_number(
        &self,
        block_number: BlockNumber,
    ) -> Result<(), StoreError> {
        self.engine.update_earliest_block_number(block_number)
    }

    // TODO(#790): This should not return an option.
    pub fn get_earliest_block_number(&self) -> Result<Option<BlockNumber>, StoreError> {
        self.engine.get_earliest_block_number()
    }

    pub fn update_finalized_block_number(
        &self,
        block_number: BlockNumber,
    ) -> Result<(), StoreError> {
        self.engine.update_finalized_block_number(block_number)
    }

    pub fn get_finalized_block_number(&self) -> Result<Option<BlockNumber>, StoreError> {
        self.engine.get_finalized_block_number()
    }

    pub fn update_safe_block_number(&self, block_number: BlockNumber) -> Result<(), StoreError> {
        self.engine.update_safe_block_number(block_number)
    }

    pub fn get_safe_block_number(&self) -> Result<Option<BlockNumber>, StoreError> {
        self.engine.get_safe_block_number()
    }

    pub fn update_latest_block_number(&self, block_number: BlockNumber) -> Result<(), StoreError> {
        self.engine.update_latest_block_number(block_number)
    }

    // TODO(#790): This should not return an option.
    pub fn get_latest_block_number(&self) -> Result<Option<BlockNumber>, StoreError> {
        self.engine.get_latest_block_number()
    }

    pub fn update_latest_total_difficulty(&self, block_difficulty: U256) -> Result<(), StoreError> {
        self.engine.update_latest_total_difficulty(block_difficulty)
    }

    pub fn get_latest_total_difficulty(&self) -> Result<Option<U256>, StoreError> {
        self.engine.get_latest_total_difficulty()
    }

    pub fn update_pending_block_number(&self, block_number: BlockNumber) -> Result<(), StoreError> {
        self.engine.update_pending_block_number(block_number)
    }

    pub fn get_pending_block_number(&self) -> Result<Option<BlockNumber>, StoreError> {
        self.engine.get_pending_block_number()
    }

    pub fn set_canonical_block(
        &self,
        number: BlockNumber,
        hash: BlockHash,
    ) -> Result<(), StoreError> {
        self.engine.set_canonical_block(number, hash)
    }

    pub fn get_canonical_block_hash(
        &self,
        block_number: BlockNumber,
    ) -> Result<Option<BlockHash>, StoreError> {
        self.engine.get_canonical_block_hash(block_number)
    }

    /// Marks a block number as not having any canonical blocks associated with it.
    /// Used for reorgs.
    /// Note: Should we also remove all others up to the head here?
    pub fn unset_canonical_block(&self, number: BlockNumber) -> Result<(), StoreError> {
        self.engine.unset_canonical_block(number)
    }

    // Obtain the storage trie for the given block
    pub fn state_trie(&self, block_hash: BlockHash) -> Result<Option<Trie>, StoreError> {
        let Some(header) = self.get_block_header_by_hash(block_hash)? else {
            return Ok(None);
        };
        Ok(Some(self.engine.open_state_trie(header.state_root)))
    }

    // Obtain the storage trie for the given account on the given block
    fn storage_trie(
        &self,
        block_hash: BlockHash,
        address: Address,
    ) -> Result<Option<Trie>, StoreError> {
        // Fetch Account from state_trie
        let Some(state_trie) = self.state_trie(block_hash)? else {
            return Ok(None);
        };
        let hashed_address = hash_address(&address);
        let Some(encoded_account) = state_trie.get(&hashed_address)? else {
            return Ok(None);
        };
        let account = AccountState::decode(&encoded_account)?;
        // Open storage_trie
        let storage_root = account.storage_root;
        Ok(Some(self.engine.open_storage_trie(address, storage_root)))
    }

    pub fn get_account_state(
        &self,
        block_number: BlockNumber,
        address: Address,
    ) -> Result<Option<AccountState>, StoreError> {
        let Some(block_hash) = self.engine.get_canonical_block_hash(block_number)? else {
            return Ok(None);
        };
        let Some(state_trie) = self.state_trie(block_hash)? else {
            return Ok(None);
        };
        let hashed_address = hash_address(&address);
        let Some(encoded_state) = state_trie.get(&hashed_address)? else {
            return Ok(None);
        };
        Ok(Some(AccountState::decode(&encoded_state)?))
    }

    pub fn get_account_proof(
        &self,
        block_number: BlockNumber,
        address: &Address,
    ) -> Result<Option<Vec<Vec<u8>>>, StoreError> {
        let Some(block_hash) = self.engine.get_canonical_block_hash(block_number)? else {
            return Ok(None);
        };
        let Some(state_trie) = self.state_trie(block_hash)? else {
            return Ok(None);
        };
        Ok(Some(state_trie.get_proof(&hash_address(address))).transpose()?)
    }

    /// Constructs a merkle proof for the given storage_key in a storage_trie with a known root
    pub fn get_storage_proof(
        &self,
        address: Address,
        storage_root: H256,
        storage_key: &H256,
    ) -> Result<Vec<Vec<u8>>, StoreError> {
        let trie = self.engine.open_storage_trie(address, storage_root);
        Ok(trie.get_proof(&hash_key(storage_key))?)
    }

    // Returns an iterator across all accounts in the state trie given by the state_root
    // Does not check that the state_root is valid
    pub fn iter_accounts(&self, state_root: H256) -> impl Iterator<Item = (H256, AccountState)> {
        self.engine
            .open_state_trie(state_root)
            .into_iter()
            .content()
            .map_while(|(path, value)| {
                Some((H256::from_slice(&path), AccountState::decode(&value).ok()?))
            })
    }

    pub fn get_account_range_proof(
        &self,
        state_root: H256,
        starting_hash: H256,
        last_hash: Option<H256>,
    ) -> Result<Vec<Vec<u8>>, StoreError> {
        let state_trie = self.engine.open_state_trie(state_root);
        let mut proof = state_trie.get_proof(&starting_hash.as_bytes().to_vec())?;
        if let Some(last_hash) = last_hash {
            proof.extend_from_slice(&state_trie.get_proof(&last_hash.as_bytes().to_vec())?);
        }
        Ok(proof)
    }

    pub fn add_payload(&self, payload_id: u64, block: Block) -> Result<(), StoreError> {
        self.engine.add_payload(payload_id, block)
    }

    pub fn get_payload(&self, payload_id: u64) -> Result<Option<Block>, StoreError> {
        self.engine.get_payload(payload_id)
    }

    /// Creates a new state trie with an empty state root, for testing purposes only
    pub fn new_state_trie_for_test(&self) -> Trie {
        self.engine.open_state_trie(*EMPTY_TRIE_HASH)
    }
}

fn hash_address(address: &Address) -> Vec<u8> {
    Keccak256::new_with_prefix(address.to_fixed_bytes())
        .finalize()
        .to_vec()
}

fn hash_key(key: &H256) -> Vec<u8> {
    Keccak256::new_with_prefix(key.to_fixed_bytes())
        .finalize()
        .to_vec()
}

#[cfg(test)]
mod tests {
    use std::{fs, panic, str::FromStr};

    use bytes::Bytes;
    use ethereum_rust_core::{
        types::{Transaction, TxType, BYTES_PER_BLOB},
        Bloom,
    };
    use ethereum_rust_rlp::decode::RLPDecode;
    use ethereum_types::{H256, U256};

    use super::*;

    #[test]
    fn test_in_memory_store() {
        test_store_suite(EngineType::InMemory);
    }

    #[cfg(feature = "libmdbx")]
    #[test]
    fn test_libmdbx_store() {
        test_store_suite(EngineType::Libmdbx);
    }

    // Creates an empty store, runs the test and then removes the store (if needed)
    fn run_test(test_func: &dyn Fn(Store), engine_type: EngineType) {
        // Remove preexistent DBs in case of a failed previous test
        if !matches!(engine_type, EngineType::InMemory) {
            remove_test_dbs("store-test-db");
        };
        // Build a new store
        let store = Store::new("store-test-db", engine_type).expect("Failed to create test db");
        // Run the test
        test_func(store);
        // Remove store (if needed)
        if !matches!(engine_type, EngineType::InMemory) {
            remove_test_dbs("store-test-db");
        };
    }

    fn test_store_suite(engine_type: EngineType) {
        run_test(&test_store_block, engine_type);
        run_test(&test_store_block_number, engine_type);
        run_test(&test_store_transaction_location, engine_type);
        run_test(&test_store_transaction_location_not_canonical, engine_type);
        run_test(&test_store_block_receipt, engine_type);
        run_test(&test_store_account_code, engine_type);
        run_test(&test_store_block_tags, engine_type);
        run_test(&test_chain_config_storage, engine_type);
        run_test(&test_genesis_block, engine_type);
        run_test(&test_filter_mempool_transactions, engine_type);
        run_test(&blobs_bundle_loadtest, engine_type);
    }

    fn test_genesis_block(store: Store) {
        const GENESIS_KURTOSIS: &str = include_str!("../../../test_data/genesis-kurtosis.json");
        const GENESIS_HIVE: &str = include_str!("../../../test_data/genesis-hive.json");
        assert_ne!(GENESIS_KURTOSIS, GENESIS_HIVE);
        let genesis_kurtosis: Genesis =
            serde_json::from_str(GENESIS_KURTOSIS).expect("deserialize genesis-kurtosis.json");
        let genesis_hive: Genesis =
            serde_json::from_str(GENESIS_HIVE).expect("deserialize genesis-hive.json");
        store
            .add_initial_state(genesis_kurtosis.clone())
            .expect("first genesis");
        store
            .add_initial_state(genesis_kurtosis)
            .expect("second genesis with same block");
        panic::catch_unwind(move || {
            let _ = store.add_initial_state(genesis_hive);
        })
        .expect_err("genesis with a different block should panic");
    }

    fn remove_test_dbs(path: &str) {
        // Removes all test databases from filesystem
        if std::path::Path::new(path).exists() {
            fs::remove_dir_all(path).expect("Failed to clean test db dir");
        }
    }

    fn test_store_block(store: Store) {
        let (block_header, block_body) = create_block_for_testing();
        let block_number = 6;
        let hash = block_header.compute_block_hash();

        store.add_block_header(hash, block_header.clone()).unwrap();
        store.add_block_body(hash, block_body.clone()).unwrap();
        store.set_canonical_block(block_number, hash).unwrap();

        let stored_header = store.get_block_header(block_number).unwrap().unwrap();
        let stored_body = store.get_block_body(block_number).unwrap().unwrap();

        assert_eq!(stored_header, block_header);
        assert_eq!(stored_body, block_body);
    }

    fn create_block_for_testing() -> (BlockHeader, BlockBody) {
        let block_header = BlockHeader {
            parent_hash: H256::from_str(
                "0x1ac1bf1eef97dc6b03daba5af3b89881b7ae4bc1600dc434f450a9ec34d44999",
            )
            .unwrap(),
            ommers_hash: H256::from_str(
                "0x1dcc4de8dec75d7aab85b567b6ccd41ad312451b948a7413f0a142fd40d49347",
            )
            .unwrap(),
            coinbase: Address::from_str("0x2adc25665018aa1fe0e6bc666dac8fc2697ff9ba").unwrap(),
            state_root: H256::from_str(
                "0x9de6f95cb4ff4ef22a73705d6ba38c4b927c7bca9887ef5d24a734bb863218d9",
            )
            .unwrap(),
            transactions_root: H256::from_str(
                "0x578602b2b7e3a3291c3eefca3a08bc13c0d194f9845a39b6f3bcf843d9fed79d",
            )
            .unwrap(),
            receipts_root: H256::from_str(
                "0x035d56bac3f47246c5eed0e6642ca40dc262f9144b582f058bc23ded72aa72fa",
            )
            .unwrap(),
            logs_bloom: Bloom::from([0; 256]),
            difficulty: U256::zero(),
            number: 1,
            gas_limit: 0x016345785d8a0000,
            gas_used: 0xa8de,
            timestamp: 0x03e8,
            extra_data: Bytes::new(),
            prev_randao: H256::zero(),
            nonce: 0x0000000000000000,
            base_fee_per_gas: Some(0x07),
            withdrawals_root: Some(
                H256::from_str(
                    "0x56e81f171bcc55a6ff8345e692c0f86e5b48e01b996cadc001622fb5e363b421",
                )
                .unwrap(),
            ),
            blob_gas_used: Some(0x00),
            excess_blob_gas: Some(0x00),
            parent_beacon_block_root: Some(H256::zero()),
        };
        let block_body = BlockBody {
            transactions: vec![Transaction::decode(&hex::decode("b86f02f86c8330182480114e82f618946177843db3138ae69679a54b95cf345ed759450d870aa87bee53800080c080a0151ccc02146b9b11adf516e6787b59acae3e76544fdcd75e77e67c6b598ce65da064c5dd5aae2fbb535830ebbdad0234975cd7ece3562013b63ea18cc0df6c97d4").unwrap()).unwrap(),
            Transaction::decode(&hex::decode("f86d80843baa0c4082f618946177843db3138ae69679a54b95cf345ed759450d870aa87bee538000808360306ba0151ccc02146b9b11adf516e6787b59acae3e76544fdcd75e77e67c6b598ce65da064c5dd5aae2fbb535830ebbdad0234975cd7ece3562013b63ea18cc0df6c97d4").unwrap()).unwrap()],
            ommers: Default::default(),
            withdrawals: Default::default(),
        };
        (block_header, block_body)
    }

    fn test_store_block_number(store: Store) {
        let block_hash = H256::random();
        let block_number = 6;

        store.add_block_number(block_hash, block_number).unwrap();

        let stored_number = store.get_block_number(block_hash).unwrap().unwrap();

        assert_eq!(stored_number, block_number);
    }

    fn test_store_transaction_location(store: Store) {
        let transaction_hash = H256::random();
        let block_hash = H256::random();
        let block_number = 6;
        let index = 3;

        store
            .add_transaction_location(transaction_hash, block_number, block_hash, index)
            .unwrap();

        store.set_canonical_block(block_number, block_hash).unwrap();

        let stored_location = store
            .get_transaction_location(transaction_hash)
            .unwrap()
            .unwrap();

        assert_eq!(stored_location, (block_number, block_hash, index));
    }

    fn test_store_transaction_location_not_canonical(store: Store) {
        let transaction_hash = H256::random();
        let block_hash = H256::random();
        let block_number = 6;
        let index = 3;

        store
            .add_transaction_location(transaction_hash, block_number, block_hash, index)
            .unwrap();

        store
            .set_canonical_block(block_number, H256::random())
            .unwrap();

        assert_eq!(
            store.get_transaction_location(transaction_hash).unwrap(),
            None
        )
    }

    fn test_store_block_receipt(store: Store) {
        let receipt = Receipt {
            tx_type: TxType::EIP2930,
            succeeded: true,
            cumulative_gas_used: 1747,
            bloom: Bloom::random(),
            logs: vec![],
        };
        let block_number = 6;
        let index = 4;
        let block_hash = H256::random();

        store
            .add_receipt(block_hash, index, receipt.clone())
            .unwrap();

        store.set_canonical_block(block_number, block_hash).unwrap();

        let stored_receipt = store.get_receipt(block_number, index).unwrap().unwrap();

        assert_eq!(stored_receipt, receipt);
    }

    fn test_store_account_code(store: Store) {
        let code_hash = H256::random();
        let code = Bytes::from("kiwi");

        store.add_account_code(code_hash, code.clone()).unwrap();

        let stored_code = store.get_account_code(code_hash).unwrap().unwrap();

        assert_eq!(stored_code, code);
    }

    fn test_store_block_tags(store: Store) {
        let earliest_block_number = 0;
        let finalized_block_number = 7;
        let safe_block_number = 6;
        let latest_block_number = 8;
        let pending_block_number = 9;

        store
            .update_earliest_block_number(earliest_block_number)
            .unwrap();
        store
            .update_finalized_block_number(finalized_block_number)
            .unwrap();
        store.update_safe_block_number(safe_block_number).unwrap();
        store
            .update_latest_block_number(latest_block_number)
            .unwrap();
        store
            .update_pending_block_number(pending_block_number)
            .unwrap();

        let stored_earliest_block_number = store.get_earliest_block_number().unwrap().unwrap();
        let stored_finalized_block_number = store.get_finalized_block_number().unwrap().unwrap();
        let stored_safe_block_number = store.get_safe_block_number().unwrap().unwrap();
        let stored_latest_block_number = store.get_latest_block_number().unwrap().unwrap();
        let stored_pending_block_number = store.get_pending_block_number().unwrap().unwrap();

        assert_eq!(earliest_block_number, stored_earliest_block_number);
        assert_eq!(finalized_block_number, stored_finalized_block_number);
        assert_eq!(safe_block_number, stored_safe_block_number);
        assert_eq!(latest_block_number, stored_latest_block_number);
        assert_eq!(pending_block_number, stored_pending_block_number);
    }

    fn test_chain_config_storage(store: Store) {
        let chain_config = example_chain_config();
        store.set_chain_config(&chain_config).unwrap();
        let retrieved_chain_config = store.get_chain_config().unwrap();
        assert_eq!(chain_config, retrieved_chain_config);
    }

    fn example_chain_config() -> ChainConfig {
        ChainConfig {
            chain_id: 3151908_u64,
            homestead_block: Some(0),
            eip150_block: Some(0),
            eip155_block: Some(0),
            eip158_block: Some(0),
            byzantium_block: Some(0),
            constantinople_block: Some(0),
            petersburg_block: Some(0),
            istanbul_block: Some(0),
            berlin_block: Some(0),
            london_block: Some(0),
            merge_netsplit_block: Some(0),
            shanghai_time: Some(0),
            cancun_time: Some(0),
            prague_time: Some(1718232101),
            terminal_total_difficulty: Some(58750000000000000000000),
            terminal_total_difficulty_passed: true,
            ..Default::default()
        }
    }

    use hex_literal::hex;

    fn test_filter_mempool_transactions(store: Store) {
        let plain_tx = MempoolTransaction::new(Transaction::decode_canonical(&hex!("f86d80843baa0c4082f618946177843db3138ae69679a54b95cf345ed759450d870aa87bee538000808360306ba0151ccc02146b9b11adf516e6787b59acae3e76544fdcd75e77e67c6b598ce65da064c5dd5aae2fbb535830ebbdad0234975cd7ece3562013b63ea18cc0df6c97d4")).unwrap());
        let blob_tx = MempoolTransaction::new(Transaction::decode_canonical(&hex!("03f88f0780843b9aca008506fc23ac00830186a09400000000000000000000000000000000000001008080c001e1a0010657f37554c781402a22917dee2f75def7ab966d7b770905398eba3c44401401a0840650aa8f74d2b07f40067dc33b715078d73422f01da17abdbd11e02bbdfda9a04b2260f6022bf53eadb337b3e59514936f7317d872defb891a708ee279bdca90")).unwrap());
        let plain_tx_hash = plain_tx.compute_hash();
        let blob_tx_hash = blob_tx.compute_hash();
        let filter =
            |tx: &Transaction| -> bool { matches!(tx, Transaction::EIP4844Transaction(_)) };
        store
            .add_transaction_to_pool(blob_tx_hash, blob_tx.clone())
            .unwrap();
        store
            .add_transaction_to_pool(plain_tx_hash, plain_tx)
            .unwrap();
        let txs = store.filter_pool_transactions(&filter).unwrap();
        assert_eq!(txs, HashMap::from([(blob_tx.sender(), vec![blob_tx])]));
    }

    fn blobs_bundle_loadtest(store: Store) {
        // Write a bundle of 6 blobs 10 times
        // If this test fails please adjust the max_size in the DB config
        for i in 0..300 {
            let blobs = [[i as u8; BYTES_PER_BLOB]; 6];
            let commitments = [[i as u8; 48]; 6];
            let proofs = [[i as u8; 48]; 6];
            let bundle = BlobsBundle {
                blobs: blobs.to_vec(),
                commitments: commitments.to_vec(),
                proofs: proofs.to_vec(),
            };
            store
                .add_blobs_bundle_to_pool(H256::random(), bundle)
                .unwrap();
        }
    }
}
