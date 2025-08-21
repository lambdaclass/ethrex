use crate::error::StoreError;
use crate::store_db::blob::BlobDbEngine;
use crate::store_db::in_memory::Store as InMemoryStore;
#[cfg(feature = "libmdbx")]
use crate::store_db::libmdbx::Store as LibmdbxStore;
use crate::{api::StoreEngine, blob::BlobDbRoTxn};
use bytes::Bytes;

use ethereum_types::{Address, H256, U256};
use ethrex_common::{
    BigEndianHash,
    constants::EMPTY_TRIE_HASH,
    types::{
        AccountInfo, AccountState, AccountUpdate, Block, BlockBody, BlockHash, BlockHeader,
        BlockNumber, ChainConfig, ForkId, Genesis, GenesisAccount, Index, Receipt, Transaction,
        code_hash, payload::PayloadBundle,
    },
};
use ethrex_rlp::decode::RLPDecode;
use ethrex_rlp::encode::RLPEncode;
use ethrex_trie::{
    Nibbles, Node, NodeHandle, NodeHash, NodeRef, Trie, TrieError, TrieLogger, TrieWitness,
};
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
    engine: Arc<dyn StoreEngine>,
    blob_engine: Arc<BlobDbEngine>,
    chain_config: Arc<RwLock<ChainConfig>>,
    latest_block_header: Arc<RwLock<BlockHeader>>,
}

pub type StorageTrieNodes = Vec<(H256, Vec<NodeRef>)>;

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EngineType {
    InMemory,
    #[cfg(feature = "libmdbx")]
    Libmdbx,
}

#[derive(Default)]
pub struct UpdateBatch {
    pub state_trie_root_hash: H256,
    pub state_trie_root_handle: NodeHandle,
    /// Nodes to be added to the state trie
    pub account_updates: Vec<NodeRef>,
    /// Storage tries updated and their new nodes
    pub storage_updates: Vec<(H256, Vec<NodeRef>)>,
    /// Blocks to be added
    pub blocks: Vec<Block>,
    /// Receipts added per block
    pub receipts: Vec<(H256, Vec<Receipt>)>,
    /// Code updates
    pub code_updates: Vec<(H256, Bytes)>,
}

type StorageUpdates = Vec<(H256, Vec<NodeRef>)>;

pub struct AccountUpdatesList {
    pub trie_version: u64,
    pub state_trie_root_hash: H256,
    pub state_trie_root_handle: NodeHandle,
    pub code_updates: Vec<(H256, Bytes)>,
}

impl Store {
    pub async fn store_block_updates(&self, update_batch: UpdateBatch) -> Result<(), StoreError> {
        self.engine.apply_updates(update_batch).await
    }

    // Tests and benchmarks only
    pub fn new(path: &str, engine_type: EngineType) -> Result<Self, StoreError> {
        info!(engine = ?engine_type, path = %path, "Opening storage engine");
        let blobdb = Arc::new(BlobDbEngine::open(
            (!path.is_empty()).then(|| [path, "/ethrex.edb"].concat()),
            0,
        )?);
        let store = match engine_type {
            #[cfg(feature = "libmdbx")]
            EngineType::Libmdbx => Self {
                engine: Arc::new(LibmdbxStore::new(path)?),
                blob_engine: blobdb,
                chain_config: Default::default(),
                latest_block_header: Arc::new(RwLock::new(BlockHeader::default())),
            },
            EngineType::InMemory => Self {
                engine: Arc::new(InMemoryStore::new()),
                blob_engine: blobdb,
                chain_config: Default::default(),
                latest_block_header: Arc::new(RwLock::new(BlockHeader::default())),
            },
        };

        Ok(store)
    }

    // Reconstruct command only
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

    // Tx validation, payload building, RPC
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

    // Self::get_account_info and StoreVmDatabase::get_account_info
    pub fn get_account_info_by_hash(
        &self,
        block_hash: BlockHash,
        address: Address,
    ) -> Result<Option<AccountInfo>, StoreError> {
        // info!(
        //     block_hash = hex::encode(block_hash),
        //     address = hex::encode(address),
        //     "GET ACCOUNT INFO"
        // );
        let Some(state_trie) = self.state_trie(block_hash)? else {
            // info!("TRIE NOT FOUND");
            return Ok(None);
        };
        let hashed_address = hash_address(&address);
        let Some(Node::Leaf(encoded_state)) = state_trie
            .db()
            .get_path(Nibbles::from_bytes(&hashed_address))?
        else {
            // info!("VALUE NOT FOUND");
            return Ok(None);
        };
        let account_state = AccountState::decode(&encoded_state.value)?;
        // info!(
        //     code = hex::encode(account_state.code_hash),
        //     balance = hex::encode(account_state.balance.to_big_endian()),
        //     nonce = hex::encode(account_state.nonce.to_be_bytes()),
        //     "FOUND"
        // );
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

    // Tests only
    pub async fn add_block_header(
        &self,
        block_hash: BlockHash,
        block_header: BlockHeader,
    ) -> Result<(), StoreError> {
        self.engine.add_block_header(block_hash, block_header).await
    }

    // Sync only (p2p:SnapBlockSyncState::process_incoming_headers)
    pub async fn add_block_headers(
        &self,
        block_headers: Vec<BlockHeader>,
    ) -> Result<(), StoreError> {
        self.engine.add_block_headers(block_headers).await
    }

    // Many real users
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

    // Many real users
    pub fn get_block_header_by_hash(
        &self,
        block_hash: BlockHash,
    ) -> Result<Option<BlockHeader>, StoreError> {
        let latest = self
            .latest_block_header
            .read()
            .map_err(|_| StoreError::LockError)?
            .clone();
        if block_hash == latest.hash() {
            // info!("GOT FROM CACHE");
            return Ok(Some(latest));
        }
        // info!("MISSED CACHE");
        self.engine.get_block_header_by_hash(block_hash)
    }

    // RPC
    pub async fn get_block_body_by_hash(
        &self,
        block_hash: BlockHash,
    ) -> Result<Option<BlockBody>, StoreError> {
        self.engine.get_block_body_by_hash(block_hash).await
    }

    // Tests and p2p:store_block_bodies (should probably do so batched)
    pub async fn add_block_body(
        &self,
        block_hash: BlockHash,
        block_body: BlockBody,
    ) -> Result<(), StoreError> {
        self.engine.add_block_body(block_hash, block_body).await
    }

    // Many real users
    pub async fn get_block_body(
        &self,
        block_number: BlockNumber,
    ) -> Result<Option<BlockBody>, StoreError> {
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

    // Only Command::RevertBatch
    pub async fn remove_block(&self, block_number: BlockNumber) -> Result<(), StoreError> {
        self.engine.remove_block(block_number).await
    }

    // Payload bodies RPC
    pub async fn get_block_bodies(
        &self,
        from: BlockNumber,
        to: BlockNumber,
    ) -> Result<Vec<BlockBody>, StoreError> {
        self.engine.get_block_bodies(from, to).await
    }

    // Unused
    pub async fn get_block_bodies_by_hash(
        &self,
        hashes: Vec<BlockHash>,
    ) -> Result<Vec<BlockBody>, StoreError> {
        self.engine.get_block_bodies_by_hash(hashes).await
    }

    // execute_block when parent is missing
    pub async fn add_pending_block(&self, block: Block) -> Result<(), StoreError> {
        info!("Adding block to pending: {}", block.hash());
        self.engine.add_pending_block(block).await
    }

    // sync_cycle
    pub async fn get_pending_block(
        &self,
        block_hash: BlockHash,
    ) -> Result<Option<Block>, StoreError> {
        info!("get pending: {}", block_hash);
        self.engine.get_pending_block(block_hash).await
    }

    // Command::reconstruct and tests
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

    // Header, body and tx RPC; they receiv hashes and return the object, the number
    // is not really necessary.
    // Also BlockIdentifierOrHash::resolve_block_number, which seems unused?
    pub async fn get_block_number(
        &self,
        block_hash: BlockHash,
    ) -> Result<Option<BlockNumber>, StoreError> {
        self.engine.get_block_number(block_hash).await
    }

    // p2p:set_fork_id
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

    // Tests only
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

    // Unused
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

    // Tracing and RPC
    pub async fn get_transaction_location(
        &self,
        transaction_hash: H256,
    ) -> Result<Option<(BlockNumber, BlockHash, Index)>, StoreError> {
        self.engine.get_transaction_location(transaction_hash).await
    }

    // Archive sync tool, fetch_bytecode_batch (should actually be batched) and internal methods
    pub async fn add_account_code(&self, code_hash: H256, code: Bytes) -> Result<(), StoreError> {
        self.engine.add_account_code(code_hash, code).await
    }

    // Many legit users
    pub fn get_account_code(&self, code_hash: H256) -> Result<Option<Bytes>, StoreError> {
        self.engine.get_account_code(code_hash)
    }

    // RPC
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

    // RPC
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
        account_updates: Vec<AccountUpdate>,
    ) -> Result<Option<AccountUpdatesList>, StoreError> {
        let Some(block_header) = self.get_block_header_by_hash(block_hash)? else {
            // info!("FAILED GET HEADER BY HASH");
            return Err(StoreError::Trie(TrieError::InconsistentTree));
        };
        let Some(root_handle) = self
            .engine
            .get_state_trie_root_handle(block_header.state_root)?
        else {
            // info!("FAILED GET STATE ROOT HANDLE");
            return Err(StoreError::Trie(TrieError::InconsistentTree));
        };
        self.blob_engine
            .apply_account_updates(block_header.state_root, root_handle, account_updates)
            .await
    }

    pub fn get_state_trie_root_handle(
        &self,
        state_root: H256,
    ) -> Result<Option<NodeHandle>, StoreError> {
        self.engine.get_state_trie_root_handle(state_root)
    }

    // Witness generation
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
                        self.add_account_code(info.code_hash, code.clone()).await?;
                    }
                }
                // Store the added storage in the account's storage trie and compute its new root
                if !update.added_storage.is_empty() {
                    let (_witness, storage_trie) = match storage_tries.entry(update.address) {
                        std::collections::hash_map::Entry::Occupied(value) => value.into_mut(),
                        std::collections::hash_map::Entry::Vacant(vacant) => {
                            let trie = self.open_storage_trie(
                                H256::from_slice(&hashed_address),
                                account_state.storage_root,
                            )?;
                            vacant.insert(TrieLogger::open_trie(trie))
                        }
                    };

                    for (storage_key, storage_value) in &update.added_storage {
                        let hashed_key = hash_key(storage_key);
                        if storage_value.is_zero() {
                            storage_trie.remove(hashed_key)?;
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

    // Self::add_initial_state
    /// Adds all genesis accounts and returns the genesis block's state_root
    pub async fn setup_genesis_state_trie(
        &self,
        genesis_accounts: BTreeMap<Address, GenesisAccount>,
    ) -> Result<H256, StoreError> {
        let account_updates = genesis_accounts
            .into_iter()
            .map(|(address, account)| AccountUpdate {
                address,
                added_storage: account
                    .storage
                    .into_iter()
                    .map(|(k, v)| (H256::from_uint(&k), v))
                    .collect(),
                removed: false,
                code: Some(account.code.clone()),
                info: Some(AccountInfo {
                    code_hash: code_hash(&account.code),
                    balance: account.balance,
                    nonce: account.nonce,
                }),
            })
            .collect();
        let Some(account_updates_list) = self
            .blob_engine
            .apply_account_updates(*EMPTY_TRIE_HASH, NodeHandle(0), account_updates)
            .await?
        else {
            return Ok(*EMPTY_TRIE_HASH);
        };
        self.store_block_updates(UpdateBatch {
            state_trie_root_hash: account_updates_list.state_trie_root_hash,
            state_trie_root_handle: account_updates_list.state_trie_root_handle,
            code_updates: account_updates_list.code_updates,
            ..Default::default()
        })
        .await?;
        Ok(account_updates_list.state_trie_root_hash)
    }

    // Tests only
    pub async fn add_receipt(
        &self,
        block_hash: BlockHash,
        index: Index,
        receipt: Receipt,
    ) -> Result<(), StoreError> {
        self.engine.add_receipt(block_hash, index, receipt).await
    }

    // Only by apparently unused P2P endpoint
    pub async fn add_receipts(
        &self,
        block_hash: BlockHash,
        receipts: Vec<Receipt>,
    ) -> Result<(), StoreError> {
        self.engine.add_receipts(block_hash, receipts).await
    }

    // Mostly L2, also get_all_block_rpc_receipts (should just get all together tho)
    pub async fn get_receipt(
        &self,
        block_number: BlockNumber,
        index: Index,
    ) -> Result<Option<Receipt>, StoreError> {
        self.engine.get_receipt(block_number, index).await
    }

    // Self::add_initial_state, add_blocks_with_transactions and archive sync tool
    pub async fn add_block(&self, block: Block) -> Result<(), StoreError> {
        self.add_blocks(vec![block]).await
    }

    // Only Self::add_block
    pub async fn add_blocks(&self, blocks: Vec<Block>) -> Result<(), StoreError> {
        self.engine.add_blocks(blocks).await
    }

    // Self::new_from_genesis, tests and init_store
    pub async fn add_initial_state(&self, genesis: Genesis) -> Result<(), StoreError> {
        debug!("Storing initial state from genesis");

        // Obtain genesis block
        let genesis_block = genesis.get_block();
        // info!("GOT BLOCK");
        let genesis_block_number = genesis_block.header.number;

        let genesis_hash = genesis_block.hash();
        // info!("HASHED BLOCK");

        // Set chain config
        self.set_chain_config(&genesis.config).await?;
        // info!("SET CHAIN CONFIG");

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
        // info!("GOT LATEST HEADER");
        // Store genesis accounts
        // TODO: Should we use this root instead of computing it before the block hash check?
        let genesis_state_root = self.setup_genesis_state_trie(genesis.alloc).await?;
        // info!("SETUP STATE TRIE");
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

    // initializers:load_store
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

    // RPC: GetRawTransaction
    // L2: l1_watcher + l1_to_l2_messages
    pub async fn get_transaction_by_hash(
        &self,
        transaction_hash: H256,
    ) -> Result<Option<Transaction>, StoreError> {
        self.engine.get_transaction_by_hash(transaction_hash).await
    }

    // Only RPC GetTransactionByHash
    // Could be a direct call to get_transaction_by_hash, maybe store the location with the tx itself
    pub async fn get_transaction_by_location(
        &self,
        block_hash: BlockHash,
        index: Index,
    ) -> Result<Option<Transaction>, StoreError> {
        self.engine
            .get_transaction_by_location(block_hash, index)
            .await
    }

    // Snap sync after becoming full + FCU to remove included txs + try_execute_payload as early exit when already executed
    // + execution witness RPC + get transaction receipt RPC (could be just the same as tx by hash mapping?)
    pub async fn get_block_by_hash(&self, block_hash: H256) -> Result<Option<Block>, StoreError> {
        self.engine.get_block_by_hash(block_hash).await
    }

    // RPC tracing + monitor widget + export command
    pub async fn get_block_by_number(
        &self,
        block_number: BlockNumber,
    ) -> Result<Option<Block>, StoreError> {
        self.engine.get_block_by_number(block_number).await
    }

    // RPC GetProofRequest + test runner
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

    // Self::get_storage_at, StoreVmDatabase::get_storage_slot
    pub fn get_storage_at_hash(
        &self,
        block_hash: BlockHash,
        address: Address,
        storage_key: H256,
    ) -> Result<Option<U256>, StoreError> {
        // info!(
        //     block_hash = hex::encode(block_hash),
        //     address = hex::encode(address),
        //     key = hex::encode(storage_key),
        //     "GET ACCOUNT STORAGE"
        // );
        let Some(storage_trie) = self.storage_trie(block_hash, address)? else {
            // info!("TRIE NOT FOUND");
            return Ok(None);
        };
        let hashed_key = hash_key(&storage_key);
        let Some(Node::Leaf(value)) = storage_trie
            .db()
            .get_path(Nibbles::from_bytes(&hashed_key))?
        else {
            // info!("VALUE NOT FOUND");
            return Ok(None);
        };
        let value = U256::decode(&value.value).map_err(StoreError::RLPDecode)?;
        // info!(value = hex::encode(value.to_big_endian()), "FOUND");
        Ok(Some(value))
    }

    // Tests, Self::add_initial_state
    pub async fn set_chain_config(&self, chain_config: &ChainConfig) -> Result<(), StoreError> {
        *self
            .chain_config
            .write()
            .map_err(|_| StoreError::LockError)? = *chain_config;
        self.engine.set_chain_config(chain_config).await
    }

    // Many users
    pub fn get_chain_config(&self) -> Result<ChainConfig, StoreError> {
        Ok(*self
            .chain_config
            .read()
            .map_err(|_| StoreError::LockError)?)
    }

    // Tests + Self::add_initial_state
    pub async fn update_earliest_block_number(
        &self,
        block_number: BlockNumber,
    ) -> Result<(), StoreError> {
        self.engine.update_earliest_block_number(block_number).await
    }

    // RPC: resolve_block_number + tests + fee market + syncing status RPC + find_link_with_canonical_chain (fork_choice)
    pub async fn get_earliest_block_number(&self) -> Result<BlockNumber, StoreError> {
        self.engine
            .get_earliest_block_number()
            .await?
            .ok_or(StoreError::MissingEarliestBlockNumber)
    }

    // RPC: resolve_block_number + tests
    pub async fn get_finalized_block_number(&self) -> Result<Option<BlockNumber>, StoreError> {
        self.engine.get_finalized_block_number().await
    }

    // RPC: resolve_block_number + tests
    pub async fn get_safe_block_number(&self) -> Result<Option<BlockNumber>, StoreError> {
        self.engine.get_safe_block_number().await
    }

    // Self::get_fork_id + BlockIdentifier::(is_latest|resolve_block_number) + estimate_gas_tip
    // + gas_price RPC + logs RPC + fee market + syncing RPC + blob base fee RPC + block number RPC
    // many many others
    pub async fn get_latest_block_number(&self) -> Result<BlockNumber, StoreError> {
        Ok(self
            .latest_block_header
            .read()
            .map_err(|_| StoreError::LockError)?
            .number)
    }

    // Only store tests
    pub async fn update_pending_block_number(
        &self,
        block_number: BlockNumber,
    ) -> Result<(), StoreError> {
        self.engine.update_pending_block_number(block_number).await
    }

    // RPC BlockIdentifier::resolve_block_number
    pub async fn get_pending_block_number(&self) -> Result<Option<BlockNumber>, StoreError> {
        self.engine.get_pending_block_number().await
    }

    // Self::get_account_(proof|state|info), Self::get_(code|nonce)_by_account_address, Self::get_storage_at,
    // gas_tip_estimator, blockchain::is_canonical, initializers, smoke_test, Command::RevertBatch
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

    // FCU for invalid payload, get_current_head for sync
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
    // FCU+test+some commands
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

    // L2, witnesses and internal
    /// Obtain the storage trie for the given block
    pub fn state_trie(&self, block_hash: BlockHash) -> Result<Option<Trie>, StoreError> {
        let Some(header) = self.get_block_header_by_hash(block_hash)? else {
            // info!(
            //     block_hash = hex::encode(block_hash),
            //     status = "BLOCK NOT FOUND",
            //     "OPEN STATE TRIE"
            // );
            return Ok(None);
        };
        let Some(state_root_handle) = self.engine.get_state_trie_root_handle(header.state_root)?
        else {
            // info!(
            //     block_hash = hex::encode(block_hash),
            //     status = "HANDLE NOT FOUND",
            //     "OPEN STATE TRIE"
            // );
            return Ok(None);
        };
        // info!(
        //     block_hash = hex::encode(block_hash),
        //     handle = hex::encode(state_root_handle.0.to_be_bytes()),
        //     status = "HANDLE FOUND",
        //     "OPEN STATE TRIE"
        // );
        Ok(Some(
            self.blob_engine
                .open_state_trie(header.state_root, state_root_handle)?,
        ))
    }

    // Witness generation and internal
    /// Obtain the storage trie for the given account on the given block
    pub fn storage_trie(
        &self,
        block_hash: BlockHash,
        address: Address,
    ) -> Result<Option<Trie>, StoreError> {
        // Fetch Account from state_trie
        let Some(header) = self.engine.get_block_header_by_hash(block_hash)? else {
            // info!(
            //     block_hash = hex::encode(block_hash),
            //     status = "BLOCK NOT FOUND",
            //     "OPEN STORAGE TRIE"
            // );
            return Ok(None);
        };
        let Some(state_root_handle) = self.engine.get_state_trie_root_handle(header.state_root)?
        else {
            // info!(
            //     block_hash = hex::encode(block_hash),
            //     status = "HANDLE NOT FOUND",
            //     "OPEN STORAGE TRIE"
            // );
            return Ok(None);
        };
        let hashed_address = hash_address(&address);
        // info!(
        //     block_hash = hex::encode(block_hash),
        //     handle = hex::encode(state_root_handle.0.to_be_bytes()),
        //     status = "HANDLE FOUND",
        //     "OPEN STORAGE TRIE"
        // );
        Ok(Some(self.blob_engine.open_storage_trie(
            header.state_root,
            state_root_handle,
            H256::from_slice(&hashed_address),
        )?))
    }

    // account RPC
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

    // No users
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

    // Internal
    pub fn get_account_state_from_trie(
        &self,
        state_trie: &Trie,
        address: Address,
    ) -> Result<Option<AccountState>, StoreError> {
        let hashed_address = hash_address(&address);
        let Some(Node::Leaf(encoded_state)) = state_trie
            .db()
            .get_path(Nibbles::from_bytes(&hashed_address))?
        else {
            return Ok(None);
        };
        Ok(Some(AccountState::decode(&encoded_state.value)?))
    }

    // RPC account proof request
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
    // RPC account proof request
    pub fn get_storage_proof(
        &self,
        address: Address,
        storage_root: H256,
        storage_key: &H256,
    ) -> Result<Vec<Vec<u8>>, StoreError> {
        let trie = self.open_storage_trie(hash_address_fixed(&address), storage_root)?;
        Ok(trie.get_proof(&hash_key(storage_key))?)
    }

    // Returns an iterator across all accounts in the state trie given by the state_root
    // Does not check that the state_root is valid
    // snap sync
    pub fn iter_accounts(
        &self,
        state_root: H256,
    ) -> Result<impl Iterator<Item = (H256, AccountState)>, StoreError> {
        Ok(self
            .open_state_trie(state_root)?
            .into_iter()
            .content()
            .map_while(|(path, value)| {
                Some((H256::from_slice(&path), AccountState::decode(&value).ok()?))
            }))
    }

    // Returns an iterator across all accounts in the state trie given by the state_root
    // Does not check that the state_root is valid
    // snap sync
    pub fn iter_storage(
        &self,
        state_root: H256,
        hashed_address: H256,
    ) -> Result<Option<impl Iterator<Item = (H256, U256)>>, StoreError> {
        let state_trie = self.open_state_trie(state_root)?;
        let Some(account_rlp) = state_trie.get(&hashed_address.as_bytes().to_vec())? else {
            return Ok(None);
        };
        let storage_root = AccountState::decode(&account_rlp)?.storage_root;
        Ok(Some(
            self.open_storage_trie(hashed_address, storage_root)?
                .into_iter()
                .content()
                .map_while(|(path, value)| {
                    Some((H256::from_slice(&path), U256::decode(&value).ok()?))
                }),
        ))
    }

    // snap sync
    pub fn get_account_range_proof(
        &self,
        state_root: H256,
        starting_hash: H256,
        last_hash: Option<H256>,
    ) -> Result<Vec<Vec<u8>>, StoreError> {
        let state_trie = self.open_state_trie(state_root)?;
        let mut proof = state_trie.get_proof(&starting_hash.as_bytes().to_vec())?;
        if let Some(last_hash) = last_hash {
            proof.extend_from_slice(&state_trie.get_proof(&last_hash.as_bytes().to_vec())?);
        }
        Ok(proof)
    }

    // snap sync
    pub fn get_storage_range_proof(
        &self,
        state_root: H256,
        hashed_address: H256,
        starting_hash: H256,
        last_hash: Option<H256>,
    ) -> Result<Option<Vec<Vec<u8>>>, StoreError> {
        let state_trie = self.open_state_trie(state_root)?;
        let Some(account_rlp) = state_trie.get(&hashed_address.as_bytes().to_vec())? else {
            return Ok(None);
        };
        let storage_root = AccountState::decode(&account_rlp)?.storage_root;
        let storage_trie = self.open_storage_trie(hashed_address, storage_root)?;
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
    // snap sync
    pub fn get_trie_nodes(
        &self,
        state_root: H256,
        paths: Vec<Vec<u8>>,
        byte_limit: u64,
    ) -> Result<Vec<Vec<u8>>, StoreError> {
        let Some(account_path) = paths.first() else {
            return Ok(vec![]);
        };
        let state_trie = self.open_state_trie(state_root)?;
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
        let storage_trie = self.open_storage_trie(hashed_address, account_state.storage_root)?;
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

    // build_payload
    pub async fn add_payload(&self, payload_id: u64, block: Block) -> Result<(), StoreError> {
        self.engine.add_payload(payload_id, block).await
    }

    // RPC
    pub async fn get_payload(&self, payload_id: u64) -> Result<Option<PayloadBundle>, StoreError> {
        self.engine.get_payload(payload_id).await
    }

    // build_payload_if_necessary and benchmark
    pub async fn update_payload(
        &self,
        payload_id: u64,
        payload: PayloadBundle,
    ) -> Result<(), StoreError> {
        self.engine.update_payload(payload_id, payload).await
    }

    // GetReceipts RLPx endpoint
    pub fn get_receipts_for_block(
        &self,
        block_hash: &BlockHash,
    ) -> Result<Vec<Receipt>, StoreError> {
        self.engine.get_receipts_for_block(block_hash)
    }

    // Tests
    /// Creates a new state trie with an empty state root, for testing purposes only
    pub fn new_state_trie_for_test(&self) -> Result<Trie, StoreError> {
        self.open_state_trie(*EMPTY_TRIE_HASH)
    }

    // Methods exclusive for trie management during snap-syncing

    /// Obtain a state trie from the given state root.
    /// Doesn't check if the state root is valid
    // Internal methods, archive sync tool, snap sync and some command
    pub fn open_state_trie(&self, state_root: H256) -> Result<Trie, StoreError> {
        if state_root == *EMPTY_TRIE_HASH {
            return Ok(Trie::new(Box::new(BlobDbRoTxn::new_empty())));
        }
        let root_handle = self
            .engine
            .get_state_trie_root_handle(state_root)?
            .ok_or(StoreError::Trie(TrieError::InconsistentTree))?;
        self.blob_engine.open_state_trie(state_root, root_handle)
    }

    /// Obtain a read-locked state trie from the given state root.
    /// Doesn't check if the state root is valid
    pub fn open_locked_state_trie(&self, state_root: H256) -> Result<Trie, StoreError> {
        self.open_state_trie(state_root)
    }

    /// Obtain a storage trie from the given address and storage_root.
    /// Doesn't check if the account is stored
    // Self::* methods, snap sync, archive sync tool
    pub fn open_storage_trie(
        &self,
        state_root: H256,
        account_hash: H256,
    ) -> Result<Trie, StoreError> {
        if state_root == *EMPTY_TRIE_HASH {
            return Ok(Trie::new(Box::new(BlobDbRoTxn::new_empty())));
        }
        let root_handle = self
            .engine
            .get_state_trie_root_handle(state_root)?
            .ok_or(StoreError::Trie(TrieError::InconsistentTree))?;
        self.blob_engine
            .open_storage_trie(state_root, root_handle, account_hash)
    }

    // tracing
    /// Returns true if the given node is part of the state trie's internal storage
    pub fn contains_state_root(&self, root_hash: H256) -> Result<bool, StoreError> {
        // Root is irrelevant, we only care about the internal state
        self.engine
            .get_state_trie_root_handle(root_hash)
            .map(|rh| rh.is_some())
    }

    // snap sync
    /// Returns true if the given node is part of the given storage trie's internal storage
    pub fn contains_storage_node(
        &self,
        _hashed_address: H256,
        _node_hash: H256,
    ) -> Result<bool, StoreError> {
        // Root is irrelevant, we only care about the internal state
        Ok(false)
        // FIXME: probably should just check if for the current state trie there is a root with this hash
        // Ok(self
        //     .open_storage_trie(hashed_address, *EMPTY_TRIE_HASH)?
        //     .db()
        //     .get(node_hash.into())?
        //     .is_some())
    }

    // snap sync
    /// Sets the hash of the last header downloaded during a snap sync
    pub async fn set_header_download_checkpoint(
        &self,
        block_hash: BlockHash,
    ) -> Result<(), StoreError> {
        self.engine.set_header_download_checkpoint(block_hash).await
    }

    // snap sync
    /// Gets the hash of the last header downloaded during a snap sync
    pub async fn get_header_download_checkpoint(&self) -> Result<Option<BlockHash>, StoreError> {
        self.engine.get_header_download_checkpoint().await
    }

    // snap sync
    /// Sets the last key fetched from the state trie being fetched during snap sync
    pub async fn set_state_trie_key_checkpoint(
        &self,
        last_keys: [H256; STATE_TRIE_SEGMENTS],
    ) -> Result<(), StoreError> {
        self.engine.set_state_trie_key_checkpoint(last_keys).await
    }

    // snap sync
    /// Gets the last key fetched from the state trie being fetched during snap sync
    pub async fn get_state_trie_key_checkpoint(
        &self,
    ) -> Result<Option<[H256; STATE_TRIE_SEGMENTS]>, StoreError> {
        self.engine.get_state_trie_key_checkpoint().await
    }

    // snap sync
    /// Sets the state trie paths in need of healing
    pub async fn set_state_heal_paths(
        &self,
        paths: Vec<(Nibbles, H256)>,
    ) -> Result<(), StoreError> {
        self.engine.set_state_heal_paths(paths).await
    }

    // snap sync
    /// Gets the state trie paths in need of healing
    pub async fn get_state_heal_paths(&self) -> Result<Option<Vec<(Nibbles, H256)>>, StoreError> {
        self.engine.get_state_heal_paths().await
    }

    // snap sync
    /// Write a storage batch into the current storage snapshot
    pub async fn write_snapshot_storage_batch(
        &self,
        account_hash: H256,
        storage_keys: Vec<H256>,
        storage_values: Vec<U256>,
    ) -> Result<(), StoreError> {
        self.engine
            .write_snapshot_storage_batch(account_hash, storage_keys, storage_values)
            .await
    }

    // snap sync
    /// Write multiple storage batches belonging to different accounts into the current storage snapshot
    pub async fn write_snapshot_storage_batches(
        &self,
        account_hashes: Vec<H256>,
        storage_keys: Vec<Vec<H256>>,
        storage_values: Vec<Vec<U256>>,
    ) -> Result<(), StoreError> {
        self.engine
            .write_snapshot_storage_batches(account_hashes, storage_keys, storage_values)
            .await
    }

    // state trie rebuild on snap sync
    /// Set the latest root of the rebuilt state trie and the last downloaded hashes from each segment
    pub async fn set_state_trie_rebuild_checkpoint(
        &self,
        checkpoint: (H256, [H256; STATE_TRIE_SEGMENTS]),
    ) -> Result<(), StoreError> {
        self.engine
            .set_state_trie_rebuild_checkpoint(checkpoint)
            .await
    }

    // state trie rebuild on snap sync
    /// Get the latest root of the rebuilt state trie and the last downloaded hashes from each segment
    pub async fn get_state_trie_rebuild_checkpoint(
        &self,
    ) -> Result<Option<(H256, [H256; STATE_TRIE_SEGMENTS])>, StoreError> {
        self.engine.get_state_trie_rebuild_checkpoint().await
    }

    // storage_trie rebuild on snap sync
    /// Set the accont hashes and roots of the storage tries awaiting rebuild
    pub async fn set_storage_trie_rebuild_pending(
        &self,
        pending: Vec<(H256, H256)>,
    ) -> Result<(), StoreError> {
        self.engine.set_storage_trie_rebuild_pending(pending).await
    }

    // storage_trie rebuild on snap sync
    /// Get the accont hashes and roots of the storage tries awaiting rebuild
    pub async fn get_storage_trie_rebuild_pending(
        &self,
    ) -> Result<Option<Vec<(H256, H256)>>, StoreError> {
        self.engine.get_storage_trie_rebuild_pending().await
    }

    // snap sync
    /// Clears all checkpoint data created during the last snap sync
    pub async fn clear_snap_state(&self) -> Result<(), StoreError> {
        self.engine.clear_snap_state().await
    }

    // storage_trie rebuild during snap sync
    /// Reads the next `MAX_SNAPSHOT_READS` elements from the storage snapshot as from the `start` storage key
    pub async fn read_storage_snapshot(
        &self,
        account_hash: H256,
        start: H256,
    ) -> Result<Vec<(H256, U256)>, StoreError> {
        self.engine.read_storage_snapshot(account_hash, start).await
    }

    // handle_forkchoice and validate_ancestors
    /// Fetches the latest valid ancestor for a block that was previously marked as invalid
    /// Returns None if the block was never marked as invalid
    pub async fn get_latest_valid_ancestor(
        &self,
        block: BlockHash,
    ) -> Result<Option<BlockHash>, StoreError> {
        self.engine.get_latest_valid_ancestor(block).await
    }

    // p2p:process_incoming_headers and RPC for payload execution
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

    // StoreVmDatabase::get_block_hash
    /// Takes a block hash and returns an iterator to its ancestors. Block headers are returned
    /// in reverse order, starting from the given block and going up to the genesis block.
    pub fn ancestors(&self, block_hash: BlockHash) -> AncestorIterator {
        AncestorIterator {
            store: self.clone(),
            next_hash: block_hash,
        }
    }

    // StoreVmDatabase::get_block_hash
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

    // StoreVmDatabase::get_block_hash only
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

// Store::ancestors
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

// For addressing into state trie/opening storage trie
pub fn hash_address(address: &Address) -> Vec<u8> {
    Keccak256::new_with_prefix(address.to_fixed_bytes())
        .finalize()
        .to_vec()
}
// Storage::get_storage_proof
fn hash_address_fixed(address: &Address) -> H256 {
    H256(
        Keccak256::new_with_prefix(address.to_fixed_bytes())
            .finalize()
            .into(),
    )
}

// Internal use and TrieLogger mostly
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
