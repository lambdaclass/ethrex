use crate::rlp::{AccountCodeRLP, BlockBodyRLP, BlockHashRLP, BlockHeaderRLP, BlockRLP};
use crate::utils::SnapStateIndex;
use crate::v2::backend::{
    StorageBackend, StorageBackendLockedTrieDB, StorageBackendTrieDB, StorageError,
};
use crate::v2::schema::{DBTable, SchemaRegistry, TableBatchOp};
use crate::{UpdateBatch, store::STATE_TRIE_SEGMENTS, utils::ChainDataIndex};

use bytes::Bytes;
use ethereum_types::{H256, U256};
use ethrex_common::types::{
    Block, BlockBody, BlockHash, BlockHeader, BlockNumber, ChainConfig, Index, Receipt, Transaction,
};
use ethrex_common::utils::u256_to_big_endian;
use ethrex_rlp::decode::RLPDecode;
use ethrex_rlp::encode::RLPEncode;
use ethrex_trie::{Nibbles, NodeHash, Trie};

use std::{fmt::Debug, sync::Arc};

/// Domain store that implements StoreEngine using the new layered architecture
///
/// This is the single implementation that replaces all the duplicated logic
/// in rocksdb.rs, libmdbx.rs, and in_memory.rs
#[derive(Debug)]
pub struct DomainStore {
    schema: SchemaRegistry,
}

impl DomainStore {
    /// Create a new DomainStore with the given storage backend
    pub fn new(backend: Arc<dyn StorageBackend>) -> Result<Self, StorageError> {
        let schema = SchemaRegistry::new(backend)
            .map_err(|e| StorageError::Custom(format!("Failed to initialize schema: {:?}", e)))?;

        Ok(Self { schema })
    }
}

impl DomainStore {
    /// Store changes in a batch from a vec of blocks
    pub async fn apply_updates(&self, update_batch: UpdateBatch) -> Result<(), StorageError> {
        let mut batch_ops = Vec::new();

        // Process account updates
        for (node_hash, account_node) in update_batch.account_updates {
            let key = node_hash.as_ref().to_vec();
            let value = account_node.encode_to_vec();
            batch_ops.push(TableBatchOp::Put {
                table: DBTable::StateTrieNodes,
                key,
                value,
            });
        }

        // Process storage updates
        for (address_hash, storage_updates) in update_batch.storage_updates {
            for (node_hash, node_data) in storage_updates {
                // Key: address_hash + node_hash
                let mut key = Vec::with_capacity(64);
                key.extend_from_slice(address_hash.as_bytes());
                key.extend_from_slice(node_hash.as_ref());
                batch_ops.push(TableBatchOp::Put {
                    table: DBTable::StorageTrieNodes,
                    key,
                    value: node_data,
                });
            }
        }

        // Process code updates
        for (code_hash, code) in update_batch.code_updates {
            let key = code_hash.as_bytes().to_vec();
            let value = AccountCodeRLP::from(code).bytes().clone();
            batch_ops.push(TableBatchOp::Put {
                table: DBTable::AccountCodes,
                key,
                value,
            });
        }

        // Process block updates
        for block in update_batch.blocks {
            let block_hash = block.hash();

            // Store header
            let header_key = BlockHashRLP::from(block_hash).bytes().clone();
            let header_value = BlockHeaderRLP::from(block.header.clone()).bytes().clone();
            batch_ops.push(TableBatchOp::Put {
                table: DBTable::Headers,
                key: header_key.clone(),
                value: header_value,
            });

            // Store body
            let body_value = BlockBodyRLP::from(block.body.clone()).bytes().clone();
            batch_ops.push(TableBatchOp::Put {
                table: DBTable::Bodies,
                key: header_key.clone(),
                value: body_value,
            });

            // Store block number mapping
            batch_ops.push(TableBatchOp::Put {
                table: DBTable::BlockNumbers,
                key: header_key,
                value: block.header.number.to_le_bytes().to_vec(),
            });
        }

        // Process receipt updates
        for (block_hash, receipts) in update_batch.receipts {
            for (index, receipt) in receipts.into_iter().enumerate() {
                let key = (block_hash, index).encode_to_vec();
                let value = receipt.encode_to_vec();
                batch_ops.push(TableBatchOp::Put {
                    table: DBTable::Receipts,
                    key,
                    value,
                });
            }
        }

        self.schema.batch_write(batch_ops).await
    }

    /// Add a batch of blocks in a single transaction.
    /// This will store -> BlockHeader, BlockBody, BlockTransactions, BlockNumber.
    pub async fn add_blocks(&self, blocks: Vec<Block>) -> Result<(), StorageError> {
        let mut batch_ops = Vec::new();

        for block in blocks {
            let block_hash = block.hash();
            let block_number = block.header.number;

            // Store header
            let header_key = BlockHashRLP::from(block_hash).bytes().clone();
            let header_value = BlockHeaderRLP::from(block.header).bytes().clone();
            batch_ops.push(TableBatchOp::Put {
                table: DBTable::Headers,
                key: header_key.clone(),
                value: header_value,
            });

            // Store body
            let body_value = BlockBodyRLP::from(block.body.clone()).bytes().clone();
            batch_ops.push(TableBatchOp::Put {
                table: DBTable::Bodies,
                key: header_key.clone(),
                value: body_value,
            });

            // Store block number mapping
            batch_ops.push(TableBatchOp::Put {
                table: DBTable::BlockNumbers,
                key: header_key,
                value: block_number.to_le_bytes().to_vec(),
            });

            // Store transaction locations
            for (index, transaction) in block.body.transactions.iter().enumerate() {
                let tx_hash = transaction.hash();
                let mut location_key = Vec::with_capacity(64);
                location_key.extend_from_slice(tx_hash.as_bytes());
                location_key.extend_from_slice(block_hash.as_bytes());
                let location_value = (block_number, block_hash, index as u64).encode_to_vec();

                batch_ops.push(TableBatchOp::Put {
                    table: DBTable::TransactionLocations,
                    key: location_key,
                    value: location_value,
                });
            }
        }

        self.schema.batch_write(batch_ops).await
    }

    /// Add block header
    pub async fn add_block_header(
        &self,
        block_hash: BlockHash,
        block_header: BlockHeader,
    ) -> Result<(), StorageError> {
        let hash_key = BlockHashRLP::from(block_hash).bytes().clone();
        let header_value = BlockHeaderRLP::from(block_header).bytes().clone();
        self.schema
            .put(DBTable::Headers, hash_key, header_value)
            .await
    }

    /// Add a batch of block headers
    pub async fn add_block_headers(
        &self,
        block_headers: Vec<BlockHeader>,
    ) -> Result<(), StorageError> {
        let mut keys_header = Vec::new();
        let mut values_header = Vec::new();
        let mut keys_number = Vec::new();
        let mut values_number = Vec::new();

        for header in block_headers {
            let block_hash = header.hash();
            let hash_key = BlockHashRLP::from(block_hash).bytes().clone();
            let header_value = BlockHeaderRLP::from(header.clone()).bytes().clone();

            keys_header.push(hash_key);
            values_header.push(header_value);

            let number_key = header.number.to_le_bytes().to_vec();
            keys_number.push(BlockHashRLP::from(block_hash).bytes().clone());
            values_number.push(number_key);
        }

        let batch_header = TableBatchOp::Put {
            table: DBTable::Headers,
            key: keys_header.into_iter().flatten().collect(),
            value: values_header.into_iter().flatten().collect(),
        };

        let batch_number = TableBatchOp::Put {
            table: DBTable::BlockNumbers,
            key: keys_number.into_iter().flatten().collect(),
            value: values_number.into_iter().flatten().collect(),
        };

        self.schema
            .batch_write(vec![batch_header, batch_number])
            .await
    }

    /// Obtain canonical block header
    pub fn get_block_header(
        &self,
        block_number: BlockNumber,
    ) -> Result<Option<BlockHeader>, StorageError> {
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
    ) -> Result<(), StorageError> {
        let hash_key = BlockHashRLP::from(block_hash).bytes().clone();
        let body_value = BlockBodyRLP::from(block_body).bytes().clone();
        self.schema.put(DBTable::Bodies, hash_key, body_value).await
    }

    /// Obtain canonical block body
    pub async fn get_block_body(
        &self,
        block_number: BlockNumber,
    ) -> Result<Option<BlockBody>, StorageError> {
        let Some(block_hash) = self.get_canonical_block_hash_sync(block_number)? else {
            return Ok(None);
        };

        self.get_block_body_by_hash(block_hash).await
    }

    /// Remove canonical block
    pub async fn remove_block(&self, block_number: BlockNumber) -> Result<(), StorageError> {
        let Some(hash) = self.get_canonical_block_hash_sync(block_number)? else {
            return Ok(());
        };

        let canonical = TableBatchOp::Delete {
            table: DBTable::Headers,
            key: block_number.to_le_bytes().to_vec(),
        };
        let body = TableBatchOp::Delete {
            table: DBTable::BlockNumbers,
            key: hash.as_bytes().to_vec(),
        };
        let batch_body = TableBatchOp::Delete {
            table: DBTable::Bodies,
            key: hash.as_bytes().to_vec(),
        };
        let batch_number = TableBatchOp::Delete {
            table: DBTable::BlockNumbers,
            key: hash.as_bytes().to_vec(),
        };

        self.schema
            .batch_write(vec![canonical, body, batch_body, batch_number])
            .await
    }

    /// Obtain canonical block bodies in from..=to
    pub async fn get_block_bodies(
        &self,
        from: BlockNumber,
        to: BlockNumber,
    ) -> Result<Vec<BlockBody>, StorageError> {
        let numbers: Vec<BlockNumber> = (from..=to).collect();
        let number_keys: Vec<Vec<u8>> = numbers.iter().map(|n| n.to_le_bytes().to_vec()).collect();
        let hashes = self
            .schema
            .get_async_batch(DBTable::CanonicalHashes, number_keys)
            .await?;

        let bodies = self.schema.get_async_batch(DBTable::Bodies, hashes).await?;

        bodies
            .into_iter()
            .map(|bytes| {
                BlockBodyRLP::from_bytes(bytes)
                    .to()
                    .map_err(StorageError::from)
            })
            .collect::<Result<Vec<BlockBody>, StorageError>>()
    }

    /// Obtain block bodies from a list of hashes
    pub async fn get_block_bodies_by_hash(
        &self,
        hashes: Vec<BlockHash>,
    ) -> Result<Vec<BlockBody>, StorageError> {
        let hash_keys: Vec<Vec<u8>> = hashes
            .iter()
            .map(|hash| BlockHashRLP::from(*hash).bytes().clone())
            .collect();

        let bodies: Vec<BlockBody> = self
            .schema
            .get_async_batch(DBTable::Bodies, hash_keys)
            .await?
            .into_iter()
            .map(|bytes| {
                BlockBodyRLP::from_bytes(bytes)
                    .to()
                    .map_err(StorageError::from)
            })
            .collect::<Result<Vec<BlockBody>, StorageError>>()?;

        Ok(bodies)
    }

    /// Obtain any block body using the hash
    pub async fn get_block_body_by_hash(
        &self,
        block_hash: BlockHash,
    ) -> Result<Option<BlockBody>, StorageError> {
        let hash_key = BlockHashRLP::from(block_hash).bytes().clone();
        self.schema
            .get_async(DBTable::Bodies, hash_key)
            .await?
            .map(|bytes| BlockBodyRLP::from_bytes(bytes).to())
            .transpose()
            .map_err(StorageError::from)
    }

    pub fn get_block_header_by_hash(
        &self,
        block_hash: BlockHash,
    ) -> Result<Option<BlockHeader>, StorageError> {
        let hash_key = BlockHashRLP::from(block_hash).bytes().clone();
        self.schema
            .get_sync(DBTable::Headers, hash_key)?
            .map(|bytes| BlockHeaderRLP::from_bytes(bytes).to())
            .transpose()
            .map_err(StorageError::from)
    }

    pub async fn add_pending_block(&self, block: Block) -> Result<(), StorageError> {
        let hash_key = BlockHashRLP::from(block.hash()).bytes().clone();
        let block_value = BlockRLP::from(block).bytes().clone();
        self.schema
            .put(DBTable::PendingBlocks, hash_key, block_value)
            .await
    }
    pub async fn get_pending_block(
        &self,
        block_hash: BlockHash,
    ) -> Result<Option<Block>, StorageError> {
        let hash_key = BlockHashRLP::from(block_hash).bytes().clone();
        self.schema
            .get_async(DBTable::PendingBlocks, hash_key)
            .await?
            .map(|bytes| BlockRLP::from_bytes(bytes).to())
            .transpose()
            .map_err(StorageError::from)
    }

    /// Add block number for a given hash
    pub async fn add_block_number(
        &self,
        block_hash: BlockHash,
        block_number: BlockNumber,
    ) -> Result<(), StorageError> {
        let hash_key = BlockHashRLP::from(block_hash).bytes().clone();
        let number_value = block_number.to_le_bytes().to_vec();
        self.schema
            .put(DBTable::BlockNumbers, hash_key, number_value)
            .await
    }

    /// Obtain block number for a given hash
    pub async fn get_block_number(
        &self,
        block_hash: BlockHash,
    ) -> Result<Option<BlockNumber>, StorageError> {
        let hash_key = BlockHashRLP::from(block_hash).bytes().clone();
        self.schema
            .get_async(DBTable::BlockNumbers, hash_key)
            .await?
            .map(|bytes| -> Result<BlockNumber, StorageError> {
                let array: [u8; 8] = bytes
                    .try_into()
                    .map_err(|_| StorageError::Custom("Invalid BlockNumber bytes".to_string()))?;
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
    ) -> Result<(), StorageError> {
        let mut composite_key = Vec::with_capacity(64);
        composite_key.extend_from_slice(transaction_hash.as_bytes());
        composite_key.extend_from_slice(block_hash.as_bytes());
        let location_value = (block_number, block_hash, index).encode_to_vec();
        self.schema
            .put(DBTable::TransactionLocations, composite_key, location_value)
            .await
    }

    /// Store transaction locations in batch (one db transaction for all)
    pub async fn add_transaction_locations(
        &self,
        locations: Vec<(H256, BlockNumber, BlockHash, Index)>,
    ) -> Result<(), StorageError> {
        let mut keys = Vec::new();
        let mut values = Vec::new();
        for (tx_hash, block_number, block_hash, index) in locations {
            // Key: tx_hash + block_hash
            let mut composite_key = Vec::with_capacity(64);
            composite_key.extend_from_slice(tx_hash.as_bytes());
            composite_key.extend_from_slice(block_hash.as_bytes());

            let location_value = (block_number, block_hash, index).encode_to_vec();
            keys.push(composite_key);
            values.push(location_value);
        }
        self.schema
            .batch_write(vec![TableBatchOp::Put {
                table: DBTable::TransactionLocations,
                key: keys.into_iter().flatten().collect(),
                value: values.into_iter().flatten().collect(),
            }])
            .await
    }

    /// Obtain transaction location (block hash and index)
    pub async fn get_transaction_location(
        &self,
        transaction_hash: H256,
    ) -> Result<Option<(BlockNumber, BlockHash, Index)>, StorageError> {
        // Use range query to find transaction location
        // Key pattern: tx_hash + block_hash
        let start_key = transaction_hash.as_bytes().to_vec();
        let mut end_key = start_key.clone();
        end_key.push(0xFF); // Extend to get all entries with this tx_hash prefix

        let results = self
            .schema
            .range(DBTable::TransactionLocations, start_key, Some(end_key))
            .await?;

        if let Some((_, value_bytes)) = results.first() {
            let location: (BlockNumber, BlockHash, Index) = RLPDecode::decode(value_bytes)?;
            Ok(Some(location))
        } else {
            Ok(None)
        }
    }

    /// Add receipt
    pub async fn add_receipt(
        &self,
        block_hash: BlockHash,
        index: Index,
        receipt: Receipt,
    ) -> Result<(), StorageError> {
        let key = (block_hash, index).encode_to_vec();
        let value = receipt.encode_to_vec();
        self.schema.put(DBTable::Receipts, key, value).await
    }

    /// Add receipts
    pub async fn add_receipts(
        &self,
        block_hash: BlockHash,
        receipts: Vec<Receipt>,
    ) -> Result<(), StorageError> {
        let mut keys = Vec::new();
        let mut values = Vec::new();
        for (index, receipt) in receipts.into_iter().enumerate() {
            let key = (block_hash, index as u64).encode_to_vec();
            let value = receipt.encode_to_vec();
            keys.push(key);
            values.push(value);
        }
        self.schema
            .batch_write(vec![TableBatchOp::Put {
                table: DBTable::Receipts,
                key: keys.into_iter().flatten().collect(),
                value: values.into_iter().flatten().collect(),
            }])
            .await
    }

    /// Obtain receipt by block hash and index
    pub async fn get_receipt(
        &self,
        block_hash: BlockHash,
        index: Index,
    ) -> Result<Option<Receipt>, StorageError> {
        let key = (block_hash, index as u64).encode_to_vec();
        self.schema
            .get_async(DBTable::Receipts, key)
            .await?
            .map(|bytes| Receipt::decode(bytes.as_slice()))
            .transpose()
            .map_err(StorageError::from)
    }

    /// Add account code
    pub async fn add_account_code(&self, code_hash: H256, code: Bytes) -> Result<(), StorageError> {
        let hash_key = code_hash.as_bytes().to_vec();
        let code_value = AccountCodeRLP::from(code).bytes().clone();
        self.schema
            .put(DBTable::AccountCodes, hash_key, code_value)
            .await
    }

    /// Clears all checkpoint data created during the last snap sync
    pub async fn clear_snap_state(&self) -> Result<(), StorageError> {
        // Clear all snap state data by removing all entries in the SnapState table
        // Since we don't have a clear_table method, we'll use range to get all keys and delete them
        let all_data = self
            .schema
            .range(DBTable::SnapState, [0].to_vec(), None)
            .await?;

        if all_data.is_empty() {
            return Ok(());
        }

        let mut batch_ops = Vec::new();
        for (key, _) in all_data {
            batch_ops.push(TableBatchOp::Delete {
                table: DBTable::SnapState,
                key,
            });
        }

        self.schema.batch_write(batch_ops).await
    }

    /// Obtain account code via code hash
    pub fn get_account_code(&self, code_hash: H256) -> Result<Option<Bytes>, StorageError> {
        let hash_key = code_hash.as_bytes().to_vec();
        self.schema
            .get_sync(DBTable::AccountCodes, hash_key)?
            .map(|bytes| AccountCodeRLP::from_bytes(bytes).to())
            .transpose()
            .map_err(StorageError::from)
    }

    pub async fn get_transaction_by_hash(
        &self,
        transaction_hash: H256,
    ) -> Result<Option<Transaction>, StorageError> {
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
    ) -> Result<Option<Transaction>, StorageError> {
        let block_body = match self.get_block_body_by_hash(block_hash).await? {
            Some(body) => body,
            None => return Ok(None),
        };
        // TODO: Fix error handling
        let index: usize = index
            .try_into()
            .map_err(|_| StorageError::Custom("Invalid index".to_string()))?;
        Ok(block_body.transactions.get(index).cloned())
    }

    pub async fn get_block_by_hash(
        &self,
        block_hash: BlockHash,
    ) -> Result<Option<Block>, StorageError> {
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
    ) -> Result<Option<Block>, StorageError> {
        let Some(block_hash) = self.get_canonical_block_hash(block_number).await? else {
            return Ok(None);
        };
        self.get_block_by_hash(block_hash).await
    }

    // Get the canonical block hash for a given block number.
    pub async fn get_canonical_block_hash(
        &self,
        block_number: BlockNumber,
    ) -> Result<Option<BlockHash>, StorageError> {
        let number_key = block_number.to_le_bytes().to_vec();
        self.schema
            .get_async(DBTable::CanonicalHashes, number_key)
            .await?
            .map(|bytes| BlockHashRLP::from_bytes(bytes).to())
            .transpose()
            .map_err(StorageError::from)
    }

    /// Stores the chain configuration values, should only be called once after reading the genesis file
    /// Ignores previously stored values if present
    pub async fn set_chain_config(&self, chain_config: &ChainConfig) -> Result<(), StorageError> {
        let key = Self::chain_data_key(ChainDataIndex::ChainConfig);
        let value = serde_json::to_string(chain_config)
            .map_err(|_| StorageError::Custom("Failed to serialize chain config".to_string()))?
            .into_bytes();
        self.schema.put(DBTable::ChainData, key, value).await
    }

    /// Update earliest block number
    pub async fn update_earliest_block_number(
        &self,
        block_number: BlockNumber,
    ) -> Result<(), StorageError> {
        let key = vec![ChainDataIndex::EarliestBlockNumber as u8];
        let value = block_number.to_le_bytes().to_vec();
        self.schema.put(DBTable::ChainData, key, value).await
    }

    /// Obtain earliest block number
    pub async fn get_earliest_block_number(&self) -> Result<Option<BlockNumber>, StorageError> {
        let key = vec![ChainDataIndex::EarliestBlockNumber as u8];
        self.schema
            .get_async(DBTable::ChainData, key)
            .await?
            .map(|bytes| -> Result<BlockNumber, StorageError> {
                let array: [u8; 8] = bytes
                    .try_into()
                    .map_err(|_| StorageError::Custom("Invalid BlockNumber bytes".to_string()))?;
                Ok(BlockNumber::from_le_bytes(array))
            })
            .transpose()
    }

    /// Obtain finalized block number
    pub async fn get_finalized_block_number(&self) -> Result<Option<BlockNumber>, StorageError> {
        let key = vec![ChainDataIndex::FinalizedBlockNumber as u8];
        self.schema
            .get_async(DBTable::ChainData, key)
            .await?
            .map(|bytes| -> Result<BlockNumber, StorageError> {
                let array: [u8; 8] = bytes
                    .try_into()
                    .map_err(|_| StorageError::Custom("Invalid BlockNumber bytes".to_string()))?;
                Ok(BlockNumber::from_le_bytes(array))
            })
            .transpose()
    }

    /// Obtain safe block number
    pub async fn get_safe_block_number(&self) -> Result<Option<BlockNumber>, StorageError> {
        let key = vec![ChainDataIndex::SafeBlockNumber as u8];
        self.schema
            .get_async(DBTable::ChainData, key)
            .await?
            .map(|bytes| -> Result<BlockNumber, StorageError> {
                let array: [u8; 8] = bytes
                    .try_into()
                    .map_err(|_| StorageError::Custom("Invalid BlockNumber bytes".to_string()))?;
                Ok(BlockNumber::from_le_bytes(array))
            })
            .transpose()
    }

    /// Obtain latest block number
    pub async fn get_latest_block_number(&self) -> Result<Option<BlockNumber>, StorageError> {
        let key = vec![ChainDataIndex::LatestBlockNumber as u8];
        self.schema
            .get_async(DBTable::ChainData, key)
            .await?
            .map(|bytes| -> Result<BlockNumber, StorageError> {
                let array: [u8; 8] = bytes
                    .try_into()
                    .map_err(|_| StorageError::Custom("Invalid BlockNumber bytes".to_string()))?;
                Ok(BlockNumber::from_le_bytes(array))
            })
            .transpose()
    }

    /// Update pending block number
    pub async fn update_pending_block_number(
        &self,
        block_number: BlockNumber,
    ) -> Result<(), StorageError> {
        let key = vec![ChainDataIndex::PendingBlockNumber as u8];
        let value = block_number.to_le_bytes().to_vec();
        self.schema.put(DBTable::ChainData, key, value).await
    }

    /// Obtain pending block number
    pub async fn get_pending_block_number(&self) -> Result<Option<BlockNumber>, StorageError> {
        let key = vec![ChainDataIndex::PendingBlockNumber as u8];
        self.schema
            .get_async(DBTable::ChainData, key)
            .await?
            .map(|bytes| -> Result<BlockNumber, StorageError> {
                let array: [u8; 8] = bytes
                    .try_into()
                    .map_err(|_| StorageError::Custom("Invalid BlockNumber bytes".to_string()))?;
                Ok(BlockNumber::from_le_bytes(array))
            })
            .transpose()
    }

    pub async fn forkchoice_update(
        &self,
        new_canonical_blocks: Option<Vec<(BlockNumber, BlockHash)>>,
        head_number: BlockNumber,
        head_hash: BlockHash,
        safe: Option<BlockNumber>,
        finalized: Option<BlockNumber>,
    ) -> Result<(), StorageError> {
        let latest = self.get_latest_block_number().await?.unwrap_or(0);
        let mut batch_ops = Vec::new();

        // Update canonical block hashes
        if let Some(canonical_blocks) = new_canonical_blocks {
            for (block_number, block_hash) in canonical_blocks {
                let number_key = block_number.to_le_bytes().to_vec();
                let hash_value = BlockHashRLP::from(block_hash).bytes().clone();
                batch_ops.push(TableBatchOp::Put {
                    table: DBTable::CanonicalHashes,
                    key: number_key,
                    value: hash_value,
                });
            }
        }

        // Remove anything after the head from the canonical chain
        for number in (head_number + 1)..=(latest) {
            let number_key = number.to_le_bytes().to_vec();
            batch_ops.push(TableBatchOp::Delete {
                table: DBTable::CanonicalHashes,
                key: number_key,
            });
        }

        // Make head canonical
        let head_key = head_number.to_le_bytes().to_vec();
        let head_value = BlockHashRLP::from(head_hash).bytes().clone();
        batch_ops.push(TableBatchOp::Put {
            table: DBTable::CanonicalHashes,
            key: head_key,
            value: head_value,
        });

        // Update chain data
        let latest_key = Self::chain_data_key(ChainDataIndex::LatestBlockNumber);
        batch_ops.push(TableBatchOp::Put {
            table: DBTable::ChainData,
            key: latest_key,
            value: head_number.to_le_bytes().to_vec(),
        });

        if let Some(safe_number) = safe {
            let safe_key = Self::chain_data_key(ChainDataIndex::SafeBlockNumber);
            batch_ops.push(TableBatchOp::Put {
                table: DBTable::ChainData,
                key: safe_key,
                value: safe_number.to_le_bytes().to_vec(),
            });
        }

        if let Some(finalized_number) = finalized {
            let finalized_key = Self::chain_data_key(ChainDataIndex::FinalizedBlockNumber);
            batch_ops.push(TableBatchOp::Put {
                table: DBTable::ChainData,
                key: finalized_key,
                value: finalized_number.to_le_bytes().to_vec(),
            });
        }

        self.schema.batch_write(batch_ops).await
    }

    pub fn get_receipts_for_block(
        &self,
        block_hash: &BlockHash,
    ) -> Result<Vec<Receipt>, StorageError> {
        let mut receipts = Vec::new();
        let mut index = 0u64;
        loop {
            let key = (*block_hash, index).encode_to_vec();
            match self.schema.get_sync(DBTable::Receipts, key)? {
                Some(bytes) => {
                    let receipt = Receipt::decode(bytes.as_slice())?;
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
    ) -> Result<(), StorageError> {
        let key = vec![SnapStateIndex::HeaderDownloadCheckpoint as u8];
        let value = BlockHashRLP::from(block_hash).bytes().clone();
        self.schema.put(DBTable::SnapState, key, value).await
    }

    /// Gets the hash of the last header downloaded during a snap sync
    pub async fn get_header_download_checkpoint(&self) -> Result<Option<BlockHash>, StorageError> {
        let key = vec![SnapStateIndex::HeaderDownloadCheckpoint as u8];
        self.schema
            .get_async(DBTable::SnapState, key)
            .await?
            .map(|bytes| BlockHashRLP::from_bytes(bytes).to())
            .transpose()
            .map_err(StorageError::from)
    }

    /// Sets the last key fetched from the state trie being fetched during snap sync
    pub async fn set_state_trie_key_checkpoint(
        &self,
        last_keys: [H256; STATE_TRIE_SEGMENTS],
    ) -> Result<(), StorageError> {
        let key = vec![SnapStateIndex::StateTrieKeyCheckpoint as u8];
        let value = last_keys.to_vec().encode_to_vec();
        self.schema.put(DBTable::SnapState, key, value).await
    }

    /// Gets the last key fetched from the state trie being fetched during snap sync
    pub async fn get_state_trie_key_checkpoint(
        &self,
    ) -> Result<Option<[H256; STATE_TRIE_SEGMENTS]>, StorageError> {
        let key = vec![SnapStateIndex::StateTrieKeyCheckpoint as u8];
        self.schema
            .get_async(DBTable::SnapState, key)
            .await?
            .map(
                |bytes| -> Result<[H256; STATE_TRIE_SEGMENTS], StorageError> {
                    let keys_vec: Vec<H256> = Vec::<H256>::decode(bytes.as_slice())?;
                    keys_vec
                        .try_into()
                        .map_err(|_| StorageError::Custom("Invalid array size".to_string()))
                },
            )
            .transpose()
    }

    /// Sets the state trie paths in need of healing
    pub async fn set_state_heal_paths(
        &self,
        paths: Vec<(Nibbles, H256)>,
    ) -> Result<(), StorageError> {
        let key = vec![SnapStateIndex::StateHealPaths as u8];
        let value = paths.encode_to_vec();
        self.schema.put(DBTable::SnapState, key, value).await
    }

    /// Gets the state trie paths in need of healing
    pub async fn get_state_heal_paths(&self) -> Result<Option<Vec<(Nibbles, H256)>>, StorageError> {
        let key = vec![SnapStateIndex::StateHealPaths as u8];
        self.schema
            .get_async(DBTable::SnapState, key)
            .await?
            .map(|bytes| Vec::<(Nibbles, H256)>::decode(bytes.as_slice()))
            .transpose()
            .map_err(StorageError::from)
    }

    /// Write a storage batch into the current storage snapshot
    pub async fn write_snapshot_storage_batch(
        &self,
        account_hash: H256,
        storage_keys: Vec<H256>,
        storage_values: Vec<U256>,
    ) -> Result<(), StorageError> {
        if storage_keys.len() != storage_values.len() {
            return Err(StorageError::Custom(
                "Storage keys and values length mismatch".to_string(),
            ));
        }

        let mut keys = Vec::new();
        let mut values = Vec::new();
        for (key, value) in storage_keys.into_iter().zip(storage_values.into_iter()) {
            let mut composite_key = Vec::with_capacity(64);
            composite_key.extend_from_slice(account_hash.as_bytes());
            composite_key.extend_from_slice(key.as_bytes());
            let value_bytes = u256_to_big_endian(value).to_vec();

            keys.push(composite_key);
            values.push(value_bytes);
        }

        self.schema
            .batch_write(vec![TableBatchOp::Put {
                table: DBTable::StorageSnapshot,
                key: keys.into_iter().flatten().collect(),
                value: values.into_iter().flatten().collect(),
            }])
            .await
    }

    /// Write multiple storage batches belonging to different accounts into the current storage snapshot
    pub async fn write_snapshot_storage_batches(
        &self,
        account_hashes: Vec<H256>,
        storage_keys: Vec<Vec<H256>>,
        storage_values: Vec<Vec<U256>>,
    ) -> Result<(), StorageError> {
        if account_hashes.len() != storage_keys.len()
            || account_hashes.len() != storage_values.len()
        {
            return Err(StorageError::Custom(
                "Account hashes, keys, and values length mismatch".to_string(),
            ));
        }

        let mut batch_keys = Vec::new();
        let mut batch_values = Vec::new();
        for ((account_hash, keys), values) in account_hashes
            .into_iter()
            .zip(storage_keys.into_iter())
            .zip(storage_values.into_iter())
        {
            if keys.len() != values.len() {
                return Err(StorageError::Custom(
                    "Storage keys and values length mismatch for account".to_string(),
                ));
            }

            for (key, value) in keys.into_iter().zip(values.into_iter()) {
                // Create composite key: account_hash + storage_key
                let mut composite_key = Vec::with_capacity(64);
                composite_key.extend_from_slice(account_hash.as_bytes());
                composite_key.extend_from_slice(key.as_bytes());
                let value_bytes = u256_to_big_endian(value).to_vec();

                batch_keys.push(composite_key);
                batch_values.push(value_bytes);
            }
        }

        self.schema
            .batch_write(vec![TableBatchOp::Put {
                table: DBTable::StorageSnapshot,
                key: batch_keys.into_iter().flatten().collect(),
                value: batch_values.into_iter().flatten().collect(),
            }])
            .await
    }

    /// Set the latest root of the rebuilt state trie and the last downloaded hashes from each segment
    pub async fn set_state_trie_rebuild_checkpoint(
        &self,
        checkpoint: (H256, [H256; STATE_TRIE_SEGMENTS]),
    ) -> Result<(), StorageError> {
        let key = vec![SnapStateIndex::StateTrieRebuildCheckpoint as u8];
        let value = (checkpoint.0, checkpoint.1.to_vec()).encode_to_vec();
        self.schema.put(DBTable::SnapState, key, value).await
    }

    /// Get the latest root of the rebuilt state trie and the last downloaded hashes from each segment
    pub async fn get_state_trie_rebuild_checkpoint(
        &self,
    ) -> Result<Option<(H256, [H256; STATE_TRIE_SEGMENTS])>, StorageError> {
        let key = vec![SnapStateIndex::StateTrieRebuildCheckpoint as u8];
        self.schema
            .get_async(DBTable::SnapState, key)
            .await?
            .map(
                |bytes| -> Result<(H256, [H256; STATE_TRIE_SEGMENTS]), StorageError> {
                    let (root, segments_vec): (H256, Vec<H256>) = RLPDecode::decode(&bytes)?;
                    let segments: [H256; STATE_TRIE_SEGMENTS] =
                        segments_vec.try_into().map_err(|_| {
                            StorageError::Custom("Invalid segments array size".to_string())
                        })?;
                    Ok((root, segments))
                },
            )
            .transpose()
    }

    /// Get the accont hashes and roots of the storage tries awaiting rebuild
    pub async fn set_storage_trie_rebuild_pending(
        &self,
        pending: Vec<(H256, H256)>,
    ) -> Result<(), StorageError> {
        let key = vec![SnapStateIndex::StorageTrieRebuildPending as u8];
        let value = pending.encode_to_vec();
        self.schema.put(DBTable::SnapState, key, value).await
    }

    /// Get the accont hashes and roots of the storage tries awaiting rebuild
    pub async fn get_storage_trie_rebuild_pending(
        &self,
    ) -> Result<Option<Vec<(H256, H256)>>, StorageError> {
        let key = vec![SnapStateIndex::StorageTrieRebuildPending as u8];
        self.schema
            .get_async(DBTable::SnapState, key)
            .await?
            .map(|bytes| Vec::<(H256, H256)>::decode(bytes.as_slice()))
            .transpose()
            .map_err(StorageError::from)
    }

    /// Reads the next `MAX_SNAPSHOT_READS` elements from the storage snapshot as from the `start` storage key
    pub async fn read_storage_snapshot(
        &self,
        start: H256,
        account_hash: H256,
    ) -> Result<Vec<(H256, U256)>, StorageError> {
        // Create start key: account_hash + start
        let mut start_key = Vec::with_capacity(64);
        start_key.extend_from_slice(account_hash.as_bytes());
        start_key.extend_from_slice(start.as_bytes());

        // Create end key: account_hash + max
        let mut end_key = Vec::with_capacity(64);
        end_key.extend_from_slice(account_hash.as_bytes());
        end_key.extend_from_slice(&[0xFF; 32]); // Max possible H256

        let results = self
            .schema
            .range(DBTable::StorageSnapshot, start_key, Some(end_key))
            .await?;

        let mut storage_entries = Vec::new();
        for (key, value) in results {
            if key.len() >= 64 {
                // Extract storage key (last 32 bytes)
                let storage_key = H256::from_slice(&key[32..64]);
                // Decode U256 value
                let storage_value = U256::from_big_endian(&value);
                storage_entries.push((storage_key, storage_value));
            }
        }

        Ok(storage_entries)
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
    ) -> Result<(), StorageError> {
        let key = BlockHashRLP::from(bad_block).bytes().clone();
        let value = BlockHashRLP::from(latest_valid).bytes().clone();
        self.schema.put(DBTable::InvalidAncestors, key, value).await
    }

    /// Returns the latest valid ancestor hash for a given invalid block hash.
    /// Used to provide `latest_valid_hash` in the Engine API when processing invalid payloads.
    pub async fn get_latest_valid_ancestor(
        &self,
        block: BlockHash,
    ) -> Result<Option<BlockHash>, StorageError> {
        let key = BlockHashRLP::from(block).bytes().clone();
        self.schema
            .get_async(DBTable::InvalidAncestors, key)
            .await?
            .map(|bytes| BlockHashRLP::from_bytes(bytes).to())
            .transpose()
            .map_err(StorageError::from)
    }

    /// Obtain block number for a given hash
    pub fn get_block_number_sync(
        &self,
        block_hash: BlockHash,
    ) -> Result<Option<BlockNumber>, StorageError> {
        let hash_key = BlockHashRLP::from(block_hash).bytes().clone();

        self.schema
            .get_sync(DBTable::BlockNumbers, hash_key)?
            .map(|bytes| -> Result<BlockNumber, StorageError> {
                let array: [u8; 8] = bytes
                    .try_into()
                    .map_err(|_| StorageError::Custom("Invalid BlockNumber bytes".to_string()))?;
                Ok(BlockNumber::from_le_bytes(array))
            })
            .transpose()
    }

    /// Get the canonical block hash for a given block number.
    pub fn get_canonical_block_hash_sync(
        &self,
        block_number: BlockNumber,
    ) -> Result<Option<BlockHash>, StorageError> {
        let number_key = block_number.to_le_bytes().to_vec();
        match self.schema.get_sync(DBTable::CanonicalHashes, number_key)? {
            Some(bytes) => {
                let rlp = BlockHashRLP::from_bytes(bytes)
                    .to()
                    .map_err(StorageError::from)?;
                Ok(Some(rlp))
            }
            None => Ok(None),
        }
    }

    pub async fn write_storage_trie_nodes_batch(
        &self,
        storage_trie_nodes: Vec<(H256, Vec<(NodeHash, Vec<u8>)>)>,
    ) -> Result<(), StorageError> {
        let mut keys = Vec::new();
        let mut values = Vec::new();
        for (address_hash, nodes) in storage_trie_nodes {
            for (node_hash, node_data) in nodes {
                // Create composite key: address_hash + node_hash
                let mut key = Vec::with_capacity(64);
                key.extend_from_slice(address_hash.as_bytes());
                key.extend_from_slice(node_hash.as_ref());
                keys.push(key);
                values.push(node_data);
            }
        }

        self.schema
            .batch_write(vec![TableBatchOp::Put {
                table: DBTable::StorageTrieNodes,
                key: keys.into_iter().flatten().collect(),
                value: values.into_iter().flatten().collect(),
            }])
            .await
    }

    pub async fn write_account_code_batch(
        &self,
        account_codes: Vec<(H256, Bytes)>,
    ) -> Result<(), StorageError> {
        let mut keys = Vec::new();
        let mut values = Vec::new();
        for (code_hash, code) in account_codes {
            let key = code_hash.as_bytes().to_vec();
            let value = AccountCodeRLP::from(code).bytes().clone();
            keys.push(key);
            values.push(value);
        }

        self.schema
            .batch_write(vec![TableBatchOp::Put {
                table: DBTable::AccountCodes,
                key: keys.into_iter().flatten().collect(),
                value: values.into_iter().flatten().collect(),
            }])
            .await
    }

    /// Obtain a state trie from the given state root
    /// Doesn't check if the state root is valid
    /// Used for internal store operations
    pub fn open_state_trie(&self, state_root: H256) -> Result<Trie, StorageError> {
        let trie_db = Box::new(StorageBackendTrieDB::new_state_trie(
            self.schema.backend.clone(),
        )?);
        Ok(Trie::open(trie_db, state_root))
    }

    /// Obtain a storage trie from the given address and storage_root
    /// Doesn't check if the account is stored
    /// Used for internal store operations
    pub fn open_storage_trie(
        &self,
        hashed_address: H256,
        storage_root: H256,
    ) -> Result<Trie, StorageError> {
        let trie_db = Box::new(StorageBackendTrieDB::new_storage_trie(
            self.schema.backend.clone(),
            hashed_address,
        )?);
        Ok(Trie::open(trie_db, storage_root))
    }

    /// Obtain a state trie locked for reads from the given state root
    /// Doesn't check if the state root is valid
    /// Used for internal store operations
    pub fn open_locked_state_trie(&self, state_root: H256) -> Result<Trie, StorageError> {
        let trie_db = Box::new(StorageBackendLockedTrieDB::new_state_trie(
            self.schema.backend.clone(),
        )?);
        Ok(Trie::open(trie_db, state_root))
    }

    /// Obtain a read-locked storage trie from the given address and storage_root
    /// Doesn't check if the account is stored
    /// Used for internal store operations
    pub fn open_locked_storage_trie(
        &self,
        hashed_address: H256,
        storage_root: H256,
    ) -> Result<Trie, StorageError> {
        let trie_db = Box::new(StorageBackendLockedTrieDB::new_storage_trie(
            self.schema.backend.clone(),
            hashed_address,
        )?);
        Ok(Trie::open(trie_db, storage_root))
    }
}

impl DomainStore {
    // Helper methods for key generation
    pub fn chain_data_key(index: ChainDataIndex) -> Vec<u8> {
        vec![index as u8]
    }
}
