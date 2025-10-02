use bytes::Bytes;
use ethrex_common::{
    H256,
    types::{
        Block, BlockBody, BlockHash, BlockHeader, BlockNumber, ChainConfig, Index, Receipt,
        Transaction,
    },
};
use ethrex_rlp::{decode::RLPDecode, encode::RLPEncode};
use ethrex_trie::{Nibbles, Trie};

use crate::{
    STATE_TRIE_SEGMENTS, UpdateBatch,
    api::{
        StorageBackend,
        tables::{
            ACCOUNT_CODES, BLOCK_NUMBERS, BODIES, CANONICAL_BLOCK_HASHES, CHAIN_DATA, HEADERS,
            INVALID_CHAINS, PENDING_BLOCKS, RECEIPTS, SNAP_STATE, STATE_TRIE_NODES,
            STORAGE_TRIE_NODES, TRANSACTION_LOCATIONS,
        },
    },
    error::StoreError,
    rlp::{AccountCodeRLP, BlockBodyRLP, BlockHashRLP, BlockHeaderRLP, BlockRLP},
    store::StorageUpdates,
    trie::{BackendTrieDB, BackendTrieDBLocked},
    utils::{ChainDataIndex, SnapStateIndex},
};
use std::sync::Arc;

#[derive(Clone, Debug)]
pub struct StoreEngine {
    backend: Arc<dyn StorageBackend>,
}

impl StoreEngine {
    pub fn new(backend: Arc<dyn StorageBackend>) -> Result<Self, StoreError> {
        // All required tables are guaranteed to exist after backend.open()
        // No need to create tables here
        Ok(Self { backend })
    }

    /// Store changes in a batch from a vec of blocks
    pub async fn apply_updates(&self, update_batch: UpdateBatch) -> Result<(), StoreError> {
        let db = self.backend.clone();
        tokio::task::spawn_blocking(move || {
            let _span = tracing::trace_span!("Block DB update").entered();
            let mut batch_items = Vec::new();

            // Account updates
            for (node_hash, node_data) in update_batch.account_updates {
                batch_items.push((STATE_TRIE_NODES, node_hash.as_ref().to_vec(), node_data));
            }

            // Storage updates
            for (hash_address, nodes) in update_batch.storage_updates {
                for (node_hash, node_data) in nodes {
                    // Key: hash_address + node_hash
                    let mut key = Vec::with_capacity(64);
                    key.extend_from_slice(hash_address.as_bytes());
                    key.extend_from_slice(node_hash.as_ref());
                    batch_items.push((STORAGE_TRIE_NODES, key, node_data));
                }
            }

            // Code updates
            for (code_hash, code) in update_batch.code_updates {
                let code_value = AccountCodeRLP::from(code).bytes().clone();
                batch_items.push((ACCOUNT_CODES, code_hash.as_bytes().to_vec(), code_value));
            }

            // Receipt updates
            for (block_hash, receipts) in update_batch.receipts {
                for (index, receipt) in receipts.into_iter().enumerate() {
                    let key = (block_hash, index as u64).encode_to_vec();
                    let value = receipt.encode_to_vec();
                    batch_items.push((RECEIPTS, key, value));
                }
            }

            // Block updates
            for block in update_batch.blocks {
                let block_number = block.header.number;
                let block_hash = block.hash();

                let header_value = BlockHeaderRLP::from(block.header.clone()).bytes().clone();
                batch_items.push((HEADERS, block_hash.as_bytes().to_vec(), header_value));

                let body_value = BlockBodyRLP::from(block.body.clone()).bytes().clone();
                batch_items.push((BODIES, block_hash.as_bytes().to_vec(), body_value));

                batch_items.push((
                    BLOCK_NUMBERS,
                    block_hash.as_bytes().to_vec(),
                    block_number.to_le_bytes().to_vec(),
                ));

                for (index, transaction) in block.body.transactions.iter().enumerate() {
                    let mut composite_key = Vec::with_capacity(64);
                    composite_key.extend_from_slice(transaction.hash().as_bytes());
                    composite_key.extend_from_slice(block_hash.as_bytes());
                    let location_value = (block_number, block_hash, index as u64).encode_to_vec();
                    batch_items.push((TRANSACTION_LOCATIONS, composite_key, location_value));
                }
            }

            let mut txn = db.begin_write()?;
            txn.put_batch(batch_items)?;
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
            let mut batch_items: Vec<(&str, Vec<u8>, Vec<u8>)> = Vec::new();

            // TODO: Same logic in apply_updates
            for block in blocks {
                let block_number = block.header.number;
                let block_hash = block.hash();

                let header_value = BlockHeaderRLP::from(block.header.clone()).bytes().clone();
                batch_items.push((HEADERS, block_hash.as_bytes().to_vec(), header_value));

                let body_value = BlockBodyRLP::from(block.body.clone()).bytes().clone();
                batch_items.push((BODIES, block_hash.as_bytes().to_vec(), body_value));

                batch_items.push((
                    BLOCK_NUMBERS,
                    block_hash.as_bytes().to_vec(),
                    block_number.to_le_bytes().to_vec(),
                ));

                for (index, transaction) in block.body.transactions.iter().enumerate() {
                    let mut composite_key = Vec::with_capacity(64);
                    composite_key.extend_from_slice(transaction.hash().as_bytes());
                    composite_key.extend_from_slice(block_hash.as_bytes());
                    let location_value = (block_number, block_hash, index as u64).encode_to_vec();
                    batch_items.push((TRANSACTION_LOCATIONS, composite_key, location_value));
                }
            }

            let mut txn = db.begin_write()?;
            txn.put_batch(batch_items)?;
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
        let header_value = BlockHeaderRLP::from(block_header).bytes().clone();
        self.write_async(HEADERS, block_hash.as_bytes().to_vec(), header_value)
            .await
    }

    /// Add a batch of block headers
    pub async fn add_block_headers(
        &self,
        block_headers: Vec<BlockHeader>,
    ) -> Result<(), StoreError> {
        let mut batch_ops = Vec::new();

        for header in block_headers {
            let block_hash = header.hash();
            let hash_key = block_hash.as_bytes().to_vec();
            let header_value = BlockHeaderRLP::from(header.clone()).bytes().clone();

            batch_ops.push((HEADERS.to_string(), hash_key.clone(), header_value));

            let number_key = header.number.to_le_bytes().to_vec();
            batch_ops.push((BLOCK_NUMBERS.to_string(), hash_key, number_key));
        }

        self.write_batch_async(batch_ops).await
    }

    /// Obtain canonical block header
    pub fn get_block_header(
        &self,
        block_number: BlockNumber,
    ) -> Result<Option<BlockHeader>, StoreError> {
        let Some(block_hash) = self.get_canonical_block_hash_sync(block_number)? else {
            return Ok(None);
        };

        self.get_block_header_by_hash(block_hash)
    }

    /// Add block body
    pub async fn add_block_body(
        &self,
        block_hash: BlockHash,
        block_body: BlockBody,
    ) -> Result<(), StoreError> {
        let body_value = BlockBodyRLP::from(block_body).bytes().clone();
        self.write_async(BODIES, block_hash.as_bytes().to_vec(), body_value)
            .await
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

        let backend = self.backend.clone();
        tokio::task::spawn_blocking(move || {
            let mut txn = backend.begin_write()?;
            txn.delete(
                CANONICAL_BLOCK_HASHES,
                block_number.to_le_bytes().as_slice(),
            )?;
            txn.delete(BODIES, hash.as_bytes())?;
            txn.delete(HEADERS, hash.as_bytes())?;
            txn.delete(BLOCK_NUMBERS, hash.as_bytes())?;
            txn.commit()
        })
        .await
        .map_err(|e| StoreError::Custom(format!("Task panicked: {}", e)))?
    }

    /// Obtain canonical block bodies in from..=to
    pub async fn get_block_bodies(
        &self,
        from: BlockNumber,
        to: BlockNumber,
    ) -> Result<Vec<BlockBody>, StoreError> {
        // TODO: Implement read bulk
        let backend = self.backend.clone();
        tokio::task::spawn_blocking(move || {
            let numbers: Vec<BlockNumber> = (from..=to).collect();
            let mut block_bodies = Vec::new();

            let txn = backend.begin_read()?;
            for number in numbers {
                let Some(hash) = txn
                    .get(CANONICAL_BLOCK_HASHES, number.to_le_bytes().as_slice())?
                    .map(|bytes| BlockHashRLP::from_bytes(bytes).to())
                    .transpose()
                    .map_err(StoreError::from)?
                else {
                    return Err(StoreError::Custom(format!(
                        "Block hash not found for number: {number}"
                    )));
                };
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
        })
        .await
        .map_err(|e| StoreError::Custom(format!("Task panicked: {}", e)))?
    }

    /// Obtain block bodies from a list of hashes
    pub async fn get_block_bodies_by_hash(
        &self,
        hashes: Vec<BlockHash>,
    ) -> Result<Vec<BlockBody>, StoreError> {
        let backend = self.backend.clone();
        // TODO: Implement read bulk
        tokio::task::spawn_blocking(move || {
            let txn = backend.begin_read()?;
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
        })
        .await
        .map_err(|e| StoreError::Custom(format!("Task panicked: {}", e)))?
    }

    /// Obtain any block body using the hash
    pub async fn get_block_body_by_hash(
        &self,
        block_hash: BlockHash,
    ) -> Result<Option<BlockBody>, StoreError> {
        self.read_async(BODIES, block_hash.as_bytes().to_vec())
            .await?
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
        self.write_async(
            PENDING_BLOCKS,
            block.hash().as_bytes().to_vec(),
            block_value,
        )
        .await
    }
    pub async fn get_pending_block(
        &self,
        block_hash: BlockHash,
    ) -> Result<Option<Block>, StoreError> {
        self.read_async(PENDING_BLOCKS, block_hash.as_bytes().to_vec())
            .await?
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
        let number_value = block_number.to_le_bytes().to_vec();
        self.write_async(BLOCK_NUMBERS, block_hash.as_bytes().to_vec(), number_value)
            .await
    }

    /// Obtain block number for a given hash
    pub async fn get_block_number(
        &self,
        block_hash: BlockHash,
    ) -> Result<Option<BlockNumber>, StoreError> {
        self.read_async(BLOCK_NUMBERS, block_hash.as_bytes().to_vec())
            .await?
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
        let mut composite_key = Vec::with_capacity(64);
        composite_key.extend_from_slice(transaction_hash.as_bytes());
        composite_key.extend_from_slice(block_hash.as_bytes());
        let location_value = (block_number, block_hash, index).encode_to_vec();

        self.write_async(TRANSACTION_LOCATIONS, composite_key, location_value)
            .await
    }

    /// Store transaction locations in batch (one db transaction for all)
    pub async fn add_transaction_locations(
        &self,
        locations: Vec<(H256, BlockNumber, BlockHash, Index)>,
    ) -> Result<(), StoreError> {
        let batch_items: Vec<(String, Vec<u8>, Vec<u8>)> = locations
            .iter()
            .map(|(tx_hash, block_number, block_hash, index)| {
                let mut composite_key = Vec::with_capacity(64);
                composite_key.extend_from_slice(tx_hash.as_bytes());
                composite_key.extend_from_slice(block_hash.as_bytes());
                let location_value = (*block_number, *block_hash, *index).encode_to_vec();
                (
                    TRANSACTION_LOCATIONS.to_string(),
                    composite_key,
                    location_value,
                )
            })
            .collect();

        self.write_batch_async(batch_items).await
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
        self.write_async(RECEIPTS, key, value).await
    }

    /// Add receipts
    pub async fn add_receipts(
        &self,
        block_hash: BlockHash,
        receipts: Vec<Receipt>,
    ) -> Result<(), StoreError> {
        let batch_items: Vec<(String, Vec<u8>, Vec<u8>)> = receipts
            .into_iter()
            .enumerate()
            .map(|(index, receipt)| {
                let key = (block_hash, index as u64).encode_to_vec();
                let value = receipt.encode_to_vec();
                (RECEIPTS.to_string(), key, value)
            })
            .collect();
        self.write_batch_async(batch_items).await
    }

    /// Obtain receipt by block hash and index
    pub async fn get_receipt(
        &self,
        block_hash: BlockHash,
        index: Index,
    ) -> Result<Option<Receipt>, StoreError> {
        let key = (block_hash, index).encode_to_vec();
        self.read_async(RECEIPTS, key)
            .await?
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
        self.write_async(ACCOUNT_CODES, code_hash.as_bytes().to_vec(), code_value)
            .await
    }

    /// Clears all checkpoint data created during the last snap sync
    pub async fn clear_snap_state(&self) -> Result<(), StoreError> {
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
        let backend = self.backend.clone();
        tokio::task::spawn_blocking(move || {
            backend
                .begin_read()?
                .get(
                    CANONICAL_BLOCK_HASHES,
                    block_number.to_le_bytes().as_slice(),
                )?
                .map(|bytes| BlockHashRLP::from_bytes(bytes).to())
                .transpose()
                .map_err(StoreError::from)
        })
        .await
        .map_err(|e| StoreError::Custom(format!("Task panicked: {}", e)))?
    }

    /// Stores the chain configuration values, should only be called once after reading the genesis file
    /// Ignores previously stored values if present
    pub async fn set_chain_config(&self, chain_config: &ChainConfig) -> Result<(), StoreError> {
        let key = vec![ChainDataIndex::ChainConfig as u8];
        let value = serde_json::to_string(chain_config)
            .map_err(|_| StoreError::Custom("Failed to serialize chain config".to_string()))?
            .into_bytes();
        self.write_async(CHAIN_DATA, key, value).await
    }

    /// Update earliest block number
    pub async fn update_earliest_block_number(
        &self,
        block_number: BlockNumber,
    ) -> Result<(), StoreError> {
        let key = vec![ChainDataIndex::EarliestBlockNumber as u8];
        let value = block_number.to_le_bytes().to_vec();
        self.write_async(CHAIN_DATA, key, value).await
    }

    /// Obtain earliest block number
    pub async fn get_earliest_block_number(&self) -> Result<Option<BlockNumber>, StoreError> {
        let key = vec![ChainDataIndex::EarliestBlockNumber as u8];
        self.read_async(CHAIN_DATA, key)
            .await?
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
        let key = vec![ChainDataIndex::FinalizedBlockNumber as u8];
        self.read_async(CHAIN_DATA, key)
            .await?
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
        let key = vec![ChainDataIndex::SafeBlockNumber as u8];
        self.read_async(CHAIN_DATA, key)
            .await?
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
        let key = vec![ChainDataIndex::LatestBlockNumber as u8];
        self.read_async(CHAIN_DATA, key)
            .await?
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
        let key = vec![ChainDataIndex::PendingBlockNumber as u8];
        let value = block_number.to_le_bytes().to_vec();
        self.write_async(CHAIN_DATA, key, value).await
    }

    /// Obtain pending block number
    pub async fn get_pending_block_number(&self) -> Result<Option<BlockNumber>, StoreError> {
        let key = vec![ChainDataIndex::PendingBlockNumber as u8];
        self.read_async(CHAIN_DATA, key)
            .await?
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
        let latest = self.get_latest_block_number().await?.unwrap_or(0);
        let db = self.backend.clone();
        tokio::task::spawn_blocking(move || {
            let mut txn = db.begin_write()?;
            let mut batch_items = Vec::new();

            if let Some(canonical_blocks) = new_canonical_blocks {
                for (block_number, block_hash) in canonical_blocks {
                    let head_value = BlockHashRLP::from(block_hash).bytes().clone();
                    batch_items.push((
                        CANONICAL_BLOCK_HASHES,
                        block_number.to_le_bytes().to_vec(),
                        head_value,
                    ));
                }
            }

            for number in (head_number + 1)..(latest + 1) {
                txn.delete(CANONICAL_BLOCK_HASHES, number.to_le_bytes().as_slice())?;
            }

            // Make head canonical
            let head_value = BlockHashRLP::from(head_hash).bytes().clone();
            batch_items.push((
                CANONICAL_BLOCK_HASHES,
                head_number.to_le_bytes().to_vec(),
                head_value,
            ));

            // Update chain data
            let latest_key = [ChainDataIndex::LatestBlockNumber as u8];
            batch_items.push((
                CHAIN_DATA,
                latest_key.to_vec(),
                head_number.to_le_bytes().to_vec(),
            ));

            if let Some(finalized) = finalized {
                batch_items.push((
                    CHAIN_DATA,
                    vec![ChainDataIndex::FinalizedBlockNumber as u8],
                    finalized.to_le_bytes().to_vec(),
                ));
            }

            if let Some(safe) = safe {
                batch_items.push((
                    CHAIN_DATA,
                    vec![ChainDataIndex::SafeBlockNumber as u8],
                    safe.to_le_bytes().to_vec(),
                ));
            }

            txn.put_batch(batch_items)?;
            // This commits is used since we deleted some items. We could have a better way to do this.
            // Accept put and delete in the same batch.
            txn.commit()
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
        let key = vec![SnapStateIndex::HeaderDownloadCheckpoint as u8];
        let value = block_hash.encode_to_vec();
        self.write_async(SNAP_STATE, key, value).await
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
        let key = vec![SnapStateIndex::StateTrieKeyCheckpoint as u8];
        let value = last_keys.to_vec().encode_to_vec();
        self.write_async(SNAP_STATE, key, value).await
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
        let key = vec![SnapStateIndex::StateHealPaths as u8];
        let value = paths.encode_to_vec();
        self.write_async(SNAP_STATE, key, value).await
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
        let key = vec![SnapStateIndex::StateTrieRebuildCheckpoint as u8];
        let value = (checkpoint.0, checkpoint.1.to_vec()).encode_to_vec();
        self.write_async(SNAP_STATE, key, value).await
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
        let key = vec![SnapStateIndex::StorageTrieRebuildPending as u8];
        let value = pending.encode_to_vec();
        self.write_async(SNAP_STATE, key, value).await
    }

    /// Get the accont hashes and roots of the storage tries awaiting rebuild
    pub async fn get_storage_trie_rebuild_pending(
        &self,
    ) -> Result<Option<Vec<(H256, H256)>>, StoreError> {
        let key = vec![SnapStateIndex::StorageTrieRebuildPending as u8];
        self.read_async(SNAP_STATE, key)
            .await?
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
        let value = BlockHashRLP::from(latest_valid).bytes().clone();
        self.write_async(INVALID_CHAINS, bad_block.as_bytes().to_vec(), value)
            .await
    }

    /// Returns the latest valid ancestor hash for a given invalid block hash.
    /// Used to provide `latest_valid_hash` in the Engine API when processing invalid payloads.
    pub async fn get_latest_valid_ancestor(
        &self,
        block: BlockHash,
    ) -> Result<Option<BlockHash>, StoreError> {
        self.read_async(INVALID_CHAINS, block.as_bytes().to_vec())
            .await?
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
        storage_trie_nodes: StorageUpdates,
    ) -> Result<(), StoreError> {
        let mut batch_items = Vec::new();
        for (address_hash, nodes) in storage_trie_nodes {
            for (node_hash, node_data) in nodes {
                let mut key = Vec::with_capacity(64);
                key.extend_from_slice(address_hash.as_bytes());
                key.extend_from_slice(node_hash.as_ref());
                batch_items.push((STORAGE_TRIE_NODES.to_string(), key, node_data));
            }
        }

        self.write_batch_async(batch_items).await
    }

    pub async fn write_account_code_batch(
        &self,
        account_codes: Vec<(H256, Bytes)>,
    ) -> Result<(), StoreError> {
        let mut batch_items = Vec::new();
        for (code_hash, code) in account_codes {
            let value = AccountCodeRLP::from(code).bytes().clone();
            batch_items.push((
                ACCOUNT_CODES.to_string(),
                code_hash.as_bytes().to_vec(),
                value,
            ));
        }

        self.write_batch_async(batch_items).await
    }

    // Helper methods for async operations with spawn_blocking
    // These methods ensure RocksDB I/O doesn't block the tokio runtime

    /// Helper method for async writes
    /// Spawns blocking task to avoid blocking tokio runtime
    async fn write_async(
        &self,
        table: &str,
        key: Vec<u8>,
        value: Vec<u8>,
    ) -> Result<(), StoreError> {
        let backend = self.backend.clone();
        let table = table.to_string();

        tokio::task::spawn_blocking(move || {
            let mut txn = backend.begin_write()?;
            txn.put(&table, &key, &value)?;
            txn.commit()
        })
        .await
        .map_err(|e| StoreError::Custom(format!("Task panicked: {}", e)))?
    }

    /// Helper method for async reads
    /// Spawns blocking task to avoid blocking tokio runtime
    pub async fn read_async(
        &self,
        table: &str,
        key: Vec<u8>,
    ) -> Result<Option<Vec<u8>>, StoreError> {
        let backend = self.backend.clone();
        let table = table.to_string();

        tokio::task::spawn_blocking(move || {
            let txn = backend.begin_read()?;
            txn.get(&table, &key)
        })
        .await
        .map_err(|e| StoreError::Custom(format!("Task panicked: {}", e)))?
    }

    /// Helper method for batch writes
    /// Spawns blocking task to avoid blocking tokio runtime
    /// This is the most important optimization for healing performance
    pub async fn write_batch_async(
        &self,
        batch_ops: Vec<(String, Vec<u8>, Vec<u8>)>,
    ) -> Result<(), StoreError> {
        let backend = self.backend.clone();

        tokio::task::spawn_blocking(move || {
            // Convert String to &str for put_batch
            let batch_refs: Vec<(&str, Vec<u8>, Vec<u8>)> = batch_ops
                .iter()
                .map(|(table, key, value)| (table.as_str(), key.clone(), value.clone()))
                .collect();

            let mut txn = backend.begin_write()?;
            txn.put_batch(batch_refs)?;
            txn.commit()
        })
        .await
        .map_err(|e| StoreError::Custom(format!("Task panicked: {}", e)))?
    }
}
