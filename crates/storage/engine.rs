use crate::backend::{BatchOp, StorageBackend, StorageBackendLockedTrieDB, StorageBackendTrieDB};
use crate::error::StoreError;
use crate::rlp::{AccountCodeRLP, BlockBodyRLP, BlockHashRLP, BlockHeaderRLP, BlockRLP};
use crate::utils::SnapStateIndex;
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

/// Struct for storage trie nodes - [address_hash, [(node_hash, node_data)]]
type StorageTrieNodes = Vec<(H256, Vec<(NodeHash, Vec<u8>)>)>;

/// Defines all the logical tables needed for Ethereum storage
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DBTable {
    // Block data
    Headers,
    Bodies,
    BlockNumbers,
    CanonicalHashes,

    // Transaction data
    TransactionLocations,
    Receipts,

    // Account data
    AccountCodes,

    // Trie data
    StateTrieNodes,
    StorageTrieNodes,

    // Chain metadata
    ChainData,

    // Snap sync data
    SnapState,
    StorageSnapshot,

    // Pending data
    PendingBlocks,

    // Error tracking
    InvalidAncestors,
}

impl DBTable {
    /// Returns the namespace string for this table
    pub fn namespace(&self) -> &'static str {
        match self {
            Self::Headers => "headers",
            Self::Bodies => "bodies",
            Self::BlockNumbers => "block_numbers",
            Self::CanonicalHashes => "canonical_hashes",
            Self::TransactionLocations => "transaction_locations",
            Self::Receipts => "receipts",
            Self::AccountCodes => "account_codes",
            Self::StateTrieNodes => "state_trie_nodes",
            Self::StorageTrieNodes => "storage_trie_nodes",
            Self::ChainData => "chain_data",
            Self::SnapState => "snap_state",
            Self::StorageSnapshot => "storage_snapshot",
            Self::PendingBlocks => "pending_blocks",
            Self::InvalidAncestors => "invalid_ancestors",
        }
    }

    /// Returns all table variants
    pub fn all() -> &'static [DBTable] {
        &[
            Self::Headers,
            Self::Bodies,
            Self::BlockNumbers,
            Self::CanonicalHashes,
            Self::TransactionLocations,
            Self::Receipts,
            Self::AccountCodes,
            Self::StateTrieNodes,
            Self::StorageTrieNodes,
            Self::ChainData,
            Self::SnapState,
            Self::StorageSnapshot,
            Self::PendingBlocks,
            Self::InvalidAncestors,
        ]
    }
}

/// Batch operation at the domain level (before translation to backend operations)
#[derive(Debug, Clone)]
pub enum BatchOperation {
    Put {
        table: DBTable,
        key: Vec<u8>,
        value: Vec<u8>,
    },
    Delete {
        table: DBTable,
        key: Vec<u8>,
    },
}

/// Engine that implements the storage backend interface
/// TODO: Could we move this logic to the store?
#[derive(Debug)]
pub struct Engine {
    backend: Arc<dyn StorageBackend>,
}

impl Engine {
    /// Create a new Engine with the given storage backend
    pub fn new(backend: Arc<dyn StorageBackend>) -> Self {
        Self { backend }
    }

    /// Execute a batch of operations
    async fn execute_batch(&self, batch_ops: Vec<BatchOperation>) -> Result<(), StoreError> {
        let backend_ops: Vec<BatchOp> = batch_ops
            .into_iter()
            .map(|op| match op {
                BatchOperation::Put { table, key, value } => BatchOp::Put {
                    namespace: table.namespace().to_string(),
                    key,
                    value,
                },
                BatchOperation::Delete { table, key } => BatchOp::Delete {
                    namespace: table.namespace().to_string(),
                    key,
                },
            })
            .collect();

        self.backend.batch_write(backend_ops).await
    }
}

impl Engine {
    /// Store changes in a batch from a vec of blocks
    pub async fn apply_updates(&self, update_batch: UpdateBatch) -> Result<(), StoreError> {
        let mut batch_ops = Vec::new();

        // Process account updates
        for (node_hash, account_node) in update_batch.account_updates {
            let key = node_hash.as_ref().to_vec();
            batch_ops.push(BatchOperation::Put {
                table: DBTable::StateTrieNodes,
                key,
                value: account_node,
            });
        }

        // Process storage updates
        for (address_hash, storage_updates) in update_batch.storage_updates {
            for (node_hash, node_data) in storage_updates {
                let mut key = Vec::with_capacity(64);
                key.extend_from_slice(address_hash.as_bytes());
                key.extend_from_slice(node_hash.as_ref());
                batch_ops.push(BatchOperation::Put {
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
            batch_ops.push(BatchOperation::Put {
                table: DBTable::AccountCodes,
                key,
                value,
            });
        }

        // Process receipt updates
        for (block_hash, receipts) in update_batch.receipts {
            for (index, receipt) in receipts.into_iter().enumerate() {
                let key = (block_hash, index).encode_to_vec();
                let value = receipt.encode_to_vec();
                batch_ops.push(BatchOperation::Put {
                    table: DBTable::Receipts,
                    key,
                    value,
                });
            }
        }

        // Process block updates
        for block in update_batch.blocks {
            let block_number = block.header.number;
            let block_hash = block.hash();

            // Store header
            let header_key = BlockHashRLP::from(block_hash).bytes().clone();
            let header_value = BlockHeaderRLP::from(block.header.clone()).bytes().clone();
            batch_ops.push(BatchOperation::Put {
                table: DBTable::Headers,
                key: header_key.clone(),
                value: header_value,
            });

            // Store body
            let body_value = BlockBodyRLP::from(block.body.clone()).bytes().clone();
            batch_ops.push(BatchOperation::Put {
                table: DBTable::Bodies,
                key: header_key.clone(),
                value: body_value,
            });

            // Store block number mapping
            let hash_key = BlockHashRLP::from(block_hash).bytes().clone();
            batch_ops.push(BatchOperation::Put {
                table: DBTable::BlockNumbers,
                key: hash_key,
                value: block_number.to_le_bytes().to_vec(),
            });

            for (index, transaction) in block.body.transactions.iter().enumerate() {
                let tx_hash = transaction.hash();
                // Key: tx_hash + block_hash
                let mut composite_key = Vec::with_capacity(64);
                composite_key.extend_from_slice(tx_hash.as_bytes());
                composite_key.extend_from_slice(block_hash.as_bytes());
                let location_value = (block_number, block_hash, index as u64).encode_to_vec();
                batch_ops.push(BatchOperation::Put {
                    table: DBTable::TransactionLocations,
                    key: composite_key,
                    value: location_value,
                });
            }
        }

        self.execute_batch(batch_ops).await
    }

    /// Add a batch of blocks in a single transaction.
    /// This will store -> BlockHeader, BlockBody, BlockTransactions, BlockNumber.
    pub async fn add_blocks(&self, blocks: Vec<Block>) -> Result<(), StoreError> {
        let mut batch_ops = Vec::new();

        for block in blocks {
            let block_hash = block.hash();
            let block_number = block.header.number;

            // Store header
            let header_key = BlockHashRLP::from(block_hash).bytes().clone();
            let header_value = BlockHeaderRLP::from(block.header).bytes().clone();
            batch_ops.push(BatchOperation::Put {
                table: DBTable::Headers,
                key: header_key.clone(),
                value: header_value,
            });

            // Store body
            let body_value = BlockBodyRLP::from(block.body.clone()).bytes().clone();
            batch_ops.push(BatchOperation::Put {
                table: DBTable::Bodies,
                key: header_key.clone(),
                value: body_value,
            });

            // Store block number mapping
            let hash_key = BlockHashRLP::from(block_hash).bytes().clone();
            batch_ops.push(BatchOperation::Put {
                table: DBTable::BlockNumbers,
                key: hash_key,
                value: block_number.to_le_bytes().to_vec(),
            });

            // Store transaction locations
            for (index, transaction) in block.body.transactions.iter().enumerate() {
                let tx_hash = transaction.hash();
                let mut location_key = Vec::with_capacity(64);
                location_key.extend_from_slice(tx_hash.as_bytes());
                location_key.extend_from_slice(block_hash.as_bytes());
                let location_value = (block_number, block_hash, index as u64).encode_to_vec();

                batch_ops.push(BatchOperation::Put {
                    table: DBTable::TransactionLocations,
                    key: location_key,
                    value: location_value,
                });
            }
        }

        self.execute_batch(batch_ops).await
    }

    /// Add block header
    pub async fn add_block_header(
        &self,
        block_hash: BlockHash,
        block_header: BlockHeader,
    ) -> Result<(), StoreError> {
        let hash_key = BlockHashRLP::from(block_hash).bytes().clone();
        let header_value = BlockHeaderRLP::from(block_header).bytes().clone();
        self.backend
            .put(DBTable::Headers.namespace(), hash_key, header_value)
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
            let hash_key = BlockHashRLP::from(block_hash).bytes().clone();
            let header_value = BlockHeaderRLP::from(header.clone()).bytes().clone();

            // Add header to headers table
            batch_ops.push(BatchOperation::Put {
                table: DBTable::Headers,
                key: hash_key.clone(),
                value: header_value,
            });

            // Add block number mapping
            let number_key = header.number.to_le_bytes().to_vec();
            batch_ops.push(BatchOperation::Put {
                table: DBTable::BlockNumbers,
                key: hash_key,
                value: number_key,
            });
        }

        self.execute_batch(batch_ops).await
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
        let hash_key = BlockHashRLP::from(block_hash).bytes().clone();
        let body_value = BlockBodyRLP::from(block_body).bytes().clone();
        self.backend
            .put(DBTable::Bodies.namespace(), hash_key, body_value)
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

        let canonical = BatchOperation::Delete {
            table: DBTable::Headers,
            key: block_number.to_le_bytes().to_vec(),
        };
        let body = BatchOperation::Delete {
            table: DBTable::BlockNumbers,
            key: hash.as_bytes().to_vec(),
        };
        let batch_body = BatchOperation::Delete {
            table: DBTable::Bodies,
            key: hash.as_bytes().to_vec(),
        };
        let batch_number = BatchOperation::Delete {
            table: DBTable::BlockNumbers,
            key: hash.as_bytes().to_vec(),
        };

        self.execute_batch(vec![canonical, body, batch_body, batch_number])
            .await
    }

    /// Obtain canonical block bodies in from..=to
    pub async fn get_block_bodies(
        &self,
        from: BlockNumber,
        to: BlockNumber,
    ) -> Result<Vec<BlockBody>, StoreError> {
        let numbers: Vec<BlockNumber> = (from..=to).collect();
        let number_keys: Vec<Vec<u8>> = numbers.iter().map(|n| n.to_le_bytes().to_vec()).collect();
        let hashes = self
            .backend
            .get_async_batch(DBTable::CanonicalHashes.namespace(), number_keys)
            .await?;

        let bodies = self
            .backend
            .get_async_batch(DBTable::Bodies.namespace(), hashes)
            .await?;

        bodies
            .into_iter()
            .map(|bytes| {
                BlockBodyRLP::from_bytes(bytes)
                    .to()
                    .map_err(StoreError::from)
            })
            .collect::<Result<Vec<BlockBody>, StoreError>>()
    }

    /// Obtain block bodies from a list of hashes
    pub async fn get_block_bodies_by_hash(
        &self,
        hashes: Vec<BlockHash>,
    ) -> Result<Vec<BlockBody>, StoreError> {
        let hash_keys: Vec<Vec<u8>> = hashes
            .iter()
            .map(|hash| BlockHashRLP::from(*hash).bytes().clone())
            .collect();

        let bodies: Vec<BlockBody> = self
            .backend
            .get_async_batch(DBTable::Bodies.namespace(), hash_keys)
            .await?
            .into_iter()
            .map(|bytes| {
                BlockBodyRLP::from_bytes(bytes)
                    .to()
                    .map_err(StoreError::from)
            })
            .collect::<Result<Vec<BlockBody>, StoreError>>()?;

        Ok(bodies)
    }

    /// Obtain any block body using the hash
    pub async fn get_block_body_by_hash(
        &self,
        block_hash: BlockHash,
    ) -> Result<Option<BlockBody>, StoreError> {
        let hash_key = BlockHashRLP::from(block_hash).bytes().clone();
        self.backend
            .get_async(DBTable::Bodies.namespace(), hash_key)
            .await?
            .map(|bytes| BlockBodyRLP::from_bytes(bytes).to())
            .transpose()
            .map_err(StoreError::from)
    }

    pub fn get_block_header_by_hash(
        &self,
        block_hash: BlockHash,
    ) -> Result<Option<BlockHeader>, StoreError> {
        let hash_key = BlockHashRLP::from(block_hash).bytes().clone();
        self.backend
            .get_sync(DBTable::Headers.namespace(), hash_key)?
            .map(|bytes| BlockHeaderRLP::from_bytes(bytes).to())
            .transpose()
            .map_err(StoreError::from)
    }

    pub async fn add_pending_block(&self, block: Block) -> Result<(), StoreError> {
        let hash_key = BlockHashRLP::from(block.hash()).bytes().clone();
        let block_value = BlockRLP::from(block).bytes().clone();
        self.backend
            .put(DBTable::PendingBlocks.namespace(), hash_key, block_value)
            .await
    }
    pub async fn get_pending_block(
        &self,
        block_hash: BlockHash,
    ) -> Result<Option<Block>, StoreError> {
        let hash_key = BlockHashRLP::from(block_hash).bytes().clone();
        self.backend
            .get_async(DBTable::PendingBlocks.namespace(), hash_key)
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
        let hash_key = BlockHashRLP::from(block_hash).bytes().clone();
        let number_value = block_number.to_le_bytes().to_vec();
        self.backend
            .put(DBTable::BlockNumbers.namespace(), hash_key, number_value)
            .await
    }

    /// Obtain block number for a given hash
    pub async fn get_block_number(
        &self,
        block_hash: BlockHash,
    ) -> Result<Option<BlockNumber>, StoreError> {
        let hash_key = BlockHashRLP::from(block_hash).bytes().clone();
        self.backend
            .get_async(DBTable::BlockNumbers.namespace(), hash_key)
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
        let mut composite_key = Vec::with_capacity(64);
        composite_key.extend_from_slice(transaction_hash.as_bytes());
        composite_key.extend_from_slice(block_hash.as_bytes());
        let location_value = (block_number, block_hash, index).encode_to_vec();
        self.backend
            .put(
                DBTable::TransactionLocations.namespace(),
                composite_key,
                location_value,
            )
            .await
    }

    /// Store transaction locations in batch (one db transaction for all)
    pub async fn add_transaction_locations(
        &self,
        locations: Vec<(H256, BlockNumber, BlockHash, Index)>,
    ) -> Result<(), StoreError> {
        let mut batch_ops = Vec::new();
        for (tx_hash, block_number, block_hash, index) in locations {
            // Key: tx_hash + block_hash
            let mut composite_key = Vec::with_capacity(64);
            composite_key.extend_from_slice(tx_hash.as_bytes());
            composite_key.extend_from_slice(block_hash.as_bytes());

            let location_value = (block_number, block_hash, index).encode_to_vec();

            batch_ops.push(BatchOperation::Put {
                table: DBTable::TransactionLocations,
                key: composite_key,
                value: location_value,
            });
        }
        self.execute_batch(batch_ops).await
    }

    /// Obtain transaction location (block hash and index)
    pub async fn get_transaction_location(
        &self,
        transaction_hash: H256,
    ) -> Result<Option<(BlockNumber, BlockHash, Index)>, StoreError> {
        // Use range query to find transaction location
        // Key pattern: tx_hash + block_hash
        let start_key = transaction_hash.as_bytes().to_vec();
        let mut end_key = start_key.clone();
        end_key.push(0xFF); // Extend to get all entries with this tx_hash prefix

        let results = self
            .backend
            .range(
                DBTable::TransactionLocations.namespace(),
                start_key,
                Some(end_key),
            )
            .await?;

        // Check each location to see if it's in the canonical chain
        for (_, value_bytes) in results {
            let location: (BlockNumber, BlockHash, Index) = RLPDecode::decode(&value_bytes)?;

            // Check if this block is canonical
            let canonical_hash = self.get_canonical_block_hash_sync(location.0)?;
            if canonical_hash == Some(location.1) {
                return Ok(Some(location));
            }
        }

        Ok(None)
    }

    /// Add receipt
    pub async fn add_receipt(
        &self,
        block_hash: BlockHash,
        index: Index,
        receipt: Receipt,
    ) -> Result<(), StoreError> {
        let key = (block_hash, index).encode_to_vec();
        let value = receipt.encode_to_vec();
        self.backend
            .put(DBTable::Receipts.namespace(), key, value)
            .await
    }

    /// Add receipts
    pub async fn add_receipts(
        &self,
        block_hash: BlockHash,
        receipts: Vec<Receipt>,
    ) -> Result<(), StoreError> {
        let mut batch_ops = Vec::new();
        for (index, receipt) in receipts.into_iter().enumerate() {
            let key = (block_hash, index as u64).encode_to_vec();
            let value = receipt.encode_to_vec();

            batch_ops.push(BatchOperation::Put {
                table: DBTable::Receipts,
                key,
                value,
            });
        }
        self.execute_batch(batch_ops).await
    }

    /// Obtain receipt by block hash and index
    pub async fn get_receipt(
        &self,
        block_hash: BlockHash,
        index: Index,
    ) -> Result<Option<Receipt>, StoreError> {
        let key = (block_hash, index).encode_to_vec();
        self.backend
            .get_async(DBTable::Receipts.namespace(), key)
            .await?
            .map(|bytes| Receipt::decode(bytes.as_slice()))
            .transpose()
            .map_err(StoreError::from)
    }

    /// Add account code
    pub async fn add_account_code(&self, code_hash: H256, code: Bytes) -> Result<(), StoreError> {
        let hash_key = code_hash.as_bytes().to_vec();
        let code_value = AccountCodeRLP::from(code).bytes().clone();
        self.backend
            .put(DBTable::AccountCodes.namespace(), hash_key, code_value)
            .await
    }

    /// Clears all checkpoint data created during the last snap sync
    pub async fn clear_snap_state(&self) -> Result<(), StoreError> {
        // Clear all snap state data by removing all entries in the SnapState table
        // Since we don't have a clear_table method, we'll use range to get all keys and delete them
        let all_data = self
            .backend
            .range(DBTable::SnapState.namespace(), [0].to_vec(), None)
            .await?;

        if all_data.is_empty() {
            return Ok(());
        }

        let mut batch_ops = Vec::new();
        for (key, _) in all_data {
            batch_ops.push(BatchOperation::Delete {
                table: DBTable::SnapState,
                key,
            });
        }

        self.execute_batch(batch_ops).await
    }

    /// Obtain account code via code hash
    pub fn get_account_code(&self, code_hash: H256) -> Result<Option<Bytes>, StoreError> {
        let hash_key = code_hash.as_bytes().to_vec();
        self.backend
            .get_sync(DBTable::AccountCodes.namespace(), hash_key)?
            .map(|bytes| AccountCodeRLP::from_bytes(bytes).to())
            .transpose()
            .map_err(StoreError::from)
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
        // TODO: Fix error handling
        let index: usize = index
            .try_into()
            .map_err(|_| StoreError::Custom("Invalid index".to_string()))?;
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
        let number_key = block_number.to_le_bytes().to_vec();
        self.backend
            .get_async(DBTable::CanonicalHashes.namespace(), number_key)
            .await?
            .map(|bytes| BlockHashRLP::from_bytes(bytes).to())
            .transpose()
            .map_err(StoreError::from)
    }

    /// Stores the chain configuration values, should only be called once after reading the genesis file
    /// Ignores previously stored values if present
    pub async fn set_chain_config(&self, chain_config: &ChainConfig) -> Result<(), StoreError> {
        let key = Self::chain_data_key(ChainDataIndex::ChainConfig);
        let value = serde_json::to_string(chain_config)
            .map_err(|_| StoreError::Custom("Failed to serialize chain config".to_string()))?
            .into_bytes();
        self.backend
            .put(DBTable::ChainData.namespace(), key, value)
            .await
    }

    /// Update earliest block number
    pub async fn update_earliest_block_number(
        &self,
        block_number: BlockNumber,
    ) -> Result<(), StoreError> {
        let key = vec![ChainDataIndex::EarliestBlockNumber as u8];
        let value = block_number.to_le_bytes().to_vec();
        self.backend
            .put(DBTable::ChainData.namespace(), key, value)
            .await
    }

    /// Obtain earliest block number
    pub async fn get_earliest_block_number(&self) -> Result<Option<BlockNumber>, StoreError> {
        let key = vec![ChainDataIndex::EarliestBlockNumber as u8];
        self.backend
            .get_async(DBTable::ChainData.namespace(), key)
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
        self.backend
            .get_async(DBTable::ChainData.namespace(), key)
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
        self.backend
            .get_async(DBTable::ChainData.namespace(), key)
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
        self.backend
            .get_async(DBTable::ChainData.namespace(), key)
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
        self.backend
            .put(DBTable::ChainData.namespace(), key, value)
            .await
    }

    /// Obtain pending block number
    pub async fn get_pending_block_number(&self) -> Result<Option<BlockNumber>, StoreError> {
        let key = vec![ChainDataIndex::PendingBlockNumber as u8];
        self.backend
            .get_async(DBTable::ChainData.namespace(), key)
            .await?
            .map(|bytes| -> Result<BlockNumber, StoreError> {
                let array: [u8; 8] = bytes
                    .try_into()
                    .map_err(|_| StoreError::Custom("Invalid BlockNumber bytes".to_string()))?;
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
    ) -> Result<(), StoreError> {
        let latest = self.get_latest_block_number().await?.unwrap_or(0);
        let mut batch_ops = Vec::new();

        // Update canonical block hashes
        if let Some(canonical_blocks) = new_canonical_blocks {
            for (block_number, block_hash) in canonical_blocks {
                let number_key = block_number.to_le_bytes().to_vec();
                let hash_value = BlockHashRLP::from(block_hash).bytes().clone();
                batch_ops.push(BatchOperation::Put {
                    table: DBTable::CanonicalHashes,
                    key: number_key,
                    value: hash_value,
                });
            }
        }

        // Remove anything after the head from the canonical chain
        for number in (head_number + 1)..=(latest) {
            let number_key = number.to_le_bytes().to_vec();
            batch_ops.push(BatchOperation::Delete {
                table: DBTable::CanonicalHashes,
                key: number_key,
            });
        }

        // Make head canonical
        let head_key = head_number.to_le_bytes().to_vec();
        let head_value = BlockHashRLP::from(head_hash).bytes().clone();
        batch_ops.push(BatchOperation::Put {
            table: DBTable::CanonicalHashes,
            key: head_key,
            value: head_value,
        });

        // Update chain data
        let latest_key = Self::chain_data_key(ChainDataIndex::LatestBlockNumber);
        batch_ops.push(BatchOperation::Put {
            table: DBTable::ChainData,
            key: latest_key,
            value: head_number.to_le_bytes().to_vec(),
        });

        if let Some(safe_number) = safe {
            let safe_key = Self::chain_data_key(ChainDataIndex::SafeBlockNumber);
            batch_ops.push(BatchOperation::Put {
                table: DBTable::ChainData,
                key: safe_key,
                value: safe_number.to_le_bytes().to_vec(),
            });
        }

        if let Some(finalized_number) = finalized {
            let finalized_key = Self::chain_data_key(ChainDataIndex::FinalizedBlockNumber);
            batch_ops.push(BatchOperation::Put {
                table: DBTable::ChainData,
                key: finalized_key,
                value: finalized_number.to_le_bytes().to_vec(),
            });
        }

        self.execute_batch(batch_ops).await
    }

    pub fn get_receipts_for_block(
        &self,
        block_hash: &BlockHash,
    ) -> Result<Vec<Receipt>, StoreError> {
        let mut receipts = Vec::new();
        let mut index = 0u64;
        loop {
            let key = (*block_hash, index).encode_to_vec();
            match self.backend.get_sync(DBTable::Receipts.namespace(), key)? {
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
    ) -> Result<(), StoreError> {
        let key = vec![SnapStateIndex::HeaderDownloadCheckpoint as u8];
        let value = BlockHashRLP::from(block_hash).bytes().clone();
        self.backend
            .put(DBTable::SnapState.namespace(), key, value)
            .await
    }

    /// Gets the hash of the last header downloaded during a snap sync
    pub async fn get_header_download_checkpoint(&self) -> Result<Option<BlockHash>, StoreError> {
        let key = vec![SnapStateIndex::HeaderDownloadCheckpoint as u8];
        self.backend
            .get_async(DBTable::SnapState.namespace(), key)
            .await?
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
        self.backend
            .put(DBTable::SnapState.namespace(), key, value)
            .await
    }

    /// Gets the last key fetched from the state trie being fetched during snap sync
    pub async fn get_state_trie_key_checkpoint(
        &self,
    ) -> Result<Option<[H256; STATE_TRIE_SEGMENTS]>, StoreError> {
        let key = vec![SnapStateIndex::StateTrieKeyCheckpoint as u8];
        self.backend
            .get_async(DBTable::SnapState.namespace(), key)
            .await?
            .map(|bytes| -> Result<[H256; STATE_TRIE_SEGMENTS], StoreError> {
                let keys_vec: Vec<H256> = Vec::<H256>::decode(bytes.as_slice())?;
                keys_vec
                    .try_into()
                    .map_err(|_| StoreError::Custom("Invalid array size".to_string()))
            })
            .transpose()
    }

    /// Sets the state trie paths in need of healing
    pub async fn set_state_heal_paths(
        &self,
        paths: Vec<(Nibbles, H256)>,
    ) -> Result<(), StoreError> {
        let key = vec![SnapStateIndex::StateHealPaths as u8];
        let value = paths.encode_to_vec();
        self.backend
            .put(DBTable::SnapState.namespace(), key, value)
            .await
    }

    /// Gets the state trie paths in need of healing
    pub async fn get_state_heal_paths(&self) -> Result<Option<Vec<(Nibbles, H256)>>, StoreError> {
        let key = vec![SnapStateIndex::StateHealPaths as u8];
        self.backend
            .get_async(DBTable::SnapState.namespace(), key)
            .await?
            .map(|bytes| Vec::<(Nibbles, H256)>::decode(bytes.as_slice()))
            .transpose()
            .map_err(StoreError::from)
    }

    /// Write a storage batch into the current storage snapshot
    pub async fn write_snapshot_storage_batch(
        &self,
        account_hash: H256,
        storage_keys: Vec<H256>,
        storage_values: Vec<U256>,
    ) -> Result<(), StoreError> {
        if storage_keys.len() != storage_values.len() {
            return Err(StoreError::Custom(
                "Storage keys and values length mismatch".to_string(),
            ));
        }

        let mut batch_ops = Vec::new();
        for (key, value) in storage_keys.into_iter().zip(storage_values.into_iter()) {
            let mut composite_key = Vec::with_capacity(64);
            composite_key.extend_from_slice(account_hash.as_bytes());
            composite_key.extend_from_slice(key.as_bytes());
            let value_bytes = u256_to_big_endian(value).to_vec();

            batch_ops.push(BatchOperation::Put {
                table: DBTable::StorageSnapshot,
                key: composite_key,
                value: value_bytes,
            });
        }

        self.execute_batch(batch_ops).await
    }

    /// Write multiple storage batches belonging to different accounts into the current storage snapshot
    pub async fn write_snapshot_storage_batches(
        &self,
        account_hashes: Vec<H256>,
        storage_keys: Vec<Vec<H256>>,
        storage_values: Vec<Vec<U256>>,
    ) -> Result<(), StoreError> {
        if account_hashes.len() != storage_keys.len()
            || account_hashes.len() != storage_values.len()
        {
            return Err(StoreError::Custom(
                "Account hashes, keys, and values length mismatch".to_string(),
            ));
        }

        let mut batch_ops = Vec::new();
        for ((account_hash, keys), values) in account_hashes
            .into_iter()
            .zip(storage_keys.into_iter())
            .zip(storage_values.into_iter())
        {
            if keys.len() != values.len() {
                return Err(StoreError::Custom(
                    "Storage keys and values length mismatch for account".to_string(),
                ));
            }

            for (key, value) in keys.into_iter().zip(values.into_iter()) {
                // Create composite key: account_hash + storage_key
                let mut composite_key = Vec::with_capacity(64);
                composite_key.extend_from_slice(account_hash.as_bytes());
                composite_key.extend_from_slice(key.as_bytes());
                let value_bytes = u256_to_big_endian(value).to_vec();

                batch_ops.push(BatchOperation::Put {
                    table: DBTable::StorageSnapshot,
                    key: composite_key,
                    value: value_bytes,
                });
            }
        }

        self.execute_batch(batch_ops).await
    }

    /// Set the latest root of the rebuilt state trie and the last downloaded hashes from each segment
    pub async fn set_state_trie_rebuild_checkpoint(
        &self,
        checkpoint: (H256, [H256; STATE_TRIE_SEGMENTS]),
    ) -> Result<(), StoreError> {
        let key = vec![SnapStateIndex::StateTrieRebuildCheckpoint as u8];
        let value = (checkpoint.0, checkpoint.1.to_vec()).encode_to_vec();
        self.backend
            .put(DBTable::SnapState.namespace(), key, value)
            .await
    }

    /// Get the latest root of the rebuilt state trie and the last downloaded hashes from each segment
    pub async fn get_state_trie_rebuild_checkpoint(
        &self,
    ) -> Result<Option<(H256, [H256; STATE_TRIE_SEGMENTS])>, StoreError> {
        let key = vec![SnapStateIndex::StateTrieRebuildCheckpoint as u8];
        self.backend
            .get_async(DBTable::SnapState.namespace(), key)
            .await?
            .map(
                |bytes| -> Result<(H256, [H256; STATE_TRIE_SEGMENTS]), StoreError> {
                    let (root, segments_vec): (H256, Vec<H256>) = RLPDecode::decode(&bytes)?;
                    let segments: [H256; STATE_TRIE_SEGMENTS] =
                        segments_vec.try_into().map_err(|_| {
                            StoreError::Custom("Invalid segments array size".to_string())
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
    ) -> Result<(), StoreError> {
        let key = vec![SnapStateIndex::StorageTrieRebuildPending as u8];
        let value = pending.encode_to_vec();
        self.backend
            .put(DBTable::SnapState.namespace(), key, value)
            .await
    }

    /// Get the accont hashes and roots of the storage tries awaiting rebuild
    pub async fn get_storage_trie_rebuild_pending(
        &self,
    ) -> Result<Option<Vec<(H256, H256)>>, StoreError> {
        let key = vec![SnapStateIndex::StorageTrieRebuildPending as u8];
        self.backend
            .get_async(DBTable::SnapState.namespace(), key)
            .await?
            .map(|bytes| Vec::<(H256, H256)>::decode(bytes.as_slice()))
            .transpose()
            .map_err(StoreError::from)
    }

    /// Reads the next `MAX_SNAPSHOT_READS` elements from the storage snapshot as from the `start` storage key
    pub async fn read_storage_snapshot(
        &self,
        start: H256,
        account_hash: H256,
    ) -> Result<Vec<(H256, U256)>, StoreError> {
        // Create start key: account_hash + start
        let mut start_key = Vec::with_capacity(64);
        start_key.extend_from_slice(account_hash.as_bytes());
        start_key.extend_from_slice(start.as_bytes());

        // Create end key: account_hash + max
        let mut end_key = Vec::with_capacity(64);
        end_key.extend_from_slice(account_hash.as_bytes());
        end_key.extend_from_slice(&[0xFF; 32]); // Max possible H256

        let results = self
            .backend
            .range(
                DBTable::StorageSnapshot.namespace(),
                start_key,
                Some(end_key),
            )
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
    ) -> Result<(), StoreError> {
        let key = BlockHashRLP::from(bad_block).bytes().clone();
        let value = BlockHashRLP::from(latest_valid).bytes().clone();
        self.backend
            .put(DBTable::InvalidAncestors.namespace(), key, value)
            .await
    }

    /// Returns the latest valid ancestor hash for a given invalid block hash.
    /// Used to provide `latest_valid_hash` in the Engine API when processing invalid payloads.
    pub async fn get_latest_valid_ancestor(
        &self,
        block: BlockHash,
    ) -> Result<Option<BlockHash>, StoreError> {
        let key = BlockHashRLP::from(block).bytes().clone();
        self.backend
            .get_async(DBTable::InvalidAncestors.namespace(), key)
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
        let hash_key = BlockHashRLP::from(block_hash).bytes().clone();

        self.backend
            .get_sync(DBTable::BlockNumbers.namespace(), hash_key)?
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
        let number_key = block_number.to_le_bytes().to_vec();
        match self
            .backend
            .get_sync(DBTable::CanonicalHashes.namespace(), number_key)?
        {
            Some(bytes) => {
                let rlp = BlockHashRLP::from_bytes(bytes)
                    .to()
                    .map_err(StoreError::from)?;
                Ok(Some(rlp))
            }
            None => Ok(None),
        }
    }

    pub async fn write_storage_trie_nodes_batch(
        &self,
        storage_trie_nodes: StorageTrieNodes,
    ) -> Result<(), StoreError> {
        let mut batch_ops = Vec::new();
        for (address_hash, nodes) in storage_trie_nodes {
            for (node_hash, node_data) in nodes {
                // Create composite key: address_hash + node_hash
                let mut key = Vec::with_capacity(64);
                key.extend_from_slice(address_hash.as_bytes());
                key.extend_from_slice(node_hash.as_ref());

                batch_ops.push(BatchOperation::Put {
                    table: DBTable::StorageTrieNodes,
                    key,
                    value: node_data,
                });
            }
        }

        self.execute_batch(batch_ops).await
    }

    pub async fn write_account_code_batch(
        &self,
        account_codes: Vec<(H256, Bytes)>,
    ) -> Result<(), StoreError> {
        let mut batch_ops = Vec::new();
        for (code_hash, code) in account_codes {
            let key = code_hash.as_bytes().to_vec();
            let value = AccountCodeRLP::from(code).bytes().clone();

            batch_ops.push(BatchOperation::Put {
                table: DBTable::AccountCodes,
                key,
                value,
            });
        }

        self.execute_batch(batch_ops).await
    }

    /// Obtain a state trie from the given state root
    /// Doesn't check if the state root is valid
    /// Used for internal store operations
    pub fn open_state_trie(&self, state_root: H256) -> Result<Trie, StoreError> {
        let trie_db = Box::new(StorageBackendTrieDB::new_state_trie(self.backend.clone()));
        let result = Trie::open(trie_db, state_root);
        Ok(result)
    }

    /// Obtain a storage trie from the given address and storage_root
    /// Doesn't check if the account is stored
    /// Used for internal store operations
    pub fn open_storage_trie(
        &self,
        hashed_address: H256,
        storage_root: H256,
    ) -> Result<Trie, StoreError> {
        let trie_db = Box::new(StorageBackendTrieDB::new_storage_trie(
            self.backend.clone(),
            hashed_address,
        ));
        Ok(Trie::open(trie_db, storage_root))
    }

    /// Obtain a state trie locked for reads from the given state root
    /// Doesn't check if the state root is valid
    /// Used for internal store operations
    pub fn open_locked_state_trie(&self, state_root: H256) -> Result<Trie, StoreError> {
        let trie_db = Box::new(StorageBackendLockedTrieDB::new_state_trie(
            self.backend.clone(),
        ));
        Ok(Trie::open(trie_db, state_root))
    }

    /// Obtain a read-locked storage trie from the given address and storage_root
    /// Doesn't check if the account is stored
    /// Used for internal store operations
    pub fn open_locked_storage_trie(
        &self,
        hashed_address: H256,
        storage_root: H256,
    ) -> Result<Trie, StoreError> {
        let trie_db = Box::new(StorageBackendLockedTrieDB::new_storage_trie(
            self.backend.clone(),
            hashed_address,
        ));
        Ok(Trie::open(trie_db, storage_root))
    }
}

impl Engine {
    // Helper methods for key generation
    pub fn chain_data_key(index: ChainDataIndex) -> Vec<u8> {
        vec![index as u8]
    }
}
