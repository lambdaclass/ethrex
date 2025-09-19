use crate::v2::api::StorageBackend;
#[cfg(feature = "rocksdb")]
use crate::v2::backend::rocksdb::RocksDBBackend;
use crate::v2::store_v2::StoreV2;
use crate::{error::StoreError, v2::backend::in_memory::InMemoryBackend};
use bytes::Bytes;
use ethereum_types::{Address, H256, U256};
use ethrex_common::{
    constants::EMPTY_TRIE_HASH,
    types::{
        AccountInfo, AccountState, AccountUpdate, Block, BlockBody, BlockHash, BlockHeader,
        BlockNumber, ChainConfig, ForkId, Genesis, GenesisAccount, Index, Receipt, Transaction,
        code_hash,
    },
};
use ethrex_rlp::decode::RLPDecode;
use ethrex_rlp::encode::RLPEncode;
use ethrex_trie::{Nibbles, NodeHash, Trie, TrieLogger, TrieNode, TrieWitness};
use sha3::{Digest as _, Keccak256};
use std::fmt::Debug;
use std::sync::Arc;
use std::{
    collections::{BTreeMap, HashMap},
    sync::RwLock,
};
use tracing::{debug, error, info, instrument};
/// Number of state trie segments to fetch concurrently during state sync
pub const STATE_TRIE_SEGMENTS: usize = 2;
/// Maximum amount of reads from the snapshot in a single transaction to avoid performance hits due to long-living reads
/// This will always be the amount yielded by snapshot reads unless there are less elements left
pub const MAX_SNAPSHOT_READS: usize = 100;

#[derive(Debug, Clone)]
pub struct Store {
    // FIXME: Use StorageBackend
    engine: Arc<StoreV2>,
    chain_config: Arc<RwLock<ChainConfig>>,
    latest_block_header: Arc<RwLock<BlockHeader>>,
}

pub type StorageTrieNodes = Vec<(H256, Vec<(NodeHash, Vec<u8>)>)>;

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EngineType {
    InMemory,
    #[cfg(feature = "libmdbx")]
    Libmdbx,
    #[cfg(feature = "rocksdb")]
    RocksDB,
}

pub struct UpdateBatch {
    /// Nodes to be added to the state trie
    pub account_updates: Vec<TrieNode>,
    /// Storage tries updated and their new nodes
    pub storage_updates: Vec<(H256, Vec<TrieNode>)>,
    /// Blocks to be added
    pub blocks: Vec<Block>,
    /// Receipts added per block
    pub receipts: Vec<(H256, Vec<Receipt>)>,
    /// Code updates
    pub code_updates: Vec<(H256, Bytes)>,
}

type StorageUpdates = Vec<(H256, Vec<(NodeHash, Vec<u8>)>)>;

pub struct AccountUpdatesList {
    pub state_trie_hash: H256,
    pub state_updates: Vec<(NodeHash, Vec<u8>)>,
    pub storage_updates: StorageUpdates,
    pub code_updates: Vec<(H256, Bytes)>,
}

impl Store {
    pub async fn store_block_updates(&self, update_batch: UpdateBatch) -> Result<(), StoreError> {
        self.engine.apply_updates(update_batch).await
    }

    pub fn new(path: &str, engine_type: EngineType) -> Result<Self, StoreError> {
        info!(engine = ?engine_type, path = %path, "Opening storage engine");
        let store = match engine_type {
            #[cfg(feature = "rocksdb")]
            EngineType::RocksDB => Self {
                engine: Arc::new(StoreV2::new(RocksDBBackend::open(path)?)?),
                chain_config: Default::default(),
                latest_block_header: Arc::new(RwLock::new(BlockHeader::default())),
            },
            #[cfg(feature = "libmdbx")]
            EngineType::Libmdbx => todo!(),
            EngineType::InMemory => Self {
                engine: Arc::new(StoreV2::new(InMemoryBackend::open(path)?)?),
                chain_config: Default::default(),
                latest_block_header: Arc::new(RwLock::new(BlockHeader::default())),
            },
        };

        Ok(store)
    }

    pub async fn new_from_genesis(
        store_path: &str,
        engine_type: EngineType,
        genesis_path: &str,
    ) -> Result<Self, StoreError> {
        let file = std::fs::File::open(genesis_path)
            .map_err(|error| StoreError::Custom(format!("Failed to open genesis file: {error}")))?;
        let reader = std::io::BufReader::new(file);
        let genesis: Genesis =
            serde_json::from_reader(reader).expect("Failed to deserialize genesis file");
        let store = Self::new(store_path, engine_type)?;
        store.add_initial_state(genesis).await?;
        Ok(store)
    }

    pub async fn get_account_info(
        &self,
        block_number: BlockNumber,
        address: Address,
    ) -> Result<Option<AccountInfo>, StoreError> {
        match self.get_canonical_block_hash(block_number).await? {
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

    pub fn get_account_state_by_acc_hash(
        &self,
        block_hash: BlockHash,
        account_hash: H256,
    ) -> Result<Option<AccountState>, StoreError> {
        let Some(state_trie) = self.state_trie(block_hash)? else {
            return Ok(None);
        };
        let Some(encoded_state) = state_trie.get(&account_hash.to_fixed_bytes().to_vec())? else {
            return Ok(None);
        };
        let account_state = AccountState::decode(&encoded_state)?;
        Ok(Some(account_state))
    }

    pub async fn add_block_header(
        &self,
        block_hash: BlockHash,
        block_header: BlockHeader,
    ) -> Result<(), StoreError> {
        self.engine.add_block_header(block_hash, block_header).await
    }

    pub async fn add_block_headers(
        &self,
        block_headers: Vec<BlockHeader>,
    ) -> Result<(), StoreError> {
        self.engine.add_block_headers(block_headers).await
    }

    pub fn get_block_header(
        &self,
        block_number: BlockNumber,
    ) -> Result<Option<BlockHeader>, StoreError> {
        let latest = self
            .latest_block_header
            .read()
            .map_err(|_| StoreError::LockError)?
            .clone();
        if block_number == latest.number {
            return Ok(Some(latest));
        }
        self.engine.get_block_header(block_number)
    }

    pub fn get_block_header_by_hash(
        &self,
        block_hash: BlockHash,
    ) -> Result<Option<BlockHeader>, StoreError> {
        {
            let latest = self
                .latest_block_header
                .read()
                .map_err(|_| StoreError::LockError)?;
            if block_hash == latest.hash() {
                return Ok(Some(latest.clone()));
            }
        }

        self.engine.get_block_header_by_hash(block_hash)
    }

    pub async fn get_block_body_by_hash(
        &self,
        block_hash: BlockHash,
    ) -> Result<Option<BlockBody>, StoreError> {
        self.engine.get_block_body_by_hash(block_hash).await
    }

    pub async fn add_block_body(
        &self,
        block_hash: BlockHash,
        block_body: BlockBody,
    ) -> Result<(), StoreError> {
        self.engine.add_block_body(block_hash, block_body).await
    }

    pub async fn get_block_body(
        &self,
        block_number: BlockNumber,
    ) -> Result<Option<BlockBody>, StoreError> {
        // FIXME (#4353)
        let latest = self
            .latest_block_header
            .read()
            .map_err(|_| StoreError::LockError)?
            .clone();
        if block_number == latest.number {
            // The latest may not be marked as canonical yet
            return self.engine.get_block_body_by_hash(latest.hash()).await;
        }
        self.engine.get_block_body(block_number).await
    }

    pub async fn remove_block(&self, block_number: BlockNumber) -> Result<(), StoreError> {
        self.engine.remove_block(block_number).await
    }

    pub async fn get_block_bodies(
        &self,
        from: BlockNumber,
        to: BlockNumber,
    ) -> Result<Vec<BlockBody>, StoreError> {
        self.engine.get_block_bodies(from, to).await
    }

    pub async fn get_block_bodies_by_hash(
        &self,
        hashes: Vec<BlockHash>,
    ) -> Result<Vec<BlockBody>, StoreError> {
        self.engine.get_block_bodies_by_hash(hashes).await
    }

    pub async fn add_pending_block(&self, block: Block) -> Result<(), StoreError> {
        info!("Adding block to pending: {}", block.hash());
        self.engine.add_pending_block(block).await
    }

    pub async fn get_pending_block(
        &self,
        block_hash: BlockHash,
    ) -> Result<Option<Block>, StoreError> {
        info!("get pending: {}", block_hash);
        self.engine.get_pending_block(block_hash).await
    }

    pub async fn add_block_number(
        &self,
        block_hash: BlockHash,
        block_number: BlockNumber,
    ) -> Result<(), StoreError> {
        self.engine
            .clone()
            .add_block_number(block_hash, block_number)
            .await
    }

    pub async fn get_block_number(
        &self,
        block_hash: BlockHash,
    ) -> Result<Option<BlockNumber>, StoreError> {
        self.engine.get_block_number(block_hash).await
    }

    pub async fn get_fork_id(&self) -> Result<ForkId, StoreError> {
        let chain_config = self.get_chain_config()?;
        let genesis_header = self
            .engine
            .get_block_header(0)?
            .ok_or(StoreError::MissingEarliestBlockNumber)?;
        let block_number = self.get_latest_block_number().await?;
        let block_header = self
            .get_block_header(block_number)?
            .ok_or(StoreError::MissingLatestBlockNumber)?;

        Ok(ForkId::new(
            chain_config,
            genesis_header,
            block_header.timestamp,
            block_number,
        ))
    }

    pub async fn add_transaction_location(
        &self,
        transaction_hash: H256,
        block_number: BlockNumber,
        block_hash: BlockHash,
        index: Index,
    ) -> Result<(), StoreError> {
        self.engine
            .add_transaction_location(transaction_hash, block_number, block_hash, index)
            .await
    }

    pub async fn add_transaction_locations(
        &self,
        transactions: &[Transaction],
        block_number: BlockNumber,
        block_hash: BlockHash,
    ) -> Result<(), StoreError> {
        let mut locations = vec![];

        for (index, transaction) in transactions.iter().enumerate() {
            locations.push((transaction.hash(), block_number, block_hash, index as Index));
        }

        self.engine.add_transaction_locations(locations).await
    }

    pub async fn get_transaction_location(
        &self,
        transaction_hash: H256,
    ) -> Result<Option<(BlockNumber, BlockHash, Index)>, StoreError> {
        self.engine.get_transaction_location(transaction_hash).await
    }

    pub async fn add_account_code(&self, code_hash: H256, code: Bytes) -> Result<(), StoreError> {
        self.engine.add_account_code(code_hash, code).await
    }

    pub fn get_account_code(&self, code_hash: H256) -> Result<Option<Bytes>, StoreError> {
        self.engine.get_account_code(code_hash)
    }

    pub async fn get_code_by_account_address(
        &self,
        block_number: BlockNumber,
        address: Address,
    ) -> Result<Option<Bytes>, StoreError> {
        let Some(block_hash) = self.get_canonical_block_hash(block_number).await? else {
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

    pub async fn get_nonce_by_account_address(
        &self,
        block_number: BlockNumber,
        address: Address,
    ) -> Result<Option<u64>, StoreError> {
        let Some(block_hash) = self.get_canonical_block_hash(block_number).await? else {
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
    #[instrument(level = "trace", name = "Trie update", skip_all)]
    pub async fn apply_account_updates_batch(
        &self,
        block_hash: BlockHash,
        account_updates: &[AccountUpdate],
    ) -> Result<Option<AccountUpdatesList>, StoreError> {
        let Some(state_trie) = self.state_trie(block_hash)? else {
            return Ok(None);
        };

        Ok(Some(
            self.apply_account_updates_from_trie_batch(state_trie, account_updates)
                .await?,
        ))
    }

    pub async fn apply_account_updates_from_trie_batch(
        &self,
        mut state_trie: Trie,
        account_updates: impl IntoIterator<Item = &AccountUpdate>,
    ) -> Result<AccountUpdatesList, StoreError> {
        let mut ret_storage_updates = Vec::new();
        let mut code_updates = Vec::new();
        for update in account_updates {
            let hashed_address = hash_address(&update.address);
            if update.removed {
                // Remove account from trie
                state_trie.remove(&hashed_address)?;
                continue;
            }
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
                    code_updates.push((info.code_hash, code.clone()));
                }
            }
            // Store the added storage in the account's storage trie and compute its new root
            if !update.added_storage.is_empty() {
                let mut storage_trie = self.engine.open_storage_trie(
                    H256::from_slice(&hashed_address),
                    account_state.storage_root,
                )?;
                for (storage_key, storage_value) in &update.added_storage {
                    let hashed_key = hash_key(storage_key);
                    if storage_value.is_zero() {
                        storage_trie.remove(&hashed_key)?;
                    } else {
                        storage_trie.insert(hashed_key, storage_value.encode_to_vec())?;
                    }
                }
                let (storage_hash, storage_updates) =
                    storage_trie.collect_changes_since_last_hash();
                account_state.storage_root = storage_hash;
                ret_storage_updates.push((H256::from_slice(&hashed_address), storage_updates));
            }
            state_trie.insert(hashed_address, account_state.encode_to_vec())?;
        }
        let (state_trie_hash, state_updates) = state_trie.collect_changes_since_last_hash();

        Ok(AccountUpdatesList {
            state_trie_hash,
            state_updates,
            storage_updates: ret_storage_updates,
            code_updates,
        })
    }

    /// Performs the same actions as apply_account_updates_from_trie
    ///  but also returns the used storage tries with witness recorded
    pub async fn apply_account_updates_from_trie_with_witness(
        &self,
        mut state_trie: Trie,
        account_updates: &[AccountUpdate],
        mut storage_tries: HashMap<Address, (TrieWitness, Trie)>,
    ) -> Result<(Trie, HashMap<Address, (TrieWitness, Trie)>), StoreError> {
        for update in account_updates.iter() {
            let hashed_address = hash_address(&update.address);
            if update.removed {
                // Remove account from trie
                state_trie.remove(&hashed_address)?;
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
                        self.add_account_code(info.code_hash, code.clone()).await?;
                    }
                }
                // Store the added storage in the account's storage trie and compute its new root
                if !update.added_storage.is_empty() {
                    let (_witness, storage_trie) = match storage_tries.entry(update.address) {
                        std::collections::hash_map::Entry::Occupied(value) => value.into_mut(),
                        std::collections::hash_map::Entry::Vacant(vacant) => {
                            let trie = self.engine.open_storage_trie(
                                H256::from_slice(&hashed_address),
                                account_state.storage_root,
                            )?;
                            vacant.insert(TrieLogger::open_trie(trie))
                        }
                    };

                    for (storage_key, storage_value) in &update.added_storage {
                        let hashed_key = hash_key(storage_key);
                        if storage_value.is_zero() {
                            storage_trie.remove(&hashed_key)?;
                        } else {
                            storage_trie.insert(hashed_key, storage_value.encode_to_vec())?;
                        }
                    }
                    account_state.storage_root = storage_trie.hash_no_commit();
                }
                state_trie.insert(hashed_address, account_state.encode_to_vec())?;
            }
        }

        Ok((state_trie, storage_tries))
    }

    /// Adds all genesis accounts and returns the genesis block's state_root
    pub async fn setup_genesis_state_trie(
        &self,
        genesis_accounts: BTreeMap<Address, GenesisAccount>,
    ) -> Result<H256, StoreError> {
        let mut genesis_state_trie = self.engine.open_state_trie(*EMPTY_TRIE_HASH)?;
        for (address, account) in genesis_accounts {
            let hashed_address = hash_address(&address);
            // Store account code (as this won't be stored in the trie)
            let code_hash = code_hash(&account.code);
            self.add_account_code(code_hash, account.code).await?;
            // Store the account's storage in a clean storage trie and compute its root
            let mut storage_trie = self
                .engine
                .open_storage_trie(H256::from_slice(&hashed_address), *EMPTY_TRIE_HASH)?;
            for (storage_key, storage_value) in account.storage {
                if !storage_value.is_zero() {
                    let hashed_key = hash_key(&H256(storage_key.to_big_endian()));
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
            genesis_state_trie.insert(hashed_address, account_state.encode_to_vec())?;
        }
        genesis_state_trie.hash().map_err(StoreError::Trie)
    }

    pub async fn add_receipt(
        &self,
        block_hash: BlockHash,
        index: Index,
        receipt: Receipt,
    ) -> Result<(), StoreError> {
        self.engine.add_receipt(block_hash, index, receipt).await
    }

    pub async fn add_receipts(
        &self,
        block_hash: BlockHash,
        receipts: Vec<Receipt>,
    ) -> Result<(), StoreError> {
        self.engine.add_receipts(block_hash, receipts).await
    }

    /// Obtain receipt for a canonical block represented by the block number.
    pub async fn get_receipt(
        &self,
        block_number: BlockNumber,
        index: Index,
    ) -> Result<Option<Receipt>, StoreError> {
        // FIXME (#4353)
        let Some(block_hash) = self.get_canonical_block_hash(block_number).await? else {
            return Ok(None);
        };
        self.engine.get_receipt(block_hash, index).await
    }

    pub async fn add_block(&self, block: Block) -> Result<(), StoreError> {
        self.add_blocks(vec![block]).await
    }

    pub async fn add_blocks(&self, blocks: Vec<Block>) -> Result<(), StoreError> {
        self.engine.add_blocks(blocks).await
    }

    pub async fn add_initial_state(&self, genesis: Genesis) -> Result<(), StoreError> {
        debug!("Storing initial state from genesis");

        // Obtain genesis block
        let genesis_block = genesis.get_block();
        let genesis_block_number = genesis_block.header.number;

        let genesis_hash = genesis_block.hash();

        // Set chain config
        self.set_chain_config(&genesis.config).await?;

        if let Some(number) = self.engine.get_latest_block_number().await? {
            *self
                .latest_block_header
                .write()
                .map_err(|_| StoreError::LockError)? = self
                .engine
                .get_block_header(number)?
                .ok_or_else(|| StoreError::MissingLatestBlockNumber)?;
        }

        match self.engine.get_block_header(genesis_block_number)? {
            Some(header) if header.hash() == genesis_hash => {
                info!("Received genesis file matching a previously stored one, nothing to do");
                return Ok(());
            }
            Some(_) => {
                error!(
                    "The chain configuration stored in the database is incompatible with the provided configuration. If you intended to switch networks, choose another datadir or clear the database (e.g., run `ethrex removedb`) and try again."
                );
                return Err(StoreError::IncompatibleChainConfig);
            }
            None => {
                self.engine
                    .add_block_header(genesis_hash, genesis_block.header.clone())
                    .await?
            }
        }
        // Store genesis accounts
        // TODO: Should we use this root instead of computing it before the block hash check?
        let genesis_state_root = self.setup_genesis_state_trie(genesis.alloc).await?;
        debug_assert_eq!(genesis_state_root, genesis_block.header.state_root);

        // Store genesis block
        info!(hash = %genesis_hash, "Storing genesis block");

        self.add_block(genesis_block).await?;
        self.update_earliest_block_number(genesis_block_number)
            .await?;
        self.forkchoice_update(None, genesis_block_number, genesis_hash, None, None)
            .await?;
        Ok(())
    }

    pub async fn load_initial_state(&self) -> Result<(), StoreError> {
        info!("Loading initial state from DB");
        let Some(number) = self.engine.get_latest_block_number().await? else {
            return Err(StoreError::MissingLatestBlockNumber);
        };
        let latest_block_header = self
            .engine
            .get_block_header(number)?
            .ok_or_else(|| StoreError::Custom("latest block header is missing".to_string()))?;
        *self
            .latest_block_header
            .write()
            .map_err(|_| StoreError::LockError)? = latest_block_header;
        Ok(())
    }

    pub async fn get_transaction_by_hash(
        &self,
        transaction_hash: H256,
    ) -> Result<Option<Transaction>, StoreError> {
        self.engine.get_transaction_by_hash(transaction_hash).await
    }

    pub async fn get_transaction_by_location(
        &self,
        block_hash: BlockHash,
        index: Index,
    ) -> Result<Option<Transaction>, StoreError> {
        self.engine
            .get_transaction_by_location(block_hash, index)
            .await
    }

    pub async fn get_block_by_hash(&self, block_hash: H256) -> Result<Option<Block>, StoreError> {
        self.engine.get_block_by_hash(block_hash).await
    }

    pub async fn get_block_by_number(
        &self,
        block_number: BlockNumber,
    ) -> Result<Option<Block>, StoreError> {
        self.engine.get_block_by_number(block_number).await
    }

    pub async fn get_storage_at(
        &self,
        block_number: BlockNumber,
        address: Address,
        storage_key: H256,
    ) -> Result<Option<U256>, StoreError> {
        match self.get_canonical_block_hash(block_number).await? {
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

    pub async fn set_chain_config(&self, chain_config: &ChainConfig) -> Result<(), StoreError> {
        *self
            .chain_config
            .write()
            .map_err(|_| StoreError::LockError)? = *chain_config;
        self.engine.set_chain_config(chain_config).await
    }

    pub fn get_chain_config(&self) -> Result<ChainConfig, StoreError> {
        Ok(*self
            .chain_config
            .read()
            .map_err(|_| StoreError::LockError)?)
    }

    pub async fn update_earliest_block_number(
        &self,
        block_number: BlockNumber,
    ) -> Result<(), StoreError> {
        self.engine.update_earliest_block_number(block_number).await
    }

    pub async fn get_earliest_block_number(&self) -> Result<BlockNumber, StoreError> {
        self.engine
            .get_earliest_block_number()
            .await?
            .ok_or(StoreError::MissingEarliestBlockNumber)
    }

    pub async fn get_finalized_block_number(&self) -> Result<Option<BlockNumber>, StoreError> {
        self.engine.get_finalized_block_number().await
    }

    pub async fn get_safe_block_number(&self) -> Result<Option<BlockNumber>, StoreError> {
        self.engine.get_safe_block_number().await
    }

    pub async fn get_latest_block_number(&self) -> Result<BlockNumber, StoreError> {
        Ok(self
            .latest_block_header
            .read()
            .map_err(|_| StoreError::LockError)?
            .number)
    }

    pub async fn update_pending_block_number(
        &self,
        block_number: BlockNumber,
    ) -> Result<(), StoreError> {
        self.engine.update_pending_block_number(block_number).await
    }

    pub async fn get_pending_block_number(&self) -> Result<Option<BlockNumber>, StoreError> {
        self.engine.get_pending_block_number().await
    }

    pub async fn get_canonical_block_hash(
        &self,
        block_number: BlockNumber,
    ) -> Result<Option<BlockHash>, StoreError> {
        {
            let last = self
                .latest_block_header
                .read()
                .map_err(|_| StoreError::LockError)?;
            if last.number == block_number {
                return Ok(Some(last.hash()));
            }
        }
        self.engine.get_canonical_block_hash(block_number).await
    }

    pub async fn get_latest_canonical_block_hash(&self) -> Result<Option<BlockHash>, StoreError> {
        Ok(Some(
            self.latest_block_header
                .read()
                .map_err(|_| StoreError::LockError)?
                .hash(),
        ))
    }

    /// Updates the canonical chain.
    /// Inserts new canonical blocks, removes blocks beyond the new head,
    /// and updates the head, safe, and finalized block pointers.
    /// All operations are performed in a single database transaction.
    pub async fn forkchoice_update(
        &self,
        new_canonical_blocks: Option<Vec<(BlockNumber, BlockHash)>>,
        head_number: BlockNumber,
        head_hash: BlockHash,
        safe: Option<BlockNumber>,
        finalized: Option<BlockNumber>,
    ) -> Result<(), StoreError> {
        // Updates first the latest_block_header
        // to avoid nonce inconsistencies #3927.
        *self
            .latest_block_header
            .write()
            .map_err(|_| StoreError::LockError)? = self
            .engine
            .get_block_header_by_hash(head_hash)?
            .ok_or_else(|| StoreError::MissingLatestBlockNumber)?;
        self.engine
            .forkchoice_update(
                new_canonical_blocks,
                head_number,
                head_hash,
                safe,
                finalized,
            )
            .await?;

        Ok(())
    }

    /// Obtain the storage trie for the given block
    pub fn state_trie(&self, block_hash: BlockHash) -> Result<Option<Trie>, StoreError> {
        let Some(header) = self.get_block_header_by_hash(block_hash)? else {
            return Ok(None);
        };
        Ok(Some(self.engine.open_state_trie(header.state_root)?))
    }

    /// Obtain the storage trie for the given account on the given block
    pub fn storage_trie(
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
        Ok(Some(self.engine.open_storage_trie(
            H256::from_slice(&hashed_address),
            storage_root,
        )?))
    }

    pub async fn get_account_state(
        &self,
        block_number: BlockNumber,
        address: Address,
    ) -> Result<Option<AccountState>, StoreError> {
        let Some(block_hash) = self.get_canonical_block_hash(block_number).await? else {
            return Ok(None);
        };
        let Some(state_trie) = self.state_trie(block_hash)? else {
            return Ok(None);
        };
        self.get_account_state_from_trie(&state_trie, address)
    }

    pub fn get_account_state_by_hash(
        &self,
        block_hash: BlockHash,
        address: Address,
    ) -> Result<Option<AccountState>, StoreError> {
        let Some(state_trie) = self.state_trie(block_hash)? else {
            return Ok(None);
        };
        self.get_account_state_from_trie(&state_trie, address)
    }

    pub fn get_account_state_from_trie(
        &self,
        state_trie: &Trie,
        address: Address,
    ) -> Result<Option<AccountState>, StoreError> {
        let hashed_address = hash_address(&address);
        let Some(encoded_state) = state_trie.get(&hashed_address)? else {
            return Ok(None);
        };
        Ok(Some(AccountState::decode(&encoded_state)?))
    }

    pub async fn get_account_proof(
        &self,
        block_number: BlockNumber,
        address: &Address,
    ) -> Result<Option<Vec<Vec<u8>>>, StoreError> {
        let Some(block_hash) = self.get_canonical_block_hash(block_number).await? else {
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
        let trie = self
            .engine
            .open_storage_trie(hash_address_fixed(&address), storage_root)?;
        Ok(trie.get_proof(&hash_key(storage_key))?)
    }

    // Returns an iterator across all accounts in the state trie given by the state_root
    // Does not check that the state_root is valid
    pub fn iter_accounts_from(
        &self,
        state_root: H256,
        starting_address: H256,
    ) -> Result<impl Iterator<Item = (H256, AccountState)>, StoreError> {
        let mut iter = self.engine.open_locked_state_trie(state_root)?.into_iter();
        iter.advance(starting_address.0.to_vec())?;
        Ok(iter.content().map_while(|(path, value)| {
            Some((H256::from_slice(&path), AccountState::decode(&value).ok()?))
        }))
    }

    // Returns an iterator across all accounts in the state trie given by the state_root
    // Does not check that the state_root is valid
    pub fn iter_accounts(
        &self,
        state_root: H256,
    ) -> Result<impl Iterator<Item = (H256, AccountState)>, StoreError> {
        self.iter_accounts_from(state_root, H256::zero())
    }

    // Returns an iterator across all accounts in the state trie given by the state_root
    // Does not check that the state_root is valid
    pub fn iter_storage_from(
        &self,
        state_root: H256,
        hashed_address: H256,
        starting_slot: H256,
    ) -> Result<Option<impl Iterator<Item = (H256, U256)>>, StoreError> {
        let state_trie = self.engine.open_locked_state_trie(state_root)?;
        let Some(account_rlp) = state_trie.get(&hashed_address.as_bytes().to_vec())? else {
            return Ok(None);
        };
        let storage_root = AccountState::decode(&account_rlp)?.storage_root;
        let mut iter = self
            .engine
            .open_locked_storage_trie(hashed_address, storage_root)?
            .into_iter();
        iter.advance(starting_slot.0.to_vec())?;
        Ok(Some(iter.content().map_while(|(path, value)| {
            Some((H256::from_slice(&path), U256::decode(&value).ok()?))
        })))
    }

    // Returns an iterator across all accounts in the state trie given by the state_root
    // Does not check that the state_root is valid
    pub fn iter_storage(
        &self,
        state_root: H256,
        hashed_address: H256,
    ) -> Result<Option<impl Iterator<Item = (H256, U256)>>, StoreError> {
        self.iter_storage_from(state_root, hashed_address, H256::zero())
    }

    pub fn get_account_range_proof(
        &self,
        state_root: H256,
        starting_hash: H256,
        last_hash: Option<H256>,
    ) -> Result<Vec<Vec<u8>>, StoreError> {
        let state_trie = self.engine.open_state_trie(state_root)?;
        let mut proof = state_trie.get_proof(&starting_hash.as_bytes().to_vec())?;
        if let Some(last_hash) = last_hash {
            proof.extend_from_slice(&state_trie.get_proof(&last_hash.as_bytes().to_vec())?);
        }
        Ok(proof)
    }

    pub fn get_storage_range_proof(
        &self,
        state_root: H256,
        hashed_address: H256,
        starting_hash: H256,
        last_hash: Option<H256>,
    ) -> Result<Option<Vec<Vec<u8>>>, StoreError> {
        let state_trie = self.engine.open_state_trie(state_root)?;
        let Some(account_rlp) = state_trie.get(&hashed_address.as_bytes().to_vec())? else {
            return Ok(None);
        };
        let storage_root = AccountState::decode(&account_rlp)?.storage_root;
        let storage_trie = self
            .engine
            .open_storage_trie(hashed_address, storage_root)?;
        let mut proof = storage_trie.get_proof(&starting_hash.as_bytes().to_vec())?;
        if let Some(last_hash) = last_hash {
            proof.extend_from_slice(&storage_trie.get_proof(&last_hash.as_bytes().to_vec())?);
        }
        Ok(Some(proof))
    }

    /// Receives the root of the state trie and a list of paths where the first path will correspond to a path in the state trie
    /// (aka a hashed account address) and the following paths will be paths in the account's storage trie (aka hashed storage keys)
    /// If only one hash (account) is received, then the state trie node containing the account will be returned.
    /// If more than one hash is received, then the storage trie nodes where each storage key is stored will be returned
    /// For more information check out snap capability message [`GetTrieNodes`](https://github.com/ethereum/devp2p/blob/master/caps/snap.md#gettrienodes-0x06)
    /// The paths can be either full paths (hash) or partial paths (compact-encoded nibbles), if a partial path is given for the account this method will not return storage nodes for it
    pub fn get_trie_nodes(
        &self,
        state_root: H256,
        paths: Vec<Vec<u8>>,
        byte_limit: u64,
    ) -> Result<Vec<Vec<u8>>, StoreError> {
        let Some(account_path) = paths.first() else {
            return Ok(vec![]);
        };
        let state_trie = self.engine.open_state_trie(state_root)?;
        // State Trie Nodes Request
        if paths.len() == 1 {
            // Fetch state trie node
            let node = state_trie.get_node(account_path)?;
            return Ok(vec![node]);
        }
        // Storage Trie Nodes Request
        let Some(account_state) = state_trie
            .get(account_path)?
            .map(|ref rlp| AccountState::decode(rlp))
            .transpose()?
        else {
            return Ok(vec![]);
        };
        // We can't access the storage trie without the account's address hash
        let Ok(hashed_address) = account_path.clone().try_into().map(H256) else {
            return Ok(vec![]);
        };
        let storage_trie = self
            .engine
            .open_storage_trie(hashed_address, account_state.storage_root)?;
        // Fetch storage trie nodes
        let mut nodes = vec![];
        let mut bytes_used = 0;
        for path in paths.iter().skip(1) {
            if bytes_used >= byte_limit {
                break;
            }
            let node = storage_trie.get_node(path)?;
            bytes_used += node.len() as u64;
            nodes.push(node);
        }
        Ok(nodes)
    }

    pub fn get_receipts_for_block(
        &self,
        block_hash: &BlockHash,
    ) -> Result<Vec<Receipt>, StoreError> {
        self.engine.get_receipts_for_block(block_hash)
    }

    /// Creates a new state trie with an empty state root, for testing purposes only
    pub fn new_state_trie_for_test(&self) -> Result<Trie, StoreError> {
        self.engine.open_state_trie(*EMPTY_TRIE_HASH)
    }

    // Methods exclusive for trie management during snap-syncing

    /// Obtain a state trie from the given state root.
    /// Doesn't check if the state root is valid
    pub fn open_state_trie(&self, state_root: H256) -> Result<Trie, StoreError> {
        self.engine.open_state_trie(state_root)
    }

    /// Obtain a read-locked state trie from the given state root.
    /// Doesn't check if the state root is valid
    pub fn open_locked_state_trie(&self, state_root: H256) -> Result<Trie, StoreError> {
        self.engine.open_locked_state_trie(state_root)
    }

    /// Obtain a storage trie from the given address and storage_root.
    /// Doesn't check if the account is stored
    pub fn open_storage_trie(
        &self,
        account_hash: H256,
        storage_root: H256,
    ) -> Result<Trie, StoreError> {
        self.engine.open_storage_trie(account_hash, storage_root)
    }

    /// Obtain a read-locked storage trie from the given address and storage_root.
    /// Doesn't check if the account is stored
    pub fn open_locked_storage_trie(
        &self,
        account_hash: H256,
        storage_root: H256,
    ) -> Result<Trie, StoreError> {
        self.engine
            .open_locked_storage_trie(account_hash, storage_root)
    }

    /// Returns true if the given node is part of the state trie's internal storage
    pub fn contains_state_node(&self, node_hash: H256) -> Result<bool, StoreError> {
        // Root is irrelevant, we only care about the internal state
        Ok(self
            .open_state_trie(*EMPTY_TRIE_HASH)?
            .db()
            .get(node_hash.into())?
            .is_some())
    }

    /// Returns true if the given node is part of the given storage trie's internal storage
    pub fn contains_storage_node(
        &self,
        hashed_address: H256,
        node_hash: H256,
    ) -> Result<bool, StoreError> {
        // Root is irrelevant, we only care about the internal state
        Ok(self
            .open_storage_trie(hashed_address, *EMPTY_TRIE_HASH)?
            .db()
            .get(node_hash.into())?
            .is_some())
    }

    /// Sets the hash of the last header downloaded during a snap sync
    pub async fn set_header_download_checkpoint(
        &self,
        block_hash: BlockHash,
    ) -> Result<(), StoreError> {
        self.engine.set_header_download_checkpoint(block_hash).await
    }

    /// Gets the hash of the last header downloaded during a snap sync
    pub async fn get_header_download_checkpoint(&self) -> Result<Option<BlockHash>, StoreError> {
        self.engine.get_header_download_checkpoint().await
    }

    /// Sets the last key fetched from the state trie being fetched during snap sync
    pub async fn set_state_trie_key_checkpoint(
        &self,
        last_keys: [H256; STATE_TRIE_SEGMENTS],
    ) -> Result<(), StoreError> {
        self.engine.set_state_trie_key_checkpoint(last_keys).await
    }

    /// Gets the last key fetched from the state trie being fetched during snap sync
    pub async fn get_state_trie_key_checkpoint(
        &self,
    ) -> Result<Option<[H256; STATE_TRIE_SEGMENTS]>, StoreError> {
        self.engine.get_state_trie_key_checkpoint().await
    }

    /// Sets the state trie paths in need of healing
    pub async fn set_state_heal_paths(
        &self,
        paths: Vec<(Nibbles, H256)>,
    ) -> Result<(), StoreError> {
        self.engine.set_state_heal_paths(paths).await
    }

    /// Gets the state trie paths in need of healing
    pub async fn get_state_heal_paths(&self) -> Result<Option<Vec<(Nibbles, H256)>>, StoreError> {
        self.engine.get_state_heal_paths().await
    }

    /// Set the latest root of the rebuilt state trie and the last downloaded hashes from each segment
    pub async fn set_state_trie_rebuild_checkpoint(
        &self,
        checkpoint: (H256, [H256; STATE_TRIE_SEGMENTS]),
    ) -> Result<(), StoreError> {
        self.engine
            .set_state_trie_rebuild_checkpoint(checkpoint)
            .await
    }

    /// Get the latest root of the rebuilt state trie and the last downloaded hashes from each segment
    pub async fn get_state_trie_rebuild_checkpoint(
        &self,
    ) -> Result<Option<(H256, [H256; STATE_TRIE_SEGMENTS])>, StoreError> {
        self.engine.get_state_trie_rebuild_checkpoint().await
    }

    /// Set the accont hashes and roots of the storage tries awaiting rebuild
    pub async fn set_storage_trie_rebuild_pending(
        &self,
        pending: Vec<(H256, H256)>,
    ) -> Result<(), StoreError> {
        self.engine.set_storage_trie_rebuild_pending(pending).await
    }

    /// Get the accont hashes and roots of the storage tries awaiting rebuild
    pub async fn get_storage_trie_rebuild_pending(
        &self,
    ) -> Result<Option<Vec<(H256, H256)>>, StoreError> {
        self.engine.get_storage_trie_rebuild_pending().await
    }

    /// Clears all checkpoint data created during the last snap sync
    pub async fn clear_snap_state(&self) -> Result<(), StoreError> {
        self.engine.clear_snap_state().await
    }

    /// Fetches the latest valid ancestor for a block that was previously marked as invalid
    /// Returns None if the block was never marked as invalid
    pub async fn get_latest_valid_ancestor(
        &self,
        block: BlockHash,
    ) -> Result<Option<BlockHash>, StoreError> {
        self.engine.get_latest_valid_ancestor(block).await
    }

    /// Marks a block as invalid and sets its latest valid ancestor
    pub async fn set_latest_valid_ancestor(
        &self,
        bad_block: BlockHash,
        latest_valid: BlockHash,
    ) -> Result<(), StoreError> {
        self.engine
            .set_latest_valid_ancestor(bad_block, latest_valid)
            .await
    }

    /// Takes a block hash and returns an iterator to its ancestors. Block headers are returned
    /// in reverse order, starting from the given block and going up to the genesis block.
    pub fn ancestors(&self, block_hash: BlockHash) -> AncestorIterator {
        AncestorIterator {
            store: self.clone(),
            next_hash: block_hash,
        }
    }

    /// Get the canonical block hash for a given block number.
    pub fn get_canonical_block_hash_sync(
        &self,
        block_number: BlockNumber,
    ) -> Result<Option<BlockHash>, StoreError> {
        {
            let last = self
                .latest_block_header
                .read()
                .map_err(|_| StoreError::LockError)?;
            if last.number == block_number {
                return Ok(Some(last.hash()));
            }
        }
        self.engine.get_canonical_block_hash_sync(block_number)
    }

    /// Checks if a given block belongs to the current canonical chain. Returns false if the block is not known
    pub fn is_canonical_sync(&self, block_hash: BlockHash) -> Result<bool, StoreError> {
        let Some(block_number) = self.engine.get_block_number_sync(block_hash)? else {
            return Ok(false);
        };
        Ok(self
            .get_canonical_block_hash_sync(block_number)?
            .is_some_and(|h| h == block_hash))
    }

    pub async fn write_storage_trie_nodes_batch(
        &self,
        storage_trie_nodes: StorageTrieNodes,
    ) -> Result<(), StoreError> {
        self.engine
            .write_storage_trie_nodes_batch(storage_trie_nodes)
            .await
    }

    pub async fn write_account_code_batch(
        &self,
        account_codes: Vec<(H256, Bytes)>,
    ) -> Result<(), StoreError> {
        self.engine.write_account_code_batch(account_codes).await
    }
}

pub struct AncestorIterator {
    store: Store,
    next_hash: BlockHash,
}

impl Iterator for AncestorIterator {
    type Item = Result<(BlockHash, BlockHeader), StoreError>;

    fn next(&mut self) -> Option<Self::Item> {
        let next_hash = self.next_hash;
        match self.store.get_block_header_by_hash(next_hash) {
            Ok(Some(header)) => {
                let ret_hash = self.next_hash;
                self.next_hash = header.parent_hash;
                Some(Ok((ret_hash, header)))
            }
            Ok(None) => None,
            Err(e) => Some(Err(e)),
        }
    }
}

pub fn hash_address(address: &Address) -> Vec<u8> {
    Keccak256::new_with_prefix(address.to_fixed_bytes())
        .finalize()
        .to_vec()
}
fn hash_address_fixed(address: &Address) -> H256 {
    H256(
        Keccak256::new_with_prefix(address.to_fixed_bytes())
            .finalize()
            .into(),
    )
}

pub fn hash_key(key: &H256) -> Vec<u8> {
    Keccak256::new_with_prefix(key.to_fixed_bytes())
        .finalize()
        .to_vec()
}

#[cfg(test)]
mod tests {
    use bytes::Bytes;
    use ethereum_types::{H256, U256};
    use ethrex_common::{
        Bloom, H160,
        constants::EMPTY_KECCACK_HASH,
        types::{Transaction, TxType},
    };
    use ethrex_rlp::decode::RLPDecode;
    use std::{fs, str::FromStr};

    use super::*;

    #[tokio::test]
    async fn test_in_memory_store() {
        test_store_suite(EngineType::InMemory).await;
    }

    #[cfg(feature = "libmdbx")]
    #[tokio::test]
    async fn test_libmdbx_store() {
        test_store_suite(EngineType::Libmdbx).await;
    }

    #[cfg(feature = "rocksdb")]
    #[tokio::test]
    async fn test_rocksdb_store() {
        test_store_suite(EngineType::RocksDB).await;
    }

    // Creates an empty store, runs the test and then removes the store (if needed)
    async fn run_test<F, Fut>(test_func: F, engine_type: EngineType)
    where
        F: FnOnce(Store) -> Fut,
        Fut: std::future::Future<Output = ()>,
    {
        // Remove preexistent DBs in case of a failed previous test
        if !matches!(engine_type, EngineType::InMemory) {
            remove_test_dbs("store-test-db");
        };
        // Build a new store
        let store = Store::new("store-test-db", engine_type).expect("Failed to create test db");
        // Run the test
        test_func(store).await;
        // Remove store (if needed)
        if !matches!(engine_type, EngineType::InMemory) {
            remove_test_dbs("store-test-db");
        };
    }

    async fn test_store_suite(engine_type: EngineType) {
        run_test(test_store_block, engine_type).await;
        run_test(test_store_block_number, engine_type).await;
        run_test(test_store_transaction_location, engine_type).await;
        run_test(test_store_transaction_location_not_canonical, engine_type).await;
        run_test(test_store_block_receipt, engine_type).await;
        run_test(test_store_account_code, engine_type).await;
        run_test(test_store_block_tags, engine_type).await;
        run_test(test_chain_config_storage, engine_type).await;
        run_test(test_genesis_block, engine_type).await;
        run_test(test_iter_accounts, engine_type).await;
        run_test(test_iter_storage, engine_type).await;
    }

    async fn test_iter_accounts(store: Store) {
        let mut accounts: Vec<_> = (0u64..1_000)
            .map(|i| {
                (
                    H256(Keccak256::digest(i.to_be_bytes()).into()),
                    AccountState {
                        nonce: 2 * i,
                        balance: U256::from(3 * i),
                        code_hash: *EMPTY_KECCACK_HASH,
                        storage_root: *EMPTY_TRIE_HASH,
                    },
                )
            })
            .collect();
        accounts.sort_by_key(|a| a.0);
        let mut trie = store.open_state_trie(*EMPTY_TRIE_HASH).unwrap();
        for (address, state) in &accounts {
            trie.insert(address.0.to_vec(), state.encode_to_vec())
                .unwrap();
        }
        let state_root = trie.hash().unwrap();
        let pivot = H256::random();
        let pos = accounts.partition_point(|(key, _)| key < &pivot);
        let account_iter = store.iter_accounts_from(state_root, pivot).unwrap();
        for (expected, actual) in std::iter::zip(accounts.drain(pos..), account_iter) {
            assert_eq!(expected, actual);
        }
    }

    async fn test_iter_storage(store: Store) {
        let address = H256(Keccak256::digest(12345u64.to_be_bytes()).into());
        let mut slots: Vec<_> = (0u64..1_000)
            .map(|i| {
                (
                    H256(Keccak256::digest(i.to_be_bytes()).into()),
                    U256::from(2 * i),
                )
            })
            .collect();
        slots.sort_by_key(|a| a.0);
        let mut trie = store.open_storage_trie(address, *EMPTY_TRIE_HASH).unwrap();
        for (slot, value) in &slots {
            trie.insert(slot.0.to_vec(), value.encode_to_vec()).unwrap();
        }
        let storage_root = trie.hash().unwrap();
        let mut trie = store.open_state_trie(*EMPTY_TRIE_HASH).unwrap();
        trie.insert(
            address.0.to_vec(),
            AccountState {
                nonce: 1,
                balance: U256::zero(),
                storage_root,
                code_hash: *EMPTY_KECCACK_HASH,
            }
            .encode_to_vec(),
        )
        .unwrap();
        let state_root = trie.hash().unwrap();
        let pivot = H256::random();
        let pos = slots.partition_point(|(key, _)| key < &pivot);
        let storage_iter = store
            .iter_storage_from(state_root, address, pivot)
            .unwrap()
            .unwrap();
        for (expected, actual) in std::iter::zip(slots.drain(pos..), storage_iter) {
            assert_eq!(expected, actual);
        }
    }

    async fn test_genesis_block(store: Store) {
        const GENESIS_KURTOSIS: &str = include_str!("../../fixtures/genesis/kurtosis.json");
        const GENESIS_HIVE: &str = include_str!("../../fixtures/genesis/hive.json");
        assert_ne!(GENESIS_KURTOSIS, GENESIS_HIVE);
        let genesis_kurtosis: Genesis =
            serde_json::from_str(GENESIS_KURTOSIS).expect("deserialize kurtosis.json");
        let genesis_hive: Genesis =
            serde_json::from_str(GENESIS_HIVE).expect("deserialize hive.json");
        store
            .add_initial_state(genesis_kurtosis.clone())
            .await
            .expect("first genesis");
        store
            .add_initial_state(genesis_kurtosis)
            .await
            .expect("second genesis with same block");
        let result = store.add_initial_state(genesis_hive).await;
        assert!(result.is_err());
        assert!(matches!(result, Err(StoreError::IncompatibleChainConfig)));
    }

    fn remove_test_dbs(path: &str) {
        // Removes all test databases from filesystem
        if std::path::Path::new(path).exists() {
            fs::remove_dir_all(path).expect("Failed to clean test db dir");
        }
    }

    async fn test_store_block(store: Store) {
        let (block_header, block_body) = create_block_for_testing();
        let block_number = 6;
        let hash = block_header.hash();

        store
            .add_block_header(hash, block_header.clone())
            .await
            .unwrap();
        store
            .add_block_body(hash, block_body.clone())
            .await
            .unwrap();
        store
            .forkchoice_update(None, block_number, hash, None, None)
            .await
            .unwrap();

        let stored_header = store.get_block_header(block_number).unwrap().unwrap();
        let stored_body = store.get_block_body(block_number).await.unwrap().unwrap();

        // Ensure both headers have their hashes computed for comparison
        let _ = stored_header.hash();
        let _ = block_header.hash();
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
            requests_hash: Some(*EMPTY_KECCACK_HASH),
            ..Default::default()
        };
        let block_body = BlockBody {
            transactions: vec![Transaction::decode(&hex::decode("b86f02f86c8330182480114e82f618946177843db3138ae69679a54b95cf345ed759450d870aa87bee53800080c080a0151ccc02146b9b11adf516e6787b59acae3e76544fdcd75e77e67c6b598ce65da064c5dd5aae2fbb535830ebbdad0234975cd7ece3562013b63ea18cc0df6c97d4").unwrap()).unwrap(),
            Transaction::decode(&hex::decode("f86d80843baa0c4082f618946177843db3138ae69679a54b95cf345ed759450d870aa87bee538000808360306ba0151ccc02146b9b11adf516e6787b59acae3e76544fdcd75e77e67c6b598ce65da064c5dd5aae2fbb535830ebbdad0234975cd7ece3562013b63ea18cc0df6c97d4").unwrap()).unwrap()],
            ommers: Default::default(),
            withdrawals: Default::default(),
        };
        (block_header, block_body)
    }

    async fn test_store_block_number(store: Store) {
        let block_hash = H256::random();
        let block_number = 6;

        store
            .add_block_number(block_hash, block_number)
            .await
            .unwrap();

        let stored_number = store.get_block_number(block_hash).await.unwrap().unwrap();

        assert_eq!(stored_number, block_number);
    }

    async fn test_store_transaction_location(store: Store) {
        let transaction_hash = H256::random();
        let block_hash = H256::random();
        let block_number = 6;
        let index = 3;

        store
            .add_transaction_location(transaction_hash, block_number, block_hash, index)
            .await
            .unwrap();

        store
            .add_block_header(block_hash, BlockHeader::default())
            .await
            .unwrap();

        store
            .forkchoice_update(None, block_number, block_hash, None, None)
            .await
            .unwrap();

        let stored_location = store
            .get_transaction_location(transaction_hash)
            .await
            .unwrap()
            .unwrap();

        assert_eq!(stored_location, (block_number, block_hash, index));
    }

    async fn test_store_transaction_location_not_canonical(store: Store) {
        let transaction_hash = H256::random();
        let block_header = BlockHeader::default();
        let random_hash = H256::random();
        let block_number = 6;
        let index = 3;

        store
            .add_transaction_location(transaction_hash, block_number, block_header.hash(), index)
            .await
            .unwrap();

        store
            .add_block_header(block_header.hash(), block_header.clone())
            .await
            .unwrap();

        // Store random block hash
        store
            .add_block_header(random_hash, block_header)
            .await
            .unwrap();

        store
            .forkchoice_update(None, block_number, random_hash, None, None)
            .await
            .unwrap();

        assert_eq!(
            store
                .get_transaction_location(transaction_hash)
                .await
                .unwrap(),
            None
        )
    }

    async fn test_store_block_receipt(store: Store) {
        let receipt = Receipt {
            tx_type: TxType::EIP2930,
            succeeded: true,
            cumulative_gas_used: 1747,
            logs: vec![],
        };
        let block_number = 6;
        let index = 4;
        let block_header = BlockHeader::default();

        store
            .add_receipt(block_header.hash(), index, receipt.clone())
            .await
            .unwrap();

        store
            .add_block_header(block_header.hash(), block_header.clone())
            .await
            .unwrap();

        store
            .forkchoice_update(None, block_number, block_header.hash(), None, None)
            .await
            .unwrap();

        let stored_receipt = store
            .get_receipt(block_number, index)
            .await
            .unwrap()
            .unwrap();

        assert_eq!(stored_receipt, receipt);
    }

    async fn test_store_account_code(store: Store) {
        let code_hash = H256::random();
        let code = Bytes::from("kiwi");

        store
            .add_account_code(code_hash, code.clone())
            .await
            .unwrap();

        let stored_code = store.get_account_code(code_hash).unwrap().unwrap();

        assert_eq!(stored_code, code);
    }

    async fn test_store_block_tags(store: Store) {
        let earliest_block_number = 0;
        let finalized_block_number = 7;
        let safe_block_number = 6;
        let latest_block_number = 8;
        let pending_block_number = 9;

        let (mut block_header, block_body) = create_block_for_testing();
        block_header.number = latest_block_number;
        let hash = block_header.hash();

        store
            .add_block_header(hash, block_header.clone())
            .await
            .unwrap();
        store
            .add_block_body(hash, block_body.clone())
            .await
            .unwrap();

        store
            .update_earliest_block_number(earliest_block_number)
            .await
            .unwrap();
        store
            .update_pending_block_number(pending_block_number)
            .await
            .unwrap();
        store
            .forkchoice_update(
                None,
                latest_block_number,
                hash,
                Some(safe_block_number),
                Some(finalized_block_number),
            )
            .await
            .unwrap();

        let stored_earliest_block_number = store.get_earliest_block_number().await.unwrap();
        let stored_finalized_block_number =
            store.get_finalized_block_number().await.unwrap().unwrap();
        let stored_latest_block_number = store.get_latest_block_number().await.unwrap();
        let stored_safe_block_number = store.get_safe_block_number().await.unwrap().unwrap();
        let stored_pending_block_number = store.get_pending_block_number().await.unwrap().unwrap();

        assert_eq!(earliest_block_number, stored_earliest_block_number);
        assert_eq!(finalized_block_number, stored_finalized_block_number);
        assert_eq!(safe_block_number, stored_safe_block_number);
        assert_eq!(latest_block_number, stored_latest_block_number);
        assert_eq!(pending_block_number, stored_pending_block_number);
    }

    async fn test_chain_config_storage(store: Store) {
        let chain_config = example_chain_config();
        store.set_chain_config(&chain_config).await.unwrap();
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
            deposit_contract_address: H160::from_str("0x4242424242424242424242424242424242424242")
                .unwrap(),
            ..Default::default()
        }
    }
}
