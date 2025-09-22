#[cfg(feature = "rocksdb")]
use crate::backend::rocksdb::RocksDBBackend;
use crate::{
    api::{StorageBackend, StorageLocked, StorageRoTx, StorageRwTx, TableOptions},
    backend::in_memory::InMemoryBackend,
    error::StoreError,
    rlp::{AccountCodeRLP, BlockBodyRLP, BlockHashRLP, BlockHeaderRLP, BlockRLP},
    trie::{BackendTrieDB, BackendTrieDBLocked},
    utils::{ChainDataIndex, SnapStateIndex},
};
use bytes::Bytes;
use ethrex_common::{
    Address, H256, U256,
    types::{
        AccountInfo, AccountState, AccountUpdate, Block, BlockBody, BlockHash, BlockHeader,
        BlockNumber, ChainConfig, ForkId, Genesis, GenesisAccount, Index, Receipt, Transaction,
        code_hash,
    },
};
use ethrex_rlp::{decode::RLPDecode, encode::RLPEncode};
use ethrex_trie::{EMPTY_TRIE_HASH, Nibbles, NodeHash, Trie, TrieLogger, TrieNode, TrieWitness};
use sha3::{Digest, Keccak256};
use std::{
    collections::{BTreeMap, HashMap},
    path::Path,
    sync::Arc,
};
use tracing::{debug, error, info, instrument};
/// Number of state trie segments to fetch concurrently during state sync
pub const STATE_TRIE_SEGMENTS: usize = 2;
/// Maximum amount of reads from the snapshot in a single transaction to avoid performance hits due to long-living reads
/// This will always be the amount yielded by snapshot reads unless there are less elements left
pub const MAX_SNAPSHOT_READS: usize = 100;

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

const CHAIN_DATA: &str = "chain_data";
const ACCOUNT_CODES: &str = "account_codes";
const BODIES: &str = "bodies";
const BLOCK_NUMBERS: &str = "block_numbers";
const CANONICAL_BLOCK_HASHES: &str = "canonical_block_hashes";
const HEADERS: &str = "headers";
const PENDING_BLOCKS: &str = "pending_blocks";
const TRANSACTION_LOCATIONS: &str = "transaction_locations";
const RECEIPTS: &str = "receipts";
const SNAP_STATE: &str = "snap_state";
const INVALID_CHAINS: &str = "invalid_chains";
const STATE_TRIE_NODES: &str = "state_trie_nodes";
const STORAGE_TRIE_NODES: &str = "storage_trie_nodes";

type AccountStorage = (H256, Vec<(NodeHash, Vec<u8>)>);

#[derive(Clone, Debug)]
pub struct StoreEngine {
    backend: Arc<dyn StorageBackend>,
}

impl StoreEngine {
    pub fn new(backend: Arc<dyn StorageBackend>) -> Result<Self, StoreError> {
        backend.create_table(CHAIN_DATA, TableOptions { dupsort: false })?;
        backend.create_table(ACCOUNT_CODES, TableOptions { dupsort: false })?;
        backend.create_table(BODIES, TableOptions { dupsort: false })?;
        backend.create_table(BLOCK_NUMBERS, TableOptions { dupsort: false })?;
        backend.create_table(CANONICAL_BLOCK_HASHES, TableOptions { dupsort: false })?;
        backend.create_table(HEADERS, TableOptions { dupsort: false })?;
        backend.create_table(PENDING_BLOCKS, TableOptions { dupsort: false })?;
        backend.create_table(TRANSACTION_LOCATIONS, TableOptions { dupsort: false })?;
        backend.create_table(RECEIPTS, TableOptions { dupsort: false })?;
        backend.create_table(SNAP_STATE, TableOptions { dupsort: false })?;
        backend.create_table(INVALID_CHAINS, TableOptions { dupsort: false })?;
        backend.create_table(STATE_TRIE_NODES, TableOptions { dupsort: false })?;
        backend.create_table(STORAGE_TRIE_NODES, TableOptions { dupsort: false })?;
        Ok(Self { backend })
    }

    /// Store changes in a batch from a vec of blocks
    pub async fn apply_updates(&self, update_batch: UpdateBatch) -> Result<(), StoreError> {
        let db = self.backend.clone();
        tokio::task::spawn_blocking(move || {
            let _span = tracing::trace_span!("Block DB update").entered();
            let txn = db.begin_write()?;

            for (node_hash, node_data) in update_batch.account_updates {
                txn.put(STATE_TRIE_NODES, node_hash.as_ref(), node_data.as_slice())?;
            }

            for (hash_address, nodes) in update_batch.storage_updates {
                for (node_hash, node_data) in nodes {
                    // Key: hash_address + node_hash
                    let mut key = Vec::with_capacity(64);
                    key.extend_from_slice(hash_address.as_bytes());
                    key.extend_from_slice(node_hash.as_ref());
                    txn.put(STORAGE_TRIE_NODES, key.as_slice(), node_data.as_slice())?;
                }
            }

            for (code_hash, code) in update_batch.code_updates {
                let code_value = AccountCodeRLP::from(code).bytes().clone();
                txn.put(ACCOUNT_CODES, code_hash.as_bytes(), code_value.as_slice())?;
            }

            for (block_hash, receipts) in update_batch.receipts {
                for (index, receipt) in receipts.into_iter().enumerate() {
                    let key = (block_hash, index as u64).encode_to_vec();
                    let value = receipt.encode_to_vec();
                    txn.put(RECEIPTS, key.as_slice(), value.as_slice())?;
                }
            }

            for block in update_batch.blocks {
                let block_number = block.header.number;
                let block_hash = block.hash();

                let header_value = BlockHeaderRLP::from(block.header.clone()).bytes().clone();
                txn.put(HEADERS, block_hash.as_bytes(), header_value.as_slice())?;

                let body_value = BlockBodyRLP::from(block.body.clone()).bytes().clone();
                txn.put(BODIES, block_hash.as_bytes(), body_value.as_slice())?;

                txn.put(
                    BLOCK_NUMBERS,
                    block_hash.as_bytes(),
                    &block_number.to_le_bytes(),
                )?;

                for (index, transaction) in block.body.transactions.iter().enumerate() {
                    let mut composite_key = [0; 64];
                    composite_key[..32].copy_from_slice(transaction.hash().as_bytes());
                    composite_key[32..].copy_from_slice(block_hash.as_bytes());
                    let location_value = (block_number, block_hash, index as u64).encode_to_vec();
                    txn.put(
                        TRANSACTION_LOCATIONS,
                        composite_key.as_slice(),
                        location_value.as_slice(),
                    )?;
                }
            }

            txn.commit()
        })
        .await
        .map_err(|e| StoreError::Custom(format!("Task panicked: {}", e)))?
    }

    /// Add a batch of blocks in a single transaction.
    /// This will store -> BlockHeader, BlockBody, BlockTransactions, BlockNumber.
    pub async fn add_blocks(&self, blocks: Vec<Block>) -> Result<(), StoreError> {
        let db = self.backend.clone();
        tokio::task::spawn_blocking(move || {
            let txn = db.begin_write()?;

            // TODO: Same logic in apply_updates
            for block in blocks {
                let block_hash = block.hash();
                let block_number = block.header.number;

                let header_value = BlockHeaderRLP::from(block.header.clone()).bytes().clone();
                txn.put(HEADERS, block_hash.as_bytes(), header_value.as_slice())?;

                let body_value = BlockBodyRLP::from(block.body.clone()).bytes().clone();
                txn.put(BODIES, block_hash.as_bytes(), body_value.as_slice())?;

                txn.put(
                    BLOCK_NUMBERS,
                    block_hash.as_bytes(),
                    &block_number.to_le_bytes(),
                )?;

                for (index, transaction) in block.body.transactions.iter().enumerate() {
                    let mut composite_key = [0; 64];
                    composite_key[..32].copy_from_slice(transaction.hash().as_bytes());
                    composite_key[32..].copy_from_slice(block_hash.as_bytes());
                    let location_value = (block_number, block_hash, index as u64).encode_to_vec();
                    txn.put(
                        TRANSACTION_LOCATIONS,
                        composite_key.as_slice(),
                        location_value.as_slice(),
                    )?;
                }
            }

            txn.commit()
        })
        .await
        .map_err(|e| StoreError::Custom(format!("Task panicked: {}", e)))?
    }

    /// Add block header
    pub async fn add_block_header(
        &self,
        block_hash: BlockHash,
        block_header: BlockHeader,
    ) -> Result<(), StoreError> {
        let txn = self.backend.begin_write()?;
        let header_value = BlockHeaderRLP::from(block_header).bytes().clone();
        txn.put(HEADERS, block_hash.as_bytes(), header_value.as_slice())?;
        txn.commit()
    }

    /// Add a batch of block headers
    pub async fn add_block_headers(
        &self,
        block_headers: Vec<BlockHeader>,
    ) -> Result<(), StoreError> {
        let txn = self.backend.begin_write()?;
        for block_header in block_headers {
            let block_hash = block_header.hash();
            let block_number = block_header.number;
            let header_value = BlockHeaderRLP::from(block_header).bytes().clone();
            txn.put(HEADERS, block_hash.as_bytes(), header_value.as_slice())?;
            txn.put(
                BLOCK_NUMBERS,
                block_hash.as_bytes(),
                &block_number.to_le_bytes(),
            )?;
        }
        txn.commit()
    }

    /// Obtain canonical block header
    pub fn get_block_header(
        &self,
        block_number: BlockNumber,
    ) -> Result<Option<BlockHeader>, StoreError> {
        let Some(block_hash) = self.get_canonical_block_hash_sync(block_number)? else {
            return Ok(None);
        };

        self.backend
            .begin_read()?
            .get(HEADERS, block_hash.as_bytes())?
            .map(|bytes| BlockHeaderRLP::from_bytes(bytes).to())
            .transpose()
            .map_err(StoreError::from)
    }

    /// Add block body
    pub async fn add_block_body(
        &self,
        block_hash: BlockHash,
        block_body: BlockBody,
    ) -> Result<(), StoreError> {
        let txn = self.backend.begin_write()?;
        let body_value = BlockBodyRLP::from(block_body).bytes().clone();
        txn.put(BODIES, block_hash.as_bytes(), body_value.as_slice())?;
        txn.commit()
    }

    /// Obtain canonical block body
    pub async fn get_block_body(
        &self,
        block_number: BlockNumber,
    ) -> Result<Option<BlockBody>, StoreError> {
        let Some(block_hash) = self.get_canonical_block_hash_sync(block_number)? else {
            return Ok(None);
        };

        self.get_block_body_by_hash(block_hash).await
    }

    /// Remove canonical block
    pub async fn remove_block(&self, block_number: BlockNumber) -> Result<(), StoreError> {
        let Some(hash) = self.get_canonical_block_hash_sync(block_number)? else {
            return Ok(());
        };

        let txn = self.backend.begin_write()?;
        txn.delete(
            CANONICAL_BLOCK_HASHES,
            block_number.to_le_bytes().as_slice(),
        )?;
        txn.delete(BODIES, hash.as_bytes())?;
        txn.delete(HEADERS, hash.as_bytes())?;
        txn.delete(BLOCK_NUMBERS, hash.as_bytes())?;
        txn.commit()
    }

    /// Obtain canonical block bodies in from..=to
    pub async fn get_block_bodies(
        &self,
        from: BlockNumber,
        to: BlockNumber,
    ) -> Result<Vec<BlockBody>, StoreError> {
        let numbers: Vec<BlockNumber> = (from..=to).collect();
        let mut block_bodies = Vec::new();

        // FIXME: We are opening two transaction for each iteration
        for number in numbers {
            let Some(hash) = self.get_canonical_block_hash_sync(number)? else {
                return Err(StoreError::Custom(format!(
                    "Block hash not found for number: {number}"
                )));
            };
            let Some(block_body) = self.get_block_body_by_hash(hash).await? else {
                return Err(StoreError::Custom(format!(
                    "Block body not found for hash: {hash}"
                )));
            };

            block_bodies.push(block_body);
        }

        Ok(block_bodies)
    }

    /// Obtain block bodies from a list of hashes
    pub async fn get_block_bodies_by_hash(
        &self,
        hashes: Vec<BlockHash>,
    ) -> Result<Vec<BlockBody>, StoreError> {
        let txn = self.backend.begin_read()?;
        let mut block_bodies = Vec::new();
        for hash in hashes {
            let Some(block_body) = txn
                .get(BODIES, hash.as_bytes())?
                .map(|bytes| BlockBodyRLP::from_bytes(bytes).to())
                .transpose()
                .map_err(StoreError::from)?
            else {
                return Err(StoreError::Custom(format!(
                    "Block body not found for hash: {hash}"
                )));
            };
            block_bodies.push(block_body);
        }
        Ok(block_bodies)
    }

    /// Obtain any block body using the hash
    pub async fn get_block_body_by_hash(
        &self,
        block_hash: BlockHash,
    ) -> Result<Option<BlockBody>, StoreError> {
        let txn = self.backend.begin_read()?;
        txn.get(BODIES, block_hash.as_bytes())?
            .map(|bytes| BlockBodyRLP::from_bytes(bytes).to())
            .transpose()
            .map_err(StoreError::from)
    }

    pub fn get_block_header_by_hash(
        &self,
        block_hash: BlockHash,
    ) -> Result<Option<BlockHeader>, StoreError> {
        let txn = self.backend.begin_read()?;
        let header_value = txn.get(HEADERS, block_hash.as_bytes())?;
        header_value
            .map(|bytes| BlockHeaderRLP::from_bytes(bytes).to())
            .transpose()
            .map_err(StoreError::from)
    }

    pub async fn add_pending_block(&self, block: Block) -> Result<(), StoreError> {
        let block_value = BlockRLP::from(block.clone()).bytes().clone();
        let txn = self.backend.begin_write()?;
        txn.put(
            PENDING_BLOCKS,
            block.hash().as_bytes(),
            block_value.as_slice(),
        )?;
        txn.commit()
    }
    pub async fn get_pending_block(
        &self,
        block_hash: BlockHash,
    ) -> Result<Option<Block>, StoreError> {
        let txn = self.backend.begin_read()?;
        let block_value = txn.get(PENDING_BLOCKS, block_hash.as_bytes())?;
        block_value
            .map(|bytes| BlockRLP::from_bytes(bytes).to())
            .transpose()
            .map_err(StoreError::from)
    }

    /// Add block number for a given hash
    pub async fn add_block_number(
        &self,
        block_hash: BlockHash,
        block_number: BlockNumber,
    ) -> Result<(), StoreError> {
        let number_value = block_number.to_le_bytes();
        let txn = self.backend.begin_write()?;
        txn.put(BLOCK_NUMBERS, block_hash.as_bytes(), &number_value)?;
        txn.commit()
    }

    /// Obtain block number for a given hash
    pub async fn get_block_number(
        &self,
        block_hash: BlockHash,
    ) -> Result<Option<BlockNumber>, StoreError> {
        self.backend
            .begin_read()?
            .get(BLOCK_NUMBERS, block_hash.as_bytes())?
            .map(|bytes| -> Result<BlockNumber, StoreError> {
                let array: [u8; 8] = bytes
                    .try_into()
                    .map_err(|_| StoreError::Custom("Invalid BlockNumber bytes".to_string()))?;
                Ok(BlockNumber::from_le_bytes(array))
            })
            .transpose()
    }

    /// Store transaction location (block number and index of the transaction within the block)
    pub async fn add_transaction_location(
        &self,
        transaction_hash: H256,
        block_number: BlockNumber,
        block_hash: BlockHash,
        index: Index,
    ) -> Result<(), StoreError> {
        // FIXME: Use dupsort table
        let mut composite_key = [0; 64];
        composite_key[..32].copy_from_slice(transaction_hash.as_bytes());
        composite_key[32..].copy_from_slice(block_hash.as_bytes());
        let location_value = (block_number, block_hash, index).encode_to_vec();

        let txn = self.backend.begin_write()?;
        txn.put(
            TRANSACTION_LOCATIONS,
            composite_key.as_slice(),
            location_value.as_slice(),
        )?;
        txn.commit()
    }

    /// Store transaction locations in batch (one db transaction for all)
    pub async fn add_transaction_locations(
        &self,
        locations: Vec<(H256, BlockNumber, BlockHash, Index)>,
    ) -> Result<(), StoreError> {
        let txn = self.backend.begin_write()?;
        let key_values: Vec<([u8; 64], Vec<u8>)> = locations
            .iter()
            .map(|(tx_hash, block_number, block_hash, index)| {
                let mut composite_key = [0; 64];
                composite_key[..32].copy_from_slice(tx_hash.as_bytes());
                composite_key[32..].copy_from_slice(block_hash.as_bytes());
                let location_value = (*block_number, *block_hash, *index).encode_to_vec();
                (composite_key, location_value)
            })
            .collect();

        // TODO: Batch write?
        for (key, value) in key_values {
            txn.put(TRANSACTION_LOCATIONS, key.as_slice(), value.as_slice())?;
        }
        txn.commit()
    }

    // FIXME: Check libmdbx implementation to see if we can replicate it
    /// Obtain transaction location (block hash and index)
    pub async fn get_transaction_location(
        &self,
        transaction_hash: H256,
    ) -> Result<Option<(BlockNumber, BlockHash, Index)>, StoreError> {
        let db = self.backend.clone();
        tokio::task::spawn_blocking(move || {
            let tx_hash_bytes = transaction_hash.as_bytes();
            let tx = db.begin_read()?;

            // Use prefix iterator to find all entries with this transaction hash
            let mut iter = tx.prefix_iterator(TRANSACTION_LOCATIONS, tx_hash_bytes)?;
            let mut transaction_locations = Vec::new();

            while let Some(Ok((key, value))) = iter.next() {
                // Ensure key is exactly tx_hash + block_hash (32 + 32 = 64 bytes)
                // and starts with our exact tx_hash
                if key.len() == 64 && &key[0..32] == tx_hash_bytes {
                    transaction_locations.push(<(BlockNumber, BlockHash, Index)>::decode(&value)?);
                }
            }

            if transaction_locations.is_empty() {
                return Ok(None);
            }

            // If there are multiple locations, filter by the canonical chain
            for (block_number, block_hash, index) in transaction_locations {
                let canonical_hash = {
                    tx.get(
                        CANONICAL_BLOCK_HASHES,
                        block_number.to_le_bytes().as_slice(),
                    )?
                    .and_then(|bytes| BlockHashRLP::from_bytes(bytes).to().ok())
                };

                if canonical_hash == Some(block_hash) {
                    return Ok(Some((block_number, block_hash, index)));
                }
            }

            Ok(None)
        })
        .await
        .map_err(|e| StoreError::Custom(format!("Task panicked: {}", e)))?
    }

    /// Add receipt
    pub async fn add_receipt(
        &self,
        block_hash: BlockHash,
        index: Index,
        receipt: Receipt,
    ) -> Result<(), StoreError> {
        // FIXME: Use dupsort table
        let key = (block_hash, index).encode_to_vec();
        let value = receipt.encode_to_vec();
        let txn = self.backend.begin_write()?;
        txn.put(RECEIPTS, key.as_slice(), value.as_slice())?;
        txn.commit()
    }

    /// Add receipts
    pub async fn add_receipts(
        &self,
        block_hash: BlockHash,
        receipts: Vec<Receipt>,
    ) -> Result<(), StoreError> {
        let txn = self.backend.begin_write()?;
        for (index, receipt) in receipts.into_iter().enumerate() {
            let key = (block_hash, index as u64).encode_to_vec();
            let value = receipt.encode_to_vec();
            txn.put(RECEIPTS, key.as_slice(), value.as_slice())?;
        }
        txn.commit()
    }

    /// Obtain receipt by block hash and index
    pub async fn get_receipt(
        &self,
        block_hash: BlockHash,
        index: Index,
    ) -> Result<Option<Receipt>, StoreError> {
        let key = (block_hash, index).encode_to_vec();
        let txn = self.backend.begin_read()?;
        txn.get(RECEIPTS, key.as_slice())?
            .map(|bytes| Receipt::decode(bytes.as_slice()))
            .transpose()
            .map_err(StoreError::from)
    }

    /// Obtain account code via code hash
    pub fn get_account_code(&self, code_hash: H256) -> Result<Option<Bytes>, StoreError> {
        self.backend
            .begin_read()?
            .get(ACCOUNT_CODES, code_hash.as_bytes())?
            .map(|bytes| AccountCodeRLP::from_bytes(bytes).to())
            .transpose()
            .map_err(StoreError::from)
    }

    /// Add account code
    pub async fn add_account_code(&self, code_hash: H256, code: Bytes) -> Result<(), StoreError> {
        let code_value = AccountCodeRLP::from(code).bytes().clone();
        let txn = self.backend.begin_write()?;
        txn.put(ACCOUNT_CODES, code_hash.as_bytes(), code_value.as_slice())?;
        txn.commit()
    }

    /// Clears all checkpoint data created during the last snap sync
    pub async fn clear_snap_state(&self) -> Result<(), StoreError> {
        // FIXME: We need a way to iterate over a table or just delete the entire table
        let db = self.backend.clone();
        tokio::task::spawn_blocking(move || db.clear_table(SNAP_STATE))
            .await
            .map_err(|e| StoreError::Custom(format!("Task panicked: {}", e)))?
    }

    pub async fn get_transaction_by_hash(
        &self,
        transaction_hash: H256,
    ) -> Result<Option<Transaction>, StoreError> {
        let (_block_number, block_hash, index) =
            match self.get_transaction_location(transaction_hash).await? {
                Some(location) => location,
                None => return Ok(None),
            };
        self.get_transaction_by_location(block_hash, index).await
    }

    pub async fn get_transaction_by_location(
        &self,
        block_hash: H256,
        index: u64,
    ) -> Result<Option<Transaction>, StoreError> {
        let block_body = match self.get_block_body_by_hash(block_hash).await? {
            Some(body) => body,
            None => return Ok(None),
        };
        let index: usize = index.try_into()?;
        Ok(block_body.transactions.get(index).cloned())
    }

    pub async fn get_block_by_hash(
        &self,
        block_hash: BlockHash,
    ) -> Result<Option<Block>, StoreError> {
        let header = match self.get_block_header_by_hash(block_hash)? {
            Some(header) => header,
            None => return Ok(None),
        };
        let body = match self.get_block_body_by_hash(block_hash).await? {
            Some(body) => body,
            None => return Ok(None),
        };
        Ok(Some(Block::new(header, body)))
    }

    pub async fn get_block_by_number(
        &self,
        block_number: BlockNumber,
    ) -> Result<Option<Block>, StoreError> {
        let Some(block_hash) = self.get_canonical_block_hash(block_number).await? else {
            return Ok(None);
        };
        self.get_block_by_hash(block_hash).await
    }

    // Get the canonical block hash for a given block number.
    pub async fn get_canonical_block_hash(
        &self,
        block_number: BlockNumber,
    ) -> Result<Option<BlockHash>, StoreError> {
        self.backend
            .begin_read()?
            .get(
                CANONICAL_BLOCK_HASHES,
                block_number.to_le_bytes().as_slice(),
            )?
            .map(|bytes| BlockHashRLP::from_bytes(bytes).to())
            .transpose()
            .map_err(StoreError::from)
    }

    /// Stores the chain configuration values, should only be called once after reading the genesis file
    /// Ignores previously stored values if present
    pub async fn set_chain_config(&self, chain_config: &ChainConfig) -> Result<(), StoreError> {
        let key = [ChainDataIndex::ChainConfig as u8];
        let value = serde_json::to_string(chain_config)
            .map_err(|_| StoreError::Custom("Failed to serialize chain config".to_string()))?
            .into_bytes();
        let txn = self.backend.begin_write()?;
        txn.put(CHAIN_DATA, &key, &value)?;
        txn.commit()
    }

    /// Update earliest block number
    pub async fn update_earliest_block_number(
        &self,
        block_number: BlockNumber,
    ) -> Result<(), StoreError> {
        let key = [ChainDataIndex::EarliestBlockNumber as u8];
        let value = block_number.to_le_bytes();
        let txn = self.backend.begin_write()?;
        txn.put(CHAIN_DATA, &key, &value)?;
        txn.commit()
    }

    /// Obtain earliest block number
    pub async fn get_earliest_block_number(&self) -> Result<Option<BlockNumber>, StoreError> {
        let key = [ChainDataIndex::EarliestBlockNumber as u8];
        let txn = self.backend.begin_read()?;
        let value = txn.get(CHAIN_DATA, &key)?;
        value
            .map(|bytes| -> Result<BlockNumber, StoreError> {
                let array: [u8; 8] = bytes
                    .try_into()
                    .map_err(|_| StoreError::Custom("Invalid BlockNumber bytes".to_string()))?;
                Ok(BlockNumber::from_le_bytes(array))
            })
            .transpose()
    }

    /// Obtain finalized block number
    pub async fn get_finalized_block_number(&self) -> Result<Option<BlockNumber>, StoreError> {
        let key = [ChainDataIndex::FinalizedBlockNumber as u8];
        let txn = self.backend.begin_read()?;
        let value = txn.get(CHAIN_DATA, &key)?;
        value
            .map(|bytes| -> Result<BlockNumber, StoreError> {
                let array: [u8; 8] = bytes
                    .try_into()
                    .map_err(|_| StoreError::Custom("Invalid BlockNumber bytes".to_string()))?;
                Ok(BlockNumber::from_le_bytes(array))
            })
            .transpose()
    }

    /// Obtain safe block number
    pub async fn get_safe_block_number(&self) -> Result<Option<BlockNumber>, StoreError> {
        let key = [ChainDataIndex::SafeBlockNumber as u8];
        let txn = self.backend.begin_read()?;
        let value = txn.get(CHAIN_DATA, &key)?;
        value
            .map(|bytes| -> Result<BlockNumber, StoreError> {
                let array: [u8; 8] = bytes
                    .try_into()
                    .map_err(|_| StoreError::Custom("Invalid BlockNumber bytes".to_string()))?;
                Ok(BlockNumber::from_le_bytes(array))
            })
            .transpose()
    }

    /// Obtain latest block number
    pub async fn get_latest_block_number(&self) -> Result<Option<BlockNumber>, StoreError> {
        let key = [ChainDataIndex::LatestBlockNumber as u8];
        let txn = self.backend.begin_read()?;
        let value = txn.get(CHAIN_DATA, &key)?;
        value
            .map(|bytes| -> Result<BlockNumber, StoreError> {
                let array: [u8; 8] = bytes
                    .try_into()
                    .map_err(|_| StoreError::Custom("Invalid BlockNumber bytes".to_string()))?;
                Ok(BlockNumber::from_le_bytes(array))
            })
            .transpose()
    }

    /// Update pending block number
    pub async fn update_pending_block_number(
        &self,
        block_number: BlockNumber,
    ) -> Result<(), StoreError> {
        let key = [ChainDataIndex::PendingBlockNumber as u8];
        let value = block_number.to_le_bytes();
        let txn = self.backend.begin_write()?;
        txn.put(CHAIN_DATA, &key, &value)?;
        txn.commit()
    }

    /// Obtain pending block number
    pub async fn get_pending_block_number(&self) -> Result<Option<BlockNumber>, StoreError> {
        let key = [ChainDataIndex::PendingBlockNumber as u8];
        let txn = self.backend.begin_read()?;
        let value = txn.get(CHAIN_DATA, &key)?;
        value
            .map(|bytes| -> Result<BlockNumber, StoreError> {
                let array: [u8; 8] = bytes
                    .try_into()
                    .map_err(|_| StoreError::Custom("Invalid BlockNumber bytes".to_string()))?;
                Ok(BlockNumber::from_le_bytes(array))
            })
            .transpose()
    }

    /// Obtain a storage trie from the given address and storage_root
    /// Doesn't check if the account is stored
    /// Used for internal store operations
    pub fn open_storage_trie(
        &self,
        hashed_address: H256,
        storage_root: H256,
    ) -> Result<Trie, StoreError> {
        let trie_db = BackendTrieDB::new(
            self.backend.clone(),
            STORAGE_TRIE_NODES,
            Some(hashed_address), // Use address as prefix for storage trie
        );
        Ok(Trie::open(Box::new(trie_db), storage_root))
    }

    /// Obtain a state trie from the given state root
    /// Doesn't check if the state root is valid
    /// Used for internal store operations
    pub fn open_state_trie(&self, state_root: H256) -> Result<Trie, StoreError> {
        let trie_db = BackendTrieDB::new(
            self.backend.clone(),
            STATE_TRIE_NODES,
            None, // No prefix for state trie
        );
        Ok(Trie::open(Box::new(trie_db), state_root))
    }

    /// Obtain a state trie locked for reads from the given state root
    /// Doesn't check if the state root is valid
    /// Used for internal store operations
    pub fn open_locked_state_trie(&self, state_root: H256) -> Result<Trie, StoreError> {
        // let trie_db = BackendTrieDB::new(
        //     self.backend.clone(),
        //     STATE_TRIE_NODES,
        //     None, // No address prefix for state trie
        // );
        let lock = self.backend.begin_locked(STATE_TRIE_NODES)?;
        let trie_db = BackendTrieDBLocked::new(
            lock, None, // No address prefix for state trie
        );
        Ok(Trie::open(Box::new(trie_db), state_root))
    }

    /// Obtain a read-locked storage trie from the given address and storage_root
    /// Doesn't check if the account is stored
    /// Used for internal store operations
    pub fn open_locked_storage_trie(
        &self,
        hashed_address: H256,
        storage_root: H256,
    ) -> Result<Trie, StoreError> {
        // let trie_db = BackendTrieDB::new(
        //     self.backend.clone(),
        //     STORAGE_TRIE_NODES,
        //     Some(hashed_address), // Use address as prefix for storage trie
        // );
        let lock = self.backend.begin_locked(STORAGE_TRIE_NODES)?;
        let trie_db = BackendTrieDBLocked::new(
            lock,
            Some(hashed_address), // Use address as prefix for storage trie
        );
        Ok(Trie::open(Box::new(trie_db), storage_root))
    }

    pub async fn forkchoice_update(
        &self,
        new_canonical_blocks: Option<Vec<(BlockNumber, BlockHash)>>,
        head_number: BlockNumber,
        head_hash: BlockHash,
        safe: Option<BlockNumber>,
        finalized: Option<BlockNumber>,
    ) -> Result<(), StoreError> {
        // FIXME: Create a new transaction
        let latest = self.get_latest_block_number().await?.unwrap_or(0);
        let db = self.backend.clone();
        tokio::task::spawn_blocking(move || {
            let tx = db.begin_write()?;

            if let Some(canonical_blocks) = new_canonical_blocks {
                for (block_number, block_hash) in canonical_blocks {
                    let head_value = BlockHashRLP::from(block_hash).bytes().clone();
                    tx.put(
                        CANONICAL_BLOCK_HASHES,
                        block_number.to_le_bytes().as_slice(),
                        head_value.as_slice(),
                    )?;
                }
            }

            for number in (head_number + 1)..(latest + 1) {
                tx.delete(CANONICAL_BLOCK_HASHES, number.to_le_bytes().as_slice())?;
            }

            // Make head canonical
            let head_value = BlockHashRLP::from(head_hash).bytes().clone();
            tx.put(
                CANONICAL_BLOCK_HASHES,
                head_number.to_le_bytes().as_slice(),
                head_value.as_slice(),
            )?;

            // Update chain data
            let latest_key = [ChainDataIndex::LatestBlockNumber as u8];
            tx.put(CHAIN_DATA, &latest_key, &head_number.to_le_bytes())?;

            if let Some(finalized) = finalized {
                tx.put(
                    CHAIN_DATA,
                    &[ChainDataIndex::FinalizedBlockNumber as u8],
                    finalized.to_le_bytes().as_slice(),
                )?;
            }

            if let Some(safe) = safe {
                tx.put(
                    CHAIN_DATA,
                    &[ChainDataIndex::SafeBlockNumber as u8],
                    safe.to_le_bytes().as_slice(),
                )?;
            }

            tx.commit()
        })
        .await
        .map_err(|e| StoreError::Custom(format!("Task panicked: {}", e)))?
    }

    pub fn get_receipts_for_block(
        &self,
        block_hash: &BlockHash,
    ) -> Result<Vec<Receipt>, StoreError> {
        let mut receipts = Vec::new();
        let mut index = 0u64;

        let txn = self.backend.begin_read()?;
        loop {
            let key = (*block_hash, index).encode_to_vec();
            match txn.get(RECEIPTS, key.as_slice())? {
                Some(receipt_bytes) => {
                    let receipt = Receipt::decode(receipt_bytes.as_slice())?;
                    receipts.push(receipt);
                    index += 1;
                }
                None => break,
            }
        }

        Ok(receipts)
    }

    // Snap State methods

    /// Sets the hash of the last header downloaded during a snap sync
    pub async fn set_header_download_checkpoint(
        &self,
        block_hash: BlockHash,
    ) -> Result<(), StoreError> {
        let key = [SnapStateIndex::HeaderDownloadCheckpoint as u8];
        let value = block_hash.encode_to_vec();
        let txn = self.backend.begin_write()?;
        txn.put(SNAP_STATE, &key, &value)?;
        txn.commit()
    }

    /// Gets the hash of the last header downloaded during a snap sync
    pub async fn get_header_download_checkpoint(&self) -> Result<Option<BlockHash>, StoreError> {
        let key = [SnapStateIndex::HeaderDownloadCheckpoint as u8];
        self.backend
            .begin_read()?
            .get(SNAP_STATE, &key)?
            .map(|bytes| BlockHashRLP::from_bytes(bytes).to())
            .transpose()
            .map_err(StoreError::from)
    }

    /// Sets the last key fetched from the state trie being fetched during snap sync
    pub async fn set_state_trie_key_checkpoint(
        &self,
        last_keys: [H256; STATE_TRIE_SEGMENTS],
    ) -> Result<(), StoreError> {
        let key = [SnapStateIndex::StateTrieKeyCheckpoint as u8];
        let value = last_keys.to_vec().encode_to_vec();
        let txn = self.backend.begin_write()?;
        txn.put(SNAP_STATE, &key, &value)?;
        txn.commit()
    }

    /// Gets the last key fetched from the state trie being fetched during snap sync
    pub async fn get_state_trie_key_checkpoint(
        &self,
    ) -> Result<Option<[H256; STATE_TRIE_SEGMENTS]>, StoreError> {
        let key = [SnapStateIndex::StateTrieKeyCheckpoint as u8];
        let txn = self.backend.begin_read()?;
        match txn.get(SNAP_STATE, &key)? {
            Some(keys_bytes) => {
                let keys_vec: Vec<H256> = Vec::<H256>::decode(keys_bytes.as_slice())?;
                if keys_vec.len() == STATE_TRIE_SEGMENTS {
                    let mut keys_array = [H256::zero(); STATE_TRIE_SEGMENTS];
                    keys_array.copy_from_slice(&keys_vec);
                    Ok(Some(keys_array))
                } else {
                    Err(StoreError::Custom("Invalid array size".to_string()))
                }
            }
            None => Ok(None),
        }
    }

    /// Sets the state trie paths in need of healing
    pub async fn set_state_heal_paths(
        &self,
        paths: Vec<(Nibbles, H256)>,
    ) -> Result<(), StoreError> {
        let key = [SnapStateIndex::StateHealPaths as u8];
        let value = paths.encode_to_vec();
        let txn = self.backend.begin_write()?;
        txn.put(SNAP_STATE, &key, &value)?;
        txn.commit()
    }

    /// Gets the state trie paths in need of healing
    pub async fn get_state_heal_paths(&self) -> Result<Option<Vec<(Nibbles, H256)>>, StoreError> {
        let key = [SnapStateIndex::StateHealPaths as u8];

        self.backend
            .begin_read()?
            .get(SNAP_STATE, &key)?
            .map(|bytes| Vec::<(Nibbles, H256)>::decode(bytes.as_slice()))
            .transpose()
            .map_err(StoreError::from)
    }

    /// Set the latest root of the rebuilt state trie and the last downloaded hashes from each segment
    pub async fn set_state_trie_rebuild_checkpoint(
        &self,
        checkpoint: (H256, [H256; STATE_TRIE_SEGMENTS]),
    ) -> Result<(), StoreError> {
        let key = [SnapStateIndex::StateTrieRebuildCheckpoint as u8];
        let value = (checkpoint.0, checkpoint.1.to_vec()).encode_to_vec();
        let txn = self.backend.begin_write()?;
        txn.put(SNAP_STATE, &key, &value)?;
        txn.commit()
    }

    /// Get the latest root of the rebuilt state trie and the last downloaded hashes from each segment
    pub async fn get_state_trie_rebuild_checkpoint(
        &self,
    ) -> Result<Option<(H256, [H256; STATE_TRIE_SEGMENTS])>, StoreError> {
        let key = [SnapStateIndex::StateTrieRebuildCheckpoint as u8];
        let txn = self.backend.begin_read()?;
        match txn.get(SNAP_STATE, &key)? {
            Some(bytes) => {
                let (root, keys_vec): (H256, Vec<H256>) =
                    <(H256, Vec<H256>)>::decode(bytes.as_slice())?;
                if keys_vec.len() == STATE_TRIE_SEGMENTS {
                    let mut keys_array = [H256::zero(); STATE_TRIE_SEGMENTS];
                    keys_array.copy_from_slice(&keys_vec);
                    Ok(Some((root, keys_array)))
                } else {
                    Err(StoreError::Custom("Invalid array size".to_string()))
                }
            }
            None => Ok(None),
        }
    }

    /// Get the accont hashes and roots of the storage tries awaiting rebuild
    pub async fn set_storage_trie_rebuild_pending(
        &self,
        pending: Vec<(H256, H256)>,
    ) -> Result<(), StoreError> {
        let key = [SnapStateIndex::StorageTrieRebuildPending as u8];
        let txn = self.backend.begin_write()?;
        txn.put(SNAP_STATE, &key, &pending.encode_to_vec())?;
        txn.commit()
    }

    /// Get the accont hashes and roots of the storage tries awaiting rebuild
    pub async fn get_storage_trie_rebuild_pending(
        &self,
    ) -> Result<Option<Vec<(H256, H256)>>, StoreError> {
        let key = [SnapStateIndex::StorageTrieRebuildPending as u8];
        let txn = self.backend.begin_read()?;
        txn.get(SNAP_STATE, &key)?
            .map(|bytes| Vec::<(H256, H256)>::decode(bytes.as_slice()))
            .transpose()
            .map_err(StoreError::from)
    }

    /// The `forkchoice_update` and `new_payload` methods require the `latest_valid_hash`
    /// when processing an invalid payload. To provide this, we must track invalid chains.
    ///
    /// We only store the last known valid head upon encountering a bad block,
    /// rather than tracking every subsequent invalid block.
    pub async fn set_latest_valid_ancestor(
        &self,
        bad_block: BlockHash,
        latest_valid: BlockHash,
    ) -> Result<(), StoreError> {
        let txn = self.backend.begin_write()?;
        let value = BlockHashRLP::from(latest_valid).bytes().clone();
        txn.put(INVALID_CHAINS, bad_block.as_bytes(), value.as_slice())?;
        txn.commit()
    }

    /// Returns the latest valid ancestor hash for a given invalid block hash.
    /// Used to provide `latest_valid_hash` in the Engine API when processing invalid payloads.
    pub async fn get_latest_valid_ancestor(
        &self,
        block: BlockHash,
    ) -> Result<Option<BlockHash>, StoreError> {
        let txn = self.backend.begin_read()?;
        txn.get(INVALID_CHAINS, block.as_bytes())?
            .map(|bytes| BlockHashRLP::from_bytes(bytes).to())
            .transpose()
            .map_err(StoreError::from)
    }

    /// Obtain block number for a given hash
    pub fn get_block_number_sync(
        &self,
        block_hash: BlockHash,
    ) -> Result<Option<BlockNumber>, StoreError> {
        let txn = self.backend.begin_read()?;
        txn.get(BLOCK_NUMBERS, block_hash.as_bytes())?
            .map(|bytes| -> Result<BlockNumber, StoreError> {
                let array: [u8; 8] = bytes
                    .try_into()
                    .map_err(|_| StoreError::Custom("Invalid BlockNumber bytes".to_string()))?;
                Ok(BlockNumber::from_le_bytes(array))
            })
            .transpose()
    }

    /// Get the canonical block hash for a given block number.
    pub fn get_canonical_block_hash_sync(
        &self,
        block_number: BlockNumber,
    ) -> Result<Option<BlockHash>, StoreError> {
        let txn = self.backend.begin_read()?;
        txn.get(
            CANONICAL_BLOCK_HASHES,
            block_number.to_le_bytes().as_slice(),
        )?
        .map(|bytes| BlockHashRLP::from_bytes(bytes).to())
        .transpose()
        .map_err(StoreError::from)
    }

    pub async fn write_storage_trie_nodes_batch(
        &self,
        storage_trie_nodes: Vec<AccountStorage>,
    ) -> Result<(), StoreError> {
        let txn = self.backend.begin_write()?;
        for (address_hash, nodes) in storage_trie_nodes {
            for (node_hash, node_data) in nodes {
                // Key: address_hash + node_hash
                let mut key = Vec::with_capacity(64);
                key.extend_from_slice(address_hash.as_bytes());
                key.extend_from_slice(node_hash.as_ref());
                txn.put(STORAGE_TRIE_NODES, key.as_slice(), node_data.as_slice())?;
            }
        }
        txn.commit()
    }

    pub async fn write_account_code_batch(
        &self,
        account_codes: Vec<(H256, Bytes)>,
    ) -> Result<(), StoreError> {
        let txn = self.backend.begin_write()?;
        for (code_hash, code) in account_codes {
            let value = AccountCodeRLP::from(code).bytes().clone();
            txn.put(ACCOUNT_CODES, code_hash.as_bytes(), value.as_slice())?;
        }

        txn.commit()
    }
}

// Hash utility functions
pub fn hash_address(address: &ethereum_types::Address) -> Vec<u8> {
    use sha3::{Digest as _, Keccak256};
    Keccak256::new_with_prefix(address.to_fixed_bytes())
        .finalize()
        .to_vec()
}

pub fn hash_key(key: &H256) -> Vec<u8> {
    use sha3::{Digest as _, Keccak256};
    Keccak256::new_with_prefix(key.to_fixed_bytes())
        .finalize()
        .to_vec()
}

pub type StorageTrieNodes = Vec<(H256, Vec<(NodeHash, Vec<u8>)>)>;

// Store wrapper with business logic
#[derive(Debug, Clone)]
pub struct Store {
    engine: Arc<StoreEngine>,
    chain_config: Arc<std::sync::RwLock<ChainConfig>>,
    latest_block_header: Arc<std::sync::RwLock<BlockHeader>>,
}

impl Store {
    pub async fn store_block_updates(&self, update_batch: UpdateBatch) -> Result<(), StoreError> {
        self.engine.apply_updates(update_batch).await
    }

    pub fn new(path: impl AsRef<Path>, engine_type: EngineType) -> Result<Self, StoreError> {
        let path = path.as_ref();

        let store = match engine_type {
            #[cfg(feature = "rocksdb")]
            EngineType::RocksDB => Self {
                engine: Arc::new(StoreEngine::new(RocksDBBackend::open(path)?)?),
                chain_config: Default::default(),
                latest_block_header: Arc::new(std::sync::RwLock::new(BlockHeader::default())),
            },
            #[cfg(feature = "libmdbx")]
            EngineType::Libmdbx => todo!("LIBMDBX not implemented yet"),
            EngineType::InMemory => Self {
                engine: Arc::new(StoreEngine::new(InMemoryBackend::open(path)?)?),
                chain_config: Default::default(),
                latest_block_header: Arc::new(std::sync::RwLock::new(BlockHeader::default())),
            },
        };

        Ok(store)
    }

    pub async fn new_from_genesis(
        store_path: &Path,
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

fn hash_address_fixed(address: &Address) -> H256 {
    H256(
        Keccak256::new_with_prefix(address.to_fixed_bytes())
            .finalize()
            .into(),
    )
}
