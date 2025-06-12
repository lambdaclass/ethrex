use alloy_primitives::FixedBytes;
use reth_db::table::DupSort;
use std::ops::Div;
use std::path::Path;
use std::sync::Arc;
use std::sync::OnceLock;

use crate::api::StoreEngine;
use crate::error::StoreError;
use crate::rlp::Rlp;
use crate::store::{MAX_SNAPSHOT_READS, STATE_TRIE_SEGMENTS};
use crate::trie_db::mdbx_fork::MDBXTrieDB;
use crate::trie_db::mdbx_fork::MDBXTrieWithFixedKey;
use crate::trie_db::utils::node_hash_to_fixed_size;
use crate::utils::{ChainDataIndex, SnapStateIndex};
use crate::UpdateBatch;
use alloy_primitives::B256;
use anyhow::Result;
use bytes::Bytes;
use ethereum_types::{H256, U256};
use ethrex_common::types::{
    payload::PayloadBundle, AccountState, Block, BlockBody, BlockHash, BlockHeader, BlockNumber,
    ChainConfig, Index, Receipt, Transaction,
};
use ethrex_rlp::decode::RLPDecode;
use ethrex_rlp::encode::RLPEncode;
use ethrex_rlp::error::RLPDecodeError;
use ethrex_trie::{Nibbles, Trie};
use reth_db::mdbx::{DatabaseArguments, DatabaseEnv};
use reth_db::tables;
use reth_db::DatabaseEnvKind;
use reth_db::{
    transaction::{DbTx, DbTxMut},
    Database,
};
use reth_db_api::cursor::DbCursorRO;
use reth_db_api::cursor::DbCursorRW;
use reth_db_api::cursor::DbDupCursorRO;

static DB_DUPSORT_MAX_SIZE: OnceLock<usize> = OnceLock::new();

pub struct MDBXFork {
    env: Arc<DatabaseEnv>,
    state_trie: Arc<MDBXTrieDB<StateTrieNodes>>,
}

impl std::fmt::Debug for MDBXFork {
    fn fmt(&self, _f: &mut Formatter<'_>) -> Result<(), Error> {
        todo!()
    }
}

impl MDBXFork {
    pub fn new(path: &str) -> Result<Self, StoreError> {
        let client_version = Default::default();
        let db_args = DatabaseArguments::new(client_version);
        // FIXME: Use DatabaseEnv
        let env = Arc::new(
            DatabaseEnv::open(Path::new(&path), DatabaseEnvKind::RW, db_args)
                .expect("Failed to initialize MDBX Fork"),
        );
        // https://libmdbx.dqdkfa.ru/intro.html#autotoc_md5
        // Value size: minimum 0, maximum 2146435072 (0x7FF00000) bytes for maps,
        // ≈½ pagesize for multimaps (2022 bytes for default 4K pagesize,
        // 32742 bytes for 64K pagesize).
        DB_DUPSORT_MAX_SIZE.get_or_init(|| page_size::get().div(2));

        let tx = env.begin_rw_txn()?;

        tx.create_db(Some("StateTrieNodes"), DatabaseFlags::default())?;
        tx.create_db(Some("StorageTriesNodes"), DatabaseFlags::default())?;
        tx.create_db(Some("Receipts"), DatabaseFlags::default())?;
        tx.create_db(Some("Bytecodes"), DatabaseFlags::default())?;
        tx.create_db(Some("TransactionLocations"), DatabaseFlags::DUP_SORT)?;
        tx.create_db(Some("Bodies"), DatabaseFlags::default())?;
        tx.create_db(Some("Headers"), DatabaseFlags::default())?;
        tx.create_db(Some("BlockNumbers"), DatabaseFlags::default())?;
        tx.create_db(Some("PendingBlocks"), DatabaseFlags::default())?;
        tx.create_db(Some("CanonicalBlockHashes"), DatabaseFlags::default())?;
        tx.create_db(Some("ChainData"), DatabaseFlags::default())?;
        tx.create_db(Some("SnapState"), DatabaseFlags::default())?;
        tx.create_db(Some("Payloads"), DatabaseFlags::default())?;
        tx.create_db(Some("StateSnapShot"), DatabaseFlags::DUP_SORT)?;
        tx.create_db(Some("StorageSnapShot"), DatabaseFlags::DUP_SORT)?;
        tx.create_db(Some("StorageHealPaths"), DatabaseFlags::DUP_SORT)?;
        tx.create_db(Some("InvalidAncestor"), DatabaseFlags::default())?;
        tx.commit()?;
        let state_trie = Arc::new(MDBXTrieDB::new(env.clone()));

        Ok(Self { env, state_trie })
    }
}

use reth_db_api::table::Table as RethTable;
use reth_libmdbx::DatabaseFlags;

use reth_db::TableType;
use reth_db::TableViewer;
use std::fmt::{self, Error, Formatter};

tables! {
    table StorageTriesNodes<Key = Vec<u8>, Value = Vec<u8>>;
    table StateTrieNodes<Key = FixedBytes<32>, Value = Vec<u8>>;
    table Receipts<Key = Vec<u8>, Value = Vec<u8>>;
    table TransactionLocations<Key = Vec<u8>, Value = Vec<u8>, SubKey = Vec<u8>>;
    table Bodies<Key = Vec<u8>, Value = Vec<u8>>;
    table Headers<Key = Vec<u8>, Value = Vec<u8>>;
    table CanonicalBlockHashes<Key = u64, Value = Vec<u8>>;
    table BlockNumbers<Key = Vec<u8>, Value = u64>;
    table PendingBlocks<Key = Vec<u8>, Value = Vec<u8>>;
    table ChainData<Key = u8, Value = Vec<u8>>;
    table SnapState<Key = u8, Value = Vec<u8>>;
    table Payloads<Key = u64, Value = Vec<u8>>;
    table StateSnapShot<Key = Vec<u8>, Value = Vec<u8>, SubKey = Vec<u8>>;
    table StorageSnapshot<Key = Vec<u8>, Value = Vec<u8>, SubKey = Vec<u8>>;
    table StorageHealPaths<Key = Vec<u8>, Value = Vec<u8>>;
    table InvalidAncestor<Key = Vec<u8>, Value = Vec<u8>>;
    table Bytecodes<Key = B256, Value = Vec<u8>>;
}

impl MDBXFork {
    // Helper method to write into a libmdbx table
    async fn write<T: RethTable>(&self, key: T::Key, value: T::Value) -> Result<(), StoreError> {
        let db = self.env.clone();

        tokio::task::spawn_blocking(move || -> Result<(), StoreError> {
            let tx = db.tx_mut()?;
            tx.put::<T>(key, value)?;
            tx.commit()?;
            Ok(())
        })
        .await
        .map_err(|e| StoreError::Custom(format!("task panicked: {e}")))?
    }

    // Helper method to write into a libmdbx table in batch
    async fn write_batch<T: RethTable>(
        &self,
        key_values: Vec<(T::Key, T::Value)>,
    ) -> Result<(), StoreError> {
        let db = self.env.clone();
        tokio::task::spawn_blocking(move || -> Result<(), StoreError> {
            let tx = db.tx_mut()?;

            let mut cursor = tx.cursor_write::<T>()?;
            for (key, value) in key_values {
                cursor.upsert(key, value)?;
            }
            tx.commit()?;
            Ok(())
        })
        .await
        .map_err(|e| StoreError::Custom(format!("task panicked: {e}")))?
    }

    // Helper method to read from a libmdbx table
    async fn read<T: RethTable>(&self, key: T::Key) -> Result<Option<T::Value>, StoreError> {
        let db = self.env.clone();
        tokio::task::spawn_blocking(move || {
            let tx = db.tx()?;
            Ok(tx.get::<T>(key)?)
        })
        .await
        .map_err(|e| StoreError::Custom(format!("task panicked: {e}")))?
    }

    // Helper method to read from a libmdbx table
    // async fn read_bulk<T: RethTable>(
    //     &self,
    //     keys: Vec<T::Key>,
    // ) -> Result<Vec<T::Value>, StoreError> {
    //     let db = self.env.clone();
    //     tokio::task::spawn_blocking(move || {
    //         let mut res = Vec::new();
    //         let tx = db.tx()?;
    //         for key in keys {
    //             let val = tx.get::<T>(key)?;
    //             match val {
    //                 Some(val) => res.push(val),
    //                 None => Err(StoreError::ReadError)?,
    //             }
    //         }
    //         Ok(res)
    //     })
    //     .await
    //     .map_err(|e| StoreError::Custom(format!("task panicked: {e}")))?
    // }

    // Helper method to read from a libmdbx table
    fn read_sync<T: RethTable>(&self, key: T::Key) -> Result<Option<T::Value>, StoreError> {
        let txn = self.env.tx()?;
        Ok(txn.get::<T>(key)?)
    }

    fn get_block_hash_by_block_number(
        &self,
        number: BlockNumber,
    ) -> Result<Option<BlockHash>, StoreError> {
        self.read_sync::<CanonicalBlockHashes>(number)?
            .map(|block_hash| Rlp::<BlockHash>::from_bytes(block_hash).to())
            .transpose()
            .map_err(StoreError::from)
    }
}

#[async_trait::async_trait]
impl StoreEngine for MDBXFork {
    async fn apply_updates(&self, update_batch: UpdateBatch) -> Result<(), StoreError> {
        let db = self.env.clone();
        tokio::task::spawn_blocking(move || -> Result<(), StoreError> {
            let tx = db.tx_mut()?;

            // store account updates
            for (node_hash, node_data) in update_batch.account_updates {
                let node_hash_bytes =
                    FixedBytes::new(node_hash.as_ref().try_into().expect("should always fit"));
                tx.put::<StateTrieNodes>(node_hash_bytes, node_data)?;
            }

            for (hashed_address, nodes) in update_batch.storage_updates {
                for (node_hash, node_data) in nodes {
                    let key = node_hash_to_fixed_size(node_hash);
                    let full_key = [&hashed_address.0, key.as_ref()].concat();
                    tx.put::<StorageTriesNodes>(full_key, node_data)?;
                }
            }

            for block in update_batch.blocks {
                // store block
                let number = block.header.number;
                let hash = block.hash();
                for (index, transaction) in block.body.transactions.iter().enumerate() {
                    tx.put::<TransactionLocations>(
                        transaction.compute_hash().0.encode_to_vec(),
                        (number, hash, index as u64).encode_to_vec(),
                    )?;
                }

                tx.put::<Bodies>(hash.encode_to_vec(), block.body.encode_to_vec())?;
                tx.put::<Headers>(hash.encode_to_vec(), block.header.encode_to_vec())?;
                tx.put::<BlockNumbers>(hash.encode_to_vec(), number)?;
            }

            for (block_hash, receipts) in update_batch.receipts {
                // store receipts
                // TODO: the non-fork uses indexed chunk here to use a cursor.
                // consider implementing it too
                for (index, receipt) in receipts.into_iter().enumerate() {
                    let receipt_bytes = receipt.encode_to_vec();
                    let receipt_db_key = (block_hash, index).encode_to_vec();
                    tx.put::<Receipts>(receipt_db_key, receipt_bytes)?;
                }
            }

            tx.commit()?;
            Ok(())
        })
        .await
        .map_err(|e| StoreError::Custom(format!("task panicked: {e}")))?
    }

    async fn add_block_header(
        &self,
        block_hash: BlockHash,
        block_header: BlockHeader,
    ) -> Result<(), StoreError> {
        self.write::<Headers>(block_hash.encode_to_vec(), block_header.encode_to_vec())
            .await
    }

    async fn add_block_headers(
        &self,
        block_hashes: Vec<BlockHash>,
        block_headers: Vec<BlockHeader>,
    ) -> Result<(), StoreError> {
        let hashes_and_headers = block_hashes
            .into_iter()
            .zip(block_headers)
            .map(|(hash, header)| (hash.encode_to_vec(), header.encode_to_vec()))
            .collect();
        self.write_batch::<Headers>(hashes_and_headers).await
    }

    fn get_block_header(
        &self,
        block_number: BlockNumber,
    ) -> Result<Option<BlockHeader>, StoreError> {
        let Some(block_hash) = self.get_block_hash_by_block_number(block_number)? else {
            return Ok(None);
        };

        self.read_sync::<Headers>(block_hash.encode_to_vec())?
            .map(|b| Rlp::from_bytes(b).to())
            .transpose()
            .map_err(StoreError::from)
    }

    async fn add_block_body(
        &self,
        block_hash: BlockHash,
        block_body: BlockBody,
    ) -> Result<(), StoreError> {
        self.write::<Bodies>(block_hash.encode_to_vec(), block_body.encode_to_vec())
            .await
    }

    async fn add_blocks(&self, blocks: Vec<Block>) -> Result<(), StoreError> {
        let db = self.env.clone();

        tokio::task::spawn_blocking(move || -> Result<(), StoreError> {
            let tx = db.tx_mut()?;
            for block in blocks {
                let number = block.header.number;
                let hash = block.hash();
                for (index, transaction) in block.body.transactions.iter().enumerate() {
                    tx.put::<TransactionLocations>(
                        transaction.compute_hash().0.encode_to_vec(),
                        (number, hash, index as u64).encode_to_vec(),
                    )?;
                }

                tx.put::<Bodies>(hash.encode_to_vec(), block.body.encode_to_vec())?;
                tx.put::<Headers>(hash.encode_to_vec(), block.header.encode_to_vec())?;
                tx.put::<BlockNumbers>(hash.encode_to_vec(), number)?;
            }
            tx.commit()?;
            Ok(())
        })
        .await??;
        Ok(())
    }

    async fn mark_chain_as_canonical(&self, blocks: &[Block]) -> Result<(), StoreError> {
        let key_values: Vec<_> = blocks
            .iter()
            .map(|e| (e.header.number, e.hash().encode_to_vec()))
            .collect();
        let db = self.env.clone();

        tokio::task::spawn_blocking(move || -> Result<(), StoreError> {
            let tx = db.tx_mut()?;
            for (k, v) in key_values {
                tx.put::<CanonicalBlockHashes>(k, v)?;
            }
            tx.commit()?;
            Ok(())
        })
        .await??;
        Ok(())
    }

    async fn get_block_body(
        &self,
        block_number: BlockNumber,
    ) -> Result<Option<BlockBody>, StoreError> {
        let Some(hash) = self.read::<CanonicalBlockHashes>(block_number).await? else {
            return Ok(None);
        };

        let Some(encoded_body) = self.read::<Bodies>(hash).await? else {
            return Ok(None);
        };
        let decoded = BlockBody::decode(&encoded_body)?;
        Ok(Some(decoded))
    }

    async fn get_block_bodies(
        &self,
        from: BlockNumber,
        to: BlockNumber,
    ) -> Result<Vec<BlockBody>, StoreError> {
        let mut encoded_bodies = vec![];
        {
            let tx = self.env.tx()?;
            let mut hashes_cursor = tx.cursor_read::<CanonicalBlockHashes>()?;
            let iterator = hashes_cursor.walk_range(from..=to)?;
            for next in iterator {
                let Ok((_, encoded_hash)) = next else {
                    break;
                };
                let Ok(Some(encoded_body)) = tx.get::<Bodies>(encoded_hash) else {
                    break;
                };
                encoded_bodies.push(encoded_body)
            }
        }
        let bodies: Result<Vec<BlockBody>, _> = encoded_bodies
            .into_iter()
            .map(|eb| RLPDecode::decode(&eb))
            .collect();
        Ok(bodies?)
    }

    async fn get_block_bodies_by_hash(
        &self,
        hashes: Vec<BlockHash>,
    ) -> Result<Vec<BlockBody>, StoreError> {
        let mut encoded_result = vec![];
        {
            let encoded_hashes: Vec<Vec<u8>> = hashes
                .into_iter()
                .map(|h| RLPEncode::encode_to_vec(&h))
                .collect();
            let tx = self.env.tx()?;
            for eh in encoded_hashes {
                let Some(encoded_body) = tx.get::<Bodies>(eh)? else {
                    break;
                };
                encoded_result.push(encoded_body);
            }
        }
        let bodies: Result<Vec<_>, _> = encoded_result
            .into_iter()
            .map(|encoded| RLPDecode::decode(&encoded))
            .collect();
        Ok(bodies?)
    }

    async fn take_storage_heal_paths(
        &self,
        limit: usize,
    ) -> Result<Vec<(H256, Vec<Nibbles>)>, StoreError> {
        let tx = self.env.tx_mut()?;
        let res: Result<Vec<(H256, Vec<Nibbles>)>, StoreError> = tx
            .cursor_read::<StorageHealPaths>()?
            .walk(None)?
            .take(limit)
            .map(|fetched| -> Result<(H256, Vec<Nibbles>), StoreError> {
                let (encoded_h, encoded_nibbles) = fetched?;
                let hash = H256::decode(&encoded_h)?;
                let nibbles = <Vec<Nibbles>>::decode(&encoded_nibbles)?;
                Ok((hash, nibbles))
            })
            .collect();
        let res = res?;
        for (hash, _) in res.iter() {
            tx.delete::<StorageHealPaths>(hash.encode_to_vec(), None)?;
        }
        Ok(res)
    }

    async fn get_block_body_by_hash(
        &self,
        block_hash: BlockHash,
    ) -> Result<Option<BlockBody>, StoreError> {
        let encoded_hash = block_hash.encode_to_vec();
        let tx = self.env.tx()?;
        let Some(encoded_body) = tx.get::<Bodies>(encoded_hash)? else {
            return Ok(None);
        };
        let decoded = BlockBody::decode(&encoded_body)?;
        Ok(Some(decoded))
    }

    fn get_block_header_by_hash(
        &self,
        block_hash: BlockHash,
    ) -> Result<Option<BlockHeader>, StoreError> {
        let tx = self.env.tx()?;
        let header = tx
            .get::<Headers>(block_hash.encode_to_vec())?
            .map(|header| BlockHeader::decode(&header[..]))
            .transpose()?;
        Ok(header)
    }

    async fn add_block_number(
        &self,
        block_hash: BlockHash,
        block_number: BlockNumber,
    ) -> Result<(), StoreError> {
        let db = self.env.clone();

        tokio::task::spawn_blocking(move || -> Result<(), StoreError> {
            let tx = db.tx_mut()?;
            let encoded_hash = block_hash.encode_to_vec();
            tx.put::<BlockNumbers>(encoded_hash, block_number)?;
            tx.commit()?;
            Ok(())
        })
        .await??;
        Ok(())
    }

    async fn get_block_number(
        &self,
        block_hash: BlockHash,
    ) -> Result<Option<BlockNumber>, StoreError> {
        self.read::<BlockNumbers>(block_hash.encode_to_vec()).await
    }

    async fn add_account_code(&self, code_hash: H256, code: Bytes) -> Result<(), StoreError> {
        let key: B256 = code_hash.0.into();

        let db = self.env.clone();
        tokio::task::spawn_blocking(move || -> Result<(), StoreError> {
            let tx = db.tx_mut().expect("could not start tx for account code");
            tx.put::<Bytecodes>(key, code.to_vec())?;
            tx.commit()?;
            Ok(())
        })
        .await??;
        Ok(())
    }

    fn get_account_code(&self, code_hash: H256) -> Result<Option<Bytes>, StoreError> {
        let key: B256 = code_hash.0.into();
        let Ok(code) = self.read_sync::<Bytecodes>(key) else {
            panic!("Failed to fetch bytecode from db")
        };
        Ok(code.map(|bytecode: Vec<u8>| -> Bytes { Bytes::from(bytecode) }))
    }

    async fn add_receipt(
        &self,
        block_hash: BlockHash,
        index: Index,
        receipt: Receipt,
    ) -> Result<(), StoreError> {
        let encoded = receipt.encode_to_vec();
        let key = (block_hash, index).encode_to_vec();
        self.write::<Receipts>(key, encoded).await?;
        Ok(())
    }

    async fn get_receipt(
        &self,
        block_number: u64,
        receipt_index: u64,
    ) -> Result<Option<Receipt>, StoreError> {
        let Some(key) = self.read::<CanonicalBlockHashes>(block_number).await? else {
            return Ok(None);
        };
        let decoded_hash: BlockHash = RLPDecode::decode(&key)?;
        let key_for_receipt = (decoded_hash, receipt_index).encode_to_vec();
        let encoded_receipt = self.read::<Receipts>(key_for_receipt).await?;
        Ok(encoded_receipt.map(|r| RLPDecode::decode(&r)).transpose()?)
    }

    async fn add_transaction_location(
        &self,
        transaction_hash: H256,
        block_number: BlockNumber,
        block_hash: BlockHash,
        index: Index,
    ) -> Result<(), StoreError> {
        let tx = self.env.tx_mut()?;
        tokio::task::spawn_blocking(move || -> Result<(), StoreError> {
            let value = (block_number, block_hash, index).encode_to_vec();
            let key = transaction_hash.encode_to_vec();
            tx.put::<TransactionLocations>(key, value)?;
            tx.commit()?;
            Ok(())
        })
        .await??;

        Ok(())
    }

    async fn get_transaction_location(
        &self,
        transaction_hash: H256,
    ) -> Result<Option<(BlockNumber, BlockHash, Index)>, StoreError> {
        let db = self.env.clone();

        let res = tokio::task::spawn_blocking(
            move || -> Result<Option<(BlockNumber, BlockHash, Index)>, StoreError> {
                let key = transaction_hash.encode_to_vec();
                let tx = db.tx()?;
                let mut cursor = tx.cursor_dup_read::<TransactionLocations>()?;
                if cursor.seek_exact(key.clone())?.is_none() {
                    return Ok(None);
                }
                let walker = cursor.walk(Some(key))?;
                for elem in walker {
                    let (_, encoded_tuple) = elem?;
                    let (bn, bh, indx) = <(BlockNumber, BlockHash, Index)>::decode(&encoded_tuple)?;
                    if let Some(block_hash) = tx.get::<CanonicalBlockHashes>(bn)? {
                        let block_hash: BlockHash = RLPDecode::decode(&block_hash)?;
                        if block_hash == bh {
                            return Ok(Some((bn, bh, indx)));
                        }
                    }
                }
                Ok(None)
            },
        )
        .await??;

        Ok(res)
    }

    async fn set_chain_config(&self, chain_config: &ChainConfig) -> Result<(), StoreError> {
        self.write::<ChainData>(
            ChainDataIndex::ChainConfig as u8,
            serde_json::to_string(chain_config)
                .map_err(|_| StoreError::Custom("failed to serialize chain config".to_string()))?
                .into_bytes(),
        )
        .await?;
        Ok(())
    }

    fn get_chain_config(&self) -> Result<ChainConfig, StoreError> {
        match self.read_sync::<ChainData>(ChainDataIndex::ChainConfig as u8)? {
            None => Err(StoreError::Custom("Chain config not found".to_string())),
            Some(bytes) => {
                let json = String::from_utf8(bytes).map_err(|_| StoreError::DecodeError)?;
                let chain_config: ChainConfig =
                    serde_json::from_str(&json).map_err(|_| StoreError::DecodeError)?;
                Ok(chain_config)
            }
        }
    }

    async fn update_earliest_block_number(
        &self,
        block_number: BlockNumber,
    ) -> Result<(), StoreError> {
        self.write::<ChainData>(
            ChainDataIndex::EarliestBlockNumber as u8,
            block_number.encode_to_vec(),
        )
        .await?;
        Ok(())
    }

    async fn get_earliest_block_number(&self) -> Result<Option<BlockNumber>, StoreError> {
        let res = self
            .read::<ChainData>(ChainDataIndex::EarliestBlockNumber as u8)
            .await?
            .map(|r| RLPDecode::decode(r.as_ref()))
            .transpose()?;
        Ok(res)
    }

    async fn update_finalized_block_number(
        &self,
        block_number: BlockNumber,
    ) -> Result<(), StoreError> {
        self.write::<ChainData>(
            ChainDataIndex::FinalizedBlockNumber as u8,
            block_number.encode_to_vec(),
        )
        .await?;
        Ok(())
    }

    async fn get_finalized_block_number(&self) -> Result<Option<BlockNumber>, StoreError> {
        Ok(self
            .read::<ChainData>(ChainDataIndex::FinalizedBlockNumber as u8)
            .await?
            .map(|ref bn| BlockNumber::decode(bn))
            .transpose()?)
    }

    async fn update_safe_block_number(&self, block_number: BlockNumber) -> Result<(), StoreError> {
        self.write::<ChainData>(
            ChainDataIndex::SafeBlockNumber as u8,
            block_number.encode_to_vec(),
        )
        .await?;
        Ok(())
    }

    async fn get_safe_block_number(&self) -> Result<Option<BlockNumber>, StoreError> {
        Ok(self
            .read::<ChainData>(ChainDataIndex::SafeBlockNumber as u8)
            .await?
            .map(|ref num| BlockNumber::decode(num))
            .transpose()?)
    }

    async fn update_latest_block_number(
        &self,
        block_number: BlockNumber,
    ) -> Result<(), StoreError> {
        self.write::<ChainData>(
            ChainDataIndex::LatestBlockNumber as u8,
            block_number.encode_to_vec(),
        )
        .await?;
        Ok(())
    }

    async fn get_latest_block_number(&self) -> Result<Option<BlockNumber>, StoreError> {
        let res = self
            .read::<ChainData>(ChainDataIndex::LatestBlockNumber as u8)
            .await?
            .map(|r| RLPDecode::decode(r.as_ref()))
            .transpose()?;
        Ok(res)
    }

    async fn update_pending_block_number(
        &self,
        block_number: BlockNumber,
    ) -> Result<(), StoreError> {
        let encoded = block_number.encode_to_vec();
        self.write::<ChainData>(ChainDataIndex::PendingBlockNumber as u8, encoded)
            .await?;
        Ok(())
    }

    async fn get_pending_block_number(&self) -> Result<Option<BlockNumber>, StoreError> {
        let Some(res) = self
            .read::<ChainData>(ChainDataIndex::PendingBlockNumber as u8)
            .await?
        else {
            return Ok(None);
        };
        let decoded = BlockNumber::decode(&res)?;
        Ok(Some(decoded))
    }

    fn open_storage_trie(
        &self,
        hashed_address: H256,
        storage_root: H256,
    ) -> Result<Trie, StoreError> {
        Ok(Trie::open(
            Arc::new(MDBXTrieWithFixedKey::new(self.env.clone(), hashed_address)),
            storage_root,
        ))
    }

    fn open_state_trie(&self, state_root: H256) -> Result<Trie, StoreError> {
        Ok(Trie::open(self.state_trie.clone(), state_root))
    }

    async fn set_canonical_block(
        &self,
        number: BlockNumber,
        hash: BlockHash,
    ) -> Result<(), StoreError> {
        let tx = self.env.tx_mut()?;
        tx.put::<CanonicalBlockHashes>(number, hash.encode_to_vec())?;
        tx.commit()?;
        Ok(())
    }

    async fn get_canonical_block_hash(
        &self,
        number: BlockNumber,
    ) -> Result<Option<BlockHash>, StoreError> {
        let bytes = self.read::<CanonicalBlockHashes>(number).await?;

        match bytes {
            Some(bytes) => {
                let hash: BlockHash = RLPDecode::decode(bytes.as_ref())?;
                Ok(Some(hash))
            }
            None => Ok(None),
        }
    }

    async fn add_payload(&self, payload_id: u64, block: Block) -> Result<(), StoreError> {
        self.write::<Payloads>(payload_id, PayloadBundle::from_block(block).encode_to_vec())
            .await?;
        Ok(())
    }

    async fn get_payload(&self, payload_id: u64) -> Result<Option<PayloadBundle>, StoreError> {
        let res = self.read::<Payloads>(payload_id).await?;
        match res {
            Some(encoded) => Ok(Some(PayloadBundle::decode(&encoded[..])?)),
            None => Ok(None),
        }
    }

    async fn update_payload(
        &self,
        payload_id: u64,
        payload: PayloadBundle,
    ) -> Result<(), StoreError> {
        self.write::<Payloads>(payload_id, payload.encode_to_vec())
            .await?;
        Ok(())
    }

    async fn get_transaction_by_hash(
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

    async fn get_transaction_by_location(
        &self,
        block_hash: H256,
        index: u64,
    ) -> Result<Option<Transaction>, StoreError> {
        let block_body = match self.get_block_body_by_hash(block_hash).await? {
            Some(body) => body,
            None => return Ok(None),
        };
        Ok(index
            .try_into()
            .ok()
            .and_then(|index: usize| block_body.transactions.get(index).cloned()))
    }

    async fn get_block_by_hash(&self, block_hash: BlockHash) -> Result<Option<Block>, StoreError> {
        let body = self.read::<Bodies>(block_hash.encode_to_vec()).await?;
        let header = self.read::<Headers>(block_hash.encode_to_vec()).await?;
        match (body, header) {
            (Some(body), Some(header)) => {
                let body: BlockBody = RLPDecode::decode(body.as_ref())?;
                let header: BlockHeader = RLPDecode::decode(header.as_ref())?;
                let block = Block::new(header, body);
                Ok(Some(block))
            }
            _ => Ok(None),
        }
    }

    async fn unset_canonical_block(&self, number: BlockNumber) -> Result<(), StoreError> {
        let db = self.env.clone();
        tokio::task::spawn_blocking(move || -> Result<(), StoreError> {
            let tx = db.tx_mut()?;
            tx.delete::<CanonicalBlockHashes>(number, None)?;
            Ok(())
        })
        .await??;

        Ok(())
    }

    async fn add_pending_block(&self, block: Block) -> Result<(), StoreError> {
        self.write::<PendingBlocks>(block.header.hash().encode_to_vec(), block.encode_to_vec())
            .await?;
        Ok(())
    }

    async fn get_pending_block(&self, block_hash: BlockHash) -> Result<Option<Block>, StoreError> {
        let encoded_hash = block_hash.encode_to_vec();
        let Some(encoded_block) = self.read::<PendingBlocks>(encoded_hash).await? else {
            return Ok(None);
        };
        Ok(Some(Block::decode(&encoded_block)?))
    }

    async fn add_transaction_locations(
        &self,
        locations: Vec<(H256, BlockNumber, BlockHash, Index)>,
    ) -> Result<(), StoreError> {
        let key_values = locations
            .into_iter()
            .map(|(tx_hash, block_number, block_hash, index)| {
                (
                    tx_hash.encode_to_vec(),
                    (block_number, block_hash, index).encode_to_vec(),
                )
            })
            .collect::<Vec<_>>();
        self.write_batch::<TransactionLocations>(key_values).await?;
        Ok(())
    }

    async fn add_receipts(
        &self,
        block_hash: BlockHash,
        receipts: Vec<Receipt>,
    ) -> Result<(), StoreError> {
        //  TODO: ADD indexed chunk in order to use a cursor

        // Using write batch here crashes libmdbx at some point due to a max size limit.
        let db = self.env.clone();

        tokio::task::spawn_blocking(move || -> Result<(), StoreError> {
            let tx = db.tx_mut()?;
            for (index, receipt) in receipts.into_iter().enumerate() {
                let receipt_bytes = receipt.encode_to_vec();
                let receipt_db_key = (block_hash, index).encode_to_vec();
                tx.put::<Receipts>(receipt_db_key, receipt_bytes)?;
            }

            tx.commit()?;
            Ok(())
        })
        .await??;

        Ok(())
    }

    fn get_receipts_for_block(&self, block_hash: &BlockHash) -> Result<Vec<Receipt>, StoreError> {
        let mut encoded_receipts: Vec<Vec<u8>> = vec![];
        {
            let mut receipt_index = 0_u64;
            let tx = self.env.tx()?;
            let mut cursor = tx.cursor_read::<Receipts>()?;
            while let Some((_, encoded_receipt)) =
                cursor.seek((*block_hash, receipt_index).encode_to_vec())?
            {
                encoded_receipts.push(encoded_receipt);
                receipt_index += 1;
            }
        }
        Ok(encoded_receipts
            .into_iter()
            .map(|r| RLPDecode::decode(&r))
            .collect::<Result<_, _>>()?)
    }

    async fn set_header_download_checkpoint(
        &self,
        block_hash: BlockHash,
    ) -> Result<(), StoreError> {
        self.write::<SnapState>(
            SnapStateIndex::HeaderDownloadCheckpoint as u8,
            block_hash.encode_to_vec(),
        )
        .await?;
        Ok(())
    }

    async fn get_header_download_checkpoint(&self) -> Result<Option<BlockHash>, StoreError> {
        Ok(self
            .read::<SnapState>(SnapStateIndex::HeaderDownloadCheckpoint as u8)
            .await?
            .map(|h| BlockHash::decode(h.as_ref()))
            .transpose()?)
    }

    async fn set_state_trie_key_checkpoint(
        &self,
        last_keys: [H256; STATE_TRIE_SEGMENTS],
    ) -> Result<(), StoreError> {
        let encoded = last_keys.to_vec().encode_to_vec();
        self.write::<SnapState>(SnapStateIndex::StateTrieKeyCheckpoint as u8, encoded)
            .await?;
        Ok(())
    }

    async fn get_state_trie_key_checkpoint(
        &self,
    ) -> Result<Option<[H256; STATE_TRIE_SEGMENTS]>, StoreError> {
        self.read::<SnapState>(SnapStateIndex::StateTrieKeyCheckpoint as u8)
            .await?
            .map(|ref c| {
                <Vec<H256>>::decode(c)?
                    .try_into()
                    .map_err(|_| RLPDecodeError::InvalidLength)
            })
            .transpose()
            .map_err(StoreError::RLPDecode)
    }

    async fn set_storage_heal_paths(
        &self,
        accounts: Vec<(H256, Vec<Nibbles>)>,
    ) -> Result<(), StoreError> {
        let encoded: Vec<_> = accounts
            .into_iter()
            .map(|(hash, nibbles)| (hash.encode_to_vec(), nibbles.encode_to_vec()))
            .collect();
        let tx = self.env.tx_mut()?;
        for (k, v) in encoded {
            tx.put::<StorageHealPaths>(k, v)?;
        }
        Ok(())
    }

    async fn set_state_heal_paths(&self, paths: Vec<Nibbles>) -> Result<(), StoreError> {
        let encoded = paths.encode_to_vec();
        let tx = self.env.tx_mut()?;
        tx.put::<SnapState>(SnapStateIndex::StateHealPaths as u8, encoded)?;
        tx.commit()?;
        Ok(())
    }

    async fn get_state_heal_paths(&self) -> Result<Option<Vec<Nibbles>>, StoreError> {
        let tx = self.env.tx()?;
        let Some(res) = tx.get::<SnapState>(SnapStateIndex::StateHealPaths as u8)? else {
            return Ok(None);
        };
        let decoded = <Vec<Nibbles>>::decode(&res[..])?;
        Ok(Some(decoded))
    }

    async fn clear_snap_state(&self) -> Result<(), StoreError> {
        let tx = self.env.tx_mut()?;
        tx.clear::<SnapState>()?;
        tx.commit()?;
        Ok(())
    }

    async fn write_snapshot_account_batch(
        &self,
        account_hashes: Vec<H256>,
        account_states: Vec<AccountState>,
    ) -> Result<(), StoreError> {
        let key_values: Vec<_> = account_hashes
            .into_iter()
            .map(|h| h.encode_to_vec())
            .zip(account_states.into_iter().map(|a| a.encode_to_vec()))
            .collect();
        self.write_batch::<StateSnapShot>(key_values).await?;
        Ok(())
    }

    async fn write_snapshot_storage_batch(
        &self,
        account_hash: H256,
        storage_keys: Vec<H256>,
        storage_values: Vec<U256>,
    ) -> Result<(), StoreError> {
        let db = self.env.clone();

        tokio::task::spawn_blocking(move || -> Result<(), StoreError> {
            let encoded_hash = account_hash.encode_to_vec();
            let encoded_values = storage_keys
                .into_iter()
                .zip(storage_values.into_iter())
                .map(|v| v.encode_to_vec())
                .collect::<Vec<_>>();
            let tx = db.tx_mut()?;
            let mut cursor = tx.cursor_dup_write::<StateSnapShot>()?;
            cursor.seek_exact(encoded_hash.clone())?;
            for v in encoded_values {
                cursor.upsert(encoded_hash.clone(), v)?;
            }
            Ok(())
        })
        .await??;

        Ok(())
    }

    async fn write_snapshot_storage_batches(
        &self,
        account_hashes: Vec<H256>,
        storage_keys: Vec<Vec<H256>>,
        storage_values: Vec<Vec<U256>>,
    ) -> Result<(), StoreError> {
        let db = self.env.clone();
        tokio::task::spawn_blocking(move || -> Result<(), StoreError> {
            // Pre-encode all data before starting DB interaction
            let pre_encoded: Vec<(Vec<u8>, Vec<Vec<u8>>)> = account_hashes
                .into_iter()
                .zip(storage_keys.into_iter().zip(storage_values.into_iter()))
                .map(|(account_hash, (keys, values))| {
                    let encoded_hash = account_hash.encode_to_vec();
                    let encoded_pairs = keys
                        .into_iter()
                        .zip(values.into_iter())
                        .map(|(k, v)| (k, v).encode_to_vec())
                        .collect();
                    (encoded_hash, encoded_pairs)
                })
                .collect();

            // Now perform all DB operations in one quick sequence
            let tx = db.tx_mut()?;
            let mut cursor = tx.cursor_dup_write::<StorageSnapshot>()?;

            for (encoded_hash, encoded_pairs) in pre_encoded {
                cursor.seek_exact(encoded_hash.clone())?;
                for pair in encoded_pairs {
                    cursor.upsert(encoded_hash.clone(), pair)?;
                }
            }
            Ok(())
        })
        .await??;

        Ok(())
    }

    async fn set_state_trie_rebuild_checkpoint(
        &self,
        checkpoint: (H256, [H256; STATE_TRIE_SEGMENTS]),
    ) -> Result<(), StoreError> {
        let encoded = (checkpoint.0, checkpoint.1.to_vec()).encode_to_vec();
        self.write::<SnapState>(SnapStateIndex::StateTrieRebuildCheckpoint as u8, encoded)
            .await?;
        Ok(())
    }

    async fn get_state_trie_rebuild_checkpoint(
        &self,
    ) -> Result<Option<(H256, [H256; STATE_TRIE_SEGMENTS])>, StoreError> {
        let Some(encoded) = self
            .read::<SnapState>(SnapStateIndex::StateTrieRebuildCheckpoint as u8)
            .await?
        else {
            return Ok(None);
        };
        let (root, checkpoints): (H256, Vec<H256>) = RLPDecode::decode(&encoded)?;
        Ok(Some((
            root,
            checkpoints.try_into().map_err(|_| {
                StoreError::Custom(
                    "failed to transform checkpoint slice into fixed size array".to_string(),
                )
            })?,
        )))
    }

    async fn set_storage_trie_rebuild_pending(
        &self,
        pending: Vec<(H256, H256)>,
    ) -> Result<(), StoreError> {
        self.write::<SnapState>(
            SnapStateIndex::StorageTrieRebuildPending as u8,
            pending.encode_to_vec(),
        )
        .await?;
        Ok(())
    }

    async fn get_storage_trie_rebuild_pending(
        &self,
    ) -> Result<Option<Vec<(H256, H256)>>, StoreError> {
        let Some(encoded) = self
            .read::<SnapState>(SnapStateIndex::StorageTrieRebuildPending as u8)
            .await?
        else {
            return Ok(None);
        };
        let decoded: Vec<(H256, H256)> = RLPDecode::decode(&encoded)?;
        Ok(Some(decoded))
    }

    async fn clear_snapshot(&self) -> Result<(), StoreError> {
        let tx = self.env.tx_mut()?;
        tx.clear::<StateSnapShot>()?;
        tx.commit()?;
        Ok(())
    }

    fn read_account_snapshot(&self, start: H256) -> Result<Vec<(H256, AccountState)>, StoreError> {
        let key = start.encode_to_vec();
        let mut results = vec![];
        {
            let tx = self.env.tx()?;
            let mut cursor = tx.cursor_read::<StateSnapShot>()?;
            cursor.seek_exact(key)?;
            let mut readings = 0;
            while readings < MAX_SNAPSHOT_READS {
                let Some((encoded_key, encoded_value)) = cursor.next_dup()? else {
                    break;
                };
                results.push((encoded_key, encoded_value));
                readings += 1;
            }
        }
        let results = results
            .into_iter()
            .map(|(ref encoded_k, ref encoded_v)| -> Result<_, StoreError> {
                Ok((H256::decode(encoded_k)?, AccountState::decode(encoded_v)?))
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(results)
    }

    async fn read_storage_snapshot(
        &self,
        _account_hash: H256,
        start: H256,
    ) -> Result<Vec<(H256, U256)>, StoreError> {
        let key = start.encode_to_vec();
        let mut results = vec![];
        {
            let tx = self.env.tx()?;
            let mut cursor = tx.cursor_read::<StateSnapShot>()?;
            cursor.seek_exact(key)?;
            let mut readings = 0;
            while readings < MAX_SNAPSHOT_READS {
                let Some((encoded_key, encoded_value)) = cursor.next_dup()? else {
                    break;
                };
                results.push((encoded_key, encoded_value));
                readings += 1;
            }
        }
        let results: Vec<(H256, U256)> = results
            .into_iter()
            .map(|(ref encoded_k, ref encoded_v)| -> Result<_, StoreError> {
                Ok((H256::decode(encoded_k)?, U256::decode(encoded_v)?))
            })
            .collect::<Result<_, _>>()?;
        Ok(results)
    }

    async fn set_latest_valid_ancestor(
        &self,
        bad_block: BlockHash,
        latest_valid: BlockHash,
    ) -> Result<(), StoreError> {
        let key = bad_block.encode_to_vec();
        let encoded_value = latest_valid.encode_to_vec();
        self.write::<InvalidAncestor>(key, encoded_value).await?;
        Ok(())
    }

    async fn get_latest_valid_ancestor(
        &self,
        block: BlockHash,
    ) -> Result<Option<BlockHash>, StoreError> {
        let key = block.encode_to_vec();
        let Some(result) = self.read::<InvalidAncestor>(key).await? else {
            return Ok(None);
        };
        Ok(Some(BlockHash::decode(&result)?))
    }

    fn get_block_number_sync(
        &self,
        block_hash: BlockHash,
    ) -> Result<Option<BlockNumber>, StoreError> {
        let encoded_key = block_hash.encode_to_vec();
        self.read_sync::<BlockNumbers>(encoded_key)
    }

    fn get_canonical_block_hash_sync(
        &self,
        block_number: BlockNumber,
    ) -> Result<Option<BlockHash>, StoreError> {
        let bytes = self.read_sync::<CanonicalBlockHashes>(block_number)?;

        match bytes {
            Some(bytes) => {
                let hash: BlockHash = RLPDecode::decode(bytes.as_ref())?;
                Ok(Some(hash))
            }
            None => Ok(None),
        }
    }
}
