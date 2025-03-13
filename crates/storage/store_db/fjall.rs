use crate::{
    api::StoreEngine,
    error::StoreError,
    rlp::{
        AccountCodeHashRLP, AccountCodeRLP, AccountHashRLP, AccountStateRLP, BlockBodyRLP,
        BlockHashRLP, BlockHeaderRLP, BlockRLP, PayloadBundleRLP, ReceiptRLP, Rlp,
        TransactionHashRLP, TupleRLP,
    },
    trie_db::{
        fjall::{create_fjall_trie, FjallTrie},
        fjall_dupsort::FjallDupsortTrieDB,
    },
    utils::{ChainDataIndex, SnapStateIndex},
};
use ethrex_common::{
    types::{
        payload::PayloadBundle, BlockBody, BlockHash, BlockHeader, BlockNumber, ChainConfig, Index,
    },
    H256,
};
use ethrex_rlp::{decode::RLPDecode, encode::RLPEncode};
use ethrex_trie::{Trie, TrieDB};
use fjall::{Config, Keyspace, PartitionCreateOptions, PartitionHandle, PersistMode};
use std::{
    collections::HashMap,
    fmt::{Debug, Formatter},
    path::Path,
    sync::{Arc, Mutex, RwLock},
};

impl Clone for Fjall {
    fn clone(&self) -> Self {
        Self {
            keyspace: self.keyspace.clone(),
            partitions: self.partitions.clone(),
        }
    }
}

impl Debug for Fjall {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result<(), std::fmt::Error> {
        f.write_str("FJALL DATABASE");
        Ok(())
    }
}

pub struct Fjall {
    partitions: Arc<RwLock<HashMap<String, PartitionHandle>>>,
    // DO NOT REMOVE
    keyspace: Arc<Mutex<Keyspace>>,
}

pub fn init<P: AsRef<Path>>(folder: P) -> Fjall {
    let keyspace = Config::new(folder)
        .max_write_buffer_size(1_u64 * (10_u64.pow(9)))
        // .fsync_ms(Some(500)
        .max_open_files(16000)
        .open()
        .unwrap();
    let mut partitions = Default::default();

    // Initialize all partitions
    init_partition::<CanonicalBlockHashes>(&keyspace, &mut partitions).unwrap();
    init_partition::<BlockNumbers>(&keyspace, &mut partitions).unwrap();
    init_partition::<Headers>(&keyspace, &mut partitions).unwrap();
    init_partition::<Bodies>(&keyspace, &mut partitions).unwrap();
    init_partition::<AccountCodes>(&keyspace, &mut partitions).unwrap();
    init_partition::<Receipts>(&keyspace, &mut partitions).unwrap();
    init_partition::<StorageTriesNodes>(&keyspace, &mut partitions).unwrap();
    init_partition::<TransactionLocations>(&keyspace, &mut partitions).unwrap();
    init_partition::<ChainData>(&keyspace, &mut partitions).unwrap();
    init_partition::<SnapState>(&keyspace, &mut partitions).unwrap();
    init_partition::<StateTrieNodes>(&keyspace, &mut partitions).unwrap();
    init_partition::<Payloads>(&keyspace, &mut partitions).unwrap();
    init_partition::<PendingBlocks>(&keyspace, &mut partitions).unwrap();
    init_partition::<StateSnapShot>(&keyspace, &mut partitions).unwrap();
    init_partition::<StorageSnapshot>(&keyspace, &mut partitions).unwrap();

    Fjall {
        keyspace: Arc::new(Mutex::new(keyspace)),
        partitions: Arc::new(RwLock::new(partitions)),
    }
}

// Helper method to initialize a single partition
fn init_partition<T: FjallStorable>(
    keyspace: &Keyspace,
    partitions: &mut HashMap<String, PartitionHandle>,
) -> Result<(), StoreError> {
    let table_name = T::table_name();
    let partition = keyspace
        .open_partition(
            &table_name,
            PartitionCreateOptions::default().max_memtable_size(64 * 1024 * 1024),
        )
        .unwrap();
    partitions.insert(table_name.to_owned(), partition);
    Ok(())
}

impl StoreEngine for Fjall {
    fn add_block_header(
        &self,
        block_hash: BlockHash,
        block_header: ethrex_common::types::BlockHeader,
    ) -> Result<(), StoreError> {
        self.partitions
            .read()
            .unwrap()
            .get(Headers::table_name())
            .unwrap()
            .insert(block_hash.to_fixed_bytes(), block_header.encode_to_vec())
            .unwrap();
        Ok(())
    }

    fn add_block_headers(
        &self,
        block_hashes: Vec<BlockHash>,
        block_headers: Vec<ethrex_common::types::BlockHeader>,
    ) -> Result<(), StoreError> {
        let partition = self.partitions.read().unwrap();
        // .unwrap()
        // .get(Headers::table_name())
        // .unwrap();

        let partition = partition.get(Headers::table_name()).unwrap();
        for (hash, header) in block_hashes.into_iter().zip(block_headers.into_iter()) {
            partition
                .insert(hash.to_fixed_bytes(), header.encode_to_vec())
                .unwrap();
        }
        Ok(())
    }

    fn get_block_header(
        &self,
        block_number: BlockNumber,
    ) -> Result<Option<ethrex_common::types::BlockHeader>, StoreError> {
        // let hash = self.read(&block_number);
        let Ok(Some(hash)) = self
            .partitions
            .read()
            .unwrap()
            .get(CanonicalBlockHashes::table_name())
            .unwrap()
            .get(block_number.to_be_bytes())
        else {
            return Ok(None);
        };

        let hash: BlockHash = BlockHash::from_slice(&hash);

        let Some(raw_header) = self
            .partitions
            .read()
            .unwrap()
            .get(Headers::table_name())
            .unwrap()
            .get(hash)
            .unwrap()
        else {
            return Ok(None);
        };
        let header: BlockHeader = BlockHeader::decode(&raw_header).unwrap();
        Ok(Some(header))
    }

    fn add_block_body(
        &self,
        block_hash: BlockHash,
        block_body: ethrex_common::types::BlockBody,
    ) -> Result<(), StoreError> {
        let key = block_hash.to_fixed_bytes();
        let value = block_body.encode_to_vec();
        self.partitions
            .read()
            .unwrap()
            .get(Bodies::table_name())
            .unwrap()
            .insert(key, value)
            .unwrap();
        Ok(())
    }

    fn get_block_body(
        &self,
        block_number: BlockNumber,
    ) -> Result<Option<ethrex_common::types::BlockBody>, StoreError> {
        let Some(raw_body) = self
            .partitions
            .read()
            .unwrap()
            .get(Bodies::table_name())
            .unwrap()
            .get(block_number.to_be_bytes())
            .unwrap()
        else {
            return Ok(None);
        };
        let (block_body, _) = BlockBody::decode_unfinished(&raw_body).unwrap();
        Ok(Some(block_body))
    }

    // Implementation for get_block_body_by_hash
    fn get_block_body_by_hash(
        &self,
        block_hash: BlockHash,
    ) -> Result<Option<ethrex_common::types::BlockBody>, StoreError> {
        let Some(raw_body) = self
            .partitions
            .read()
            .unwrap()
            .get(Bodies::table_name())
            .unwrap()
            .get(block_hash.to_fixed_bytes())
            .unwrap()
        else {
            return Ok(None);
        };
        let (block_body, _) = BlockBody::decode_unfinished(&raw_body).unwrap();
        Ok(Some(block_body))
    }

    // Implementation for get_block_header_by_hash
    fn get_block_header_by_hash(
        &self,
        block_hash: BlockHash,
    ) -> Result<Option<ethrex_common::types::BlockHeader>, StoreError> {
        let Some(raw_header) = self
            .partitions
            .read()
            .unwrap()
            .get(Headers::table_name())
            .unwrap()
            .get(block_hash.to_fixed_bytes())
            .unwrap()
        else {
            return Ok(None);
        };
        let header: BlockHeader = BlockHeader::decode(&raw_header).unwrap();
        Ok(Some(header))
    }

    // Implementation for add_pending_block
    fn add_pending_block(&self, block: ethrex_common::types::Block) -> Result<(), StoreError> {
        self.partitions
            .read()
            .unwrap()
            .get(PendingBlocks::table_name())
            .unwrap()
            .insert(
                block.header.compute_block_hash().to_fixed_bytes(),
                block.encode_to_vec(),
            )
            .unwrap();
        Ok(())
    }

    // Implementation for get_pending_block
    fn get_pending_block(
        &self,
        block_hash: BlockHash,
    ) -> Result<Option<ethrex_common::types::Block>, StoreError> {
        let Some(raw_block) = self
            .partitions
            .read()
            .unwrap()
            .get(PendingBlocks::table_name())
            .unwrap()
            .get(block_hash.to_fixed_bytes())
            .unwrap()
        else {
            return Ok(None);
        };
        let block = ethrex_common::types::Block::decode(&raw_block).unwrap();
        Ok(Some(block))
    }

    // Implementation for add_block_number
    fn add_block_number(
        &self,
        block_hash: BlockHash,
        block_number: BlockNumber,
    ) -> Result<(), StoreError> {
        self.partitions
            .read()
            .unwrap()
            .get(BlockNumbers::table_name())
            .unwrap()
            .insert(block_hash.to_fixed_bytes(), block_number.to_be_bytes())
            .unwrap();
        Ok(())
    }

    // Implementation for get_block_number
    fn get_block_number(&self, block_hash: BlockHash) -> Result<Option<BlockNumber>, StoreError> {
        let Some(number_bytes) = self
            .partitions
            .read()
            .unwrap()
            .get(BlockNumbers::table_name())
            .unwrap()
            .get(block_hash.to_fixed_bytes())
            .unwrap()
        else {
            return Ok(None);
        };

        let block_number = BlockNumber::from_be_bytes((&number_bytes[..]).try_into().unwrap());
        Ok(Some(block_number))
    }

    // Implementation for add_transaction_location
    fn add_transaction_location(
        &self,
        transaction_hash: H256,
        block_number: BlockNumber,
        block_hash: BlockHash,
        index: Index,
    ) -> Result<(), StoreError> {
        let key = transaction_hash.to_fixed_bytes();
        let value = Rlp::from((block_number, block_hash, index));

        self.partitions
            .read()
            .unwrap()
            .get(TransactionLocations::table_name())
            .unwrap()
            .insert(key, value.bytes())
            .unwrap();
        Ok(())
    }

    // Implementation for add_transaction_locations
    fn add_transaction_locations(
        &self,
        locations: Vec<(H256, BlockNumber, BlockHash, Index)>,
    ) -> Result<(), StoreError> {
        let partitions = self.partitions.read().unwrap();
        let partition = partitions.get(TransactionLocations::table_name()).unwrap();

        for (tx_hash, block_number, block_hash, index) in locations {
            let key = tx_hash.to_fixed_bytes();
            let value = Rlp::from((block_number, block_hash, index));
            partition.insert(key, value.bytes()).unwrap();
        }
        Ok(())
    }

    // Implementation for get_transaction_location
    fn get_transaction_location(
        &self,
        transaction_hash: H256,
    ) -> Result<Option<(BlockNumber, BlockHash, Index)>, StoreError> {
        let Some(location_bytes) = self
            .partitions
            .read()
            .unwrap()
            .get(TransactionLocations::table_name())
            .unwrap()
            .get(transaction_hash.to_fixed_bytes())
            .unwrap()
        else {
            return Ok(None);
        };

        let (location, _) =
            <(BlockNumber, BlockHash, Index)>::decode_unfinished(&location_bytes).unwrap();
        Ok(Some(location))
    }

    // Implementation for add_receipt
    fn add_receipt(
        &self,
        block_hash: BlockHash,
        index: Index,
        receipt: ethrex_common::types::Receipt,
    ) -> Result<(), StoreError> {
        let key = TupleRLP::from((block_hash, index));
        let value = ReceiptRLP::from(receipt);

        self.partitions
            .read()
            .unwrap()
            .get(Receipts::table_name())
            .unwrap()
            .insert(key.bytes(), value.bytes())
            .unwrap();
        Ok(())
    }

    // Implementation for add_receipts
    fn add_receipts(
        &self,
        block_hash: BlockHash,
        receipts: Vec<ethrex_common::types::Receipt>,
    ) -> Result<(), StoreError> {
        let partitions = self.partitions.read().unwrap();
        let partition = partitions.get(Receipts::table_name()).unwrap();

        for (idx, receipt) in receipts.into_iter().enumerate() {
            let key = TupleRLP::from((block_hash, idx as u64));
            let value = ReceiptRLP::from(receipt);
            partition.insert(key.bytes(), value.bytes()).unwrap();
        }
        Ok(())
    }

    // Implementation for get_receipt
    fn get_receipt(
        &self,
        block_number: BlockNumber,
        index: Index,
    ) -> Result<Option<ethrex_common::types::Receipt>, StoreError> {
        // First get block hash for the block number
        let Some(hash_bytes) = self
            .partitions
            .read()
            .unwrap()
            .get(CanonicalBlockHashes::table_name())
            .unwrap()
            .get(block_number.to_be_bytes())
            .unwrap()
        else {
            return Ok(None);
        };

        let block_hash = BlockHash::from_slice(&hash_bytes);

        let key = TupleRLP::from((block_hash, index));
        let Some(receipt_bytes) = self
            .partitions
            .read()
            .unwrap()
            .get(Receipts::table_name())
            .unwrap()
            .get(key.bytes())
            .unwrap()
        else {
            return Ok(None);
        };

        let receipt = ethrex_common::types::Receipt::decode(&receipt_bytes).unwrap();
        Ok(Some(receipt))
    }

    // Implementation for add_account_code
    fn add_account_code(&self, code_hash: H256, code: bytes::Bytes) -> Result<(), StoreError> {
        let key = code_hash.to_fixed_bytes();
        let value = AccountCodeRLP::from(code);

        self.partitions
            .read()
            .unwrap()
            .get(AccountCodes::table_name())
            .unwrap()
            .insert(key, value.bytes())
            .unwrap();
        Ok(())
    }

    // Implementation for get_account_code
    fn get_account_code(&self, code_hash: H256) -> Result<Option<bytes::Bytes>, StoreError> {
        let Some(code_bytes) = self
            .partitions
            .read()
            .unwrap()
            .get(AccountCodes::table_name())
            .unwrap()
            .get(code_hash.to_fixed_bytes())
            .unwrap()
        else {
            return Ok(None);
        };

        let code = bytes::Bytes::decode(&code_bytes).unwrap();
        Ok(Some(code))
    }

    // Implementation for get_canonical_block_hash
    fn get_canonical_block_hash(
        &self,
        block_number: BlockNumber,
    ) -> Result<Option<BlockHash>, StoreError> {
        let Some(hash_bytes) = self
            .partitions
            .read()
            .unwrap()
            .get(CanonicalBlockHashes::table_name())
            .unwrap()
            .get(block_number.to_be_bytes())
            .unwrap()
        else {
            return Ok(None);
        };

        let block_hash = BlockHash::from_slice(&hash_bytes);
        Ok(Some(block_hash))
    }

    // Implementation for set_chain_config
    fn set_chain_config(
        &self,
        chain_config: &ethrex_common::types::ChainConfig,
    ) -> Result<(), StoreError> {
        self.partitions
            .read()
            .unwrap()
            .get(ChainData::table_name())
            .unwrap()
            .insert(
                "chain_config",
                serde_json::to_string(chain_config).unwrap().into_bytes(),
            )
            .unwrap();
        Ok(())
    }

    // Implementation for get_chain_config
    fn get_chain_config(&self) -> Result<ethrex_common::types::ChainConfig, StoreError> {
        let config_bytes = self
            .partitions
            .read()
            .unwrap()
            .get(ChainData::table_name())
            .unwrap()
            .get("chain_config")
            .unwrap()
            .unwrap();

        let json = String::from_utf8(config_bytes.to_vec()).unwrap();
        let chain_config: ChainConfig = serde_json::from_str(&json).unwrap();
        Ok(chain_config)
    }

    // Implementation for update_earliest_block_number
    fn update_earliest_block_number(&self, block_number: BlockNumber) -> Result<(), StoreError> {
        self.partitions
            .read()
            .unwrap()
            .get(ChainData::table_name())
            .unwrap()
            .insert("earliest_block_number", block_number.to_be_bytes().to_vec())
            .unwrap();
        Ok(())
    }

    // Implementation for get_earliest_block_number
    fn get_earliest_block_number(&self) -> Result<Option<BlockNumber>, StoreError> {
        let Some(number_bytes) = self
            .partitions
            .read()
            .unwrap()
            .get(ChainData::table_name())
            .unwrap()
            .get("earliest_block_number")
            .unwrap()
        else {
            return Ok(None);
        };

        let mut bytes = [0u8; 8];
        bytes.copy_from_slice(&number_bytes);
        Ok(Some(BlockNumber::from_be_bytes(bytes)))
    }

    // Implementation for update_finalized_block_number
    fn update_finalized_block_number(&self, block_number: BlockNumber) -> Result<(), StoreError> {
        self.partitions
            .read()
            .unwrap()
            .get(ChainData::table_name())
            .unwrap()
            .insert(
                "finalized_block_number",
                block_number.to_be_bytes().to_vec(),
            )
            .unwrap();
        Ok(())
    }

    // Implementation for get_finalized_block_number
    fn get_finalized_block_number(&self) -> Result<Option<BlockNumber>, StoreError> {
        let Some(number_bytes) = self
            .partitions
            .read()
            .unwrap()
            .get(ChainData::table_name())
            .unwrap()
            .get("finalized_block_number")
            .unwrap()
        else {
            return Ok(None);
        };

        let mut bytes = [0u8; 8];
        bytes.copy_from_slice(&number_bytes);
        Ok(Some(BlockNumber::from_be_bytes(bytes)))
    }

    // Implementation for update_safe_block_number
    fn update_safe_block_number(&self, block_number: BlockNumber) -> Result<(), StoreError> {
        self.partitions
            .read()
            .unwrap()
            .get(ChainData::table_name())
            .unwrap()
            .insert("safe_block_number", block_number.to_be_bytes().to_vec())
            .unwrap();
        Ok(())
    }

    // Implementation for get_safe_block_number
    fn get_safe_block_number(&self) -> Result<Option<BlockNumber>, StoreError> {
        todo!()
    }
    fn update_latest_block_number(&self, block_number: BlockNumber) -> Result<(), StoreError> {
        self.partitions
            .read()
            .unwrap()
            .get(BlockNumbers::table_name())
            .unwrap()
            .insert("latest", block_number.to_be_bytes())
            .unwrap();
        Ok(())
    }

    fn get_latest_block_number(&self) -> Result<Option<BlockNumber>, StoreError> {
        let num = self
            .partitions
            .read()
            .unwrap()
            .get(BlockNumbers::table_name())
            .unwrap()
            .get("latest")
            .unwrap()
            .unwrap()
            .to_vec();
        Ok(Some(u64::from_be_bytes(num.try_into().unwrap())))
    }

    fn update_pending_block_number(&self, block_number: BlockNumber) -> Result<(), StoreError> {
        todo!()
    }

    fn get_pending_block_number(&self) -> Result<Option<BlockNumber>, StoreError> {
        todo!()
    }

    fn open_storage_trie(&self, hashed_address: H256, storage_root: H256) -> Trie {
        // Get the StorageTriesNodes partition from the RwLock-protected HashMap
        let partitions = self.partitions.read().unwrap();
        let storage_partition = partitions.get("state_trie_nodes").unwrap().clone();

        // Create a box of the FjallDupsortTrieDB with the address as fixed key
        let db = Box::new(FjallDupsortTrieDB::<[u8; 32]>::new(
            storage_partition,
            hashed_address.0,
        ));

        // Open the trie with the provided storage root
        Trie::open(db, storage_root)
    }

    fn open_state_trie(&self, state_root: H256) -> Trie {
        // Get the StateTrieNodes partition from the RwLock-protected HashMap
        let partitions = self.partitions.read().unwrap();
        let state_partition = partitions.get("state_trie_nodes").unwrap().clone();

        // Create a box of the FjallTrie for the state trie
        let db = Box::new(FjallTrie::new(state_partition));

        // Open the trie with the provided state root
        Trie::open(db, state_root)
    }

    fn set_canonical_block(&self, number: BlockNumber, hash: BlockHash) -> Result<(), StoreError> {
        let partitions = self.partitions.read().unwrap();
        let partition = partitions.get(CanonicalBlockHashes::table_name()).unwrap();
        let number_bytes = number.to_be_bytes();
        partition.insert(number_bytes, hash.as_bytes()).unwrap();

        Ok(())
    }

    fn unset_canonical_block(&self, number: BlockNumber) -> Result<(), StoreError> {
        // Get the partition for canonical block hashes
        let partitions = self.partitions.read().unwrap();
        let partition = partitions.get(CanonicalBlockHashes::table_name()).unwrap();

        let number_bytes = number.to_be_bytes();
        partition.remove(&number_bytes).unwrap();

        Ok(())
    }

    fn add_payload(
        &self,
        payload_id: u64,
        block: ethrex_common::types::Block,
    ) -> Result<(), StoreError> {
        todo!()
    }

    fn get_payload(&self, payload_id: u64) -> Result<Option<PayloadBundle>, StoreError> {
        todo!()
    }

    fn update_payload(&self, payload_id: u64, payload: PayloadBundle) -> Result<(), StoreError> {
        todo!()
    }

    fn get_receipts_for_block(
        &self,
        block_hash: &BlockHash,
    ) -> Result<Vec<ethrex_common::types::Receipt>, StoreError> {
        todo!()
    }

    fn set_header_download_checkpoint(&self, block_hash: BlockHash) -> Result<(), StoreError> {
        todo!()
    }

    fn get_header_download_checkpoint(&self) -> Result<Option<BlockHash>, StoreError> {
        todo!()
    }

    fn set_state_trie_key_checkpoint(
        &self,
        last_keys: [H256; crate::STATE_TRIE_SEGMENTS],
    ) -> Result<(), StoreError> {
        todo!()
    }

    fn get_state_trie_key_checkpoint(
        &self,
    ) -> Result<Option<[H256; crate::STATE_TRIE_SEGMENTS]>, StoreError> {
        todo!()
    }

    fn set_storage_heal_paths(
        &self,
        accounts: Vec<(H256, Vec<ethrex_trie::Nibbles>)>,
    ) -> Result<(), StoreError> {
        todo!()
    }

    fn get_storage_heal_paths(
        &self,
    ) -> Result<Option<Vec<(H256, Vec<ethrex_trie::Nibbles>)>>, StoreError> {
        todo!()
    }

    fn set_state_heal_paths(&self, paths: Vec<ethrex_trie::Nibbles>) -> Result<(), StoreError> {
        todo!()
    }

    fn get_state_heal_paths(&self) -> Result<Option<Vec<ethrex_trie::Nibbles>>, StoreError> {
        todo!()
    }

    fn clear_snap_state(&self) -> Result<(), StoreError> {
        todo!()
    }

    fn is_synced(&self) -> Result<bool, StoreError> {
        todo!()
    }

    fn update_sync_status(&self, status: bool) -> Result<(), StoreError> {
        self.partitions
            .read()
            .unwrap()
            .get(ChainData::table_name())
            .unwrap()
            .insert("is_synced", &status.to_string())
            .unwrap();

        Ok(())
    }

    fn write_snapshot_account_batch(
        &self,
        account_hashes: Vec<H256>,
        account_states: Vec<ethrex_common::types::AccountState>,
    ) -> Result<(), StoreError> {
        todo!()
    }

    fn write_snapshot_storage_batch(
        &self,
        account_hash: H256,
        storage_keys: Vec<H256>,
        storage_values: Vec<ethrex_common::U256>,
    ) -> Result<(), StoreError> {
        todo!()
    }

    fn set_state_trie_rebuild_checkpoint(
        &self,
        checkpoint: (H256, [H256; crate::STATE_TRIE_SEGMENTS]),
    ) -> Result<(), StoreError> {
        todo!()
    }

    fn get_state_trie_rebuild_checkpoint(
        &self,
    ) -> Result<Option<(H256, [H256; crate::STATE_TRIE_SEGMENTS])>, StoreError> {
        todo!()
    }

    fn set_storage_trie_rebuild_pending(
        &self,
        pending: Vec<(H256, H256)>,
    ) -> Result<(), StoreError> {
        todo!()
    }

    fn get_storage_trie_rebuild_pending(&self) -> Result<Option<Vec<(H256, H256)>>, StoreError> {
        todo!()
    }

    fn clear_snapshot(&self) -> Result<(), StoreError> {
        todo!()
    }

    fn read_account_snapshot(
        &self,
        start: H256,
    ) -> Result<Vec<(H256, ethrex_common::types::AccountState)>, StoreError> {
        todo!()
    }

    fn read_storage_snapshot(
        &self,
        start: H256,
        account_hash: H256,
    ) -> Result<Vec<(H256, ethrex_common::U256)>, StoreError> {
        todo!()
    }
}

// Define the FjallStorable trait
// Define the FjallStorable trait with associated types for Key and Value
pub trait FjallStorable {
    type Key;
    type Value;

    fn table_name() -> &'static str;

    // You might want additional methods like:
    // fn encode_key(key: &Self::Key) -> Vec<u8>;
    // fn decode_key(bytes: &[u8]) -> Self::Key;
    // fn encode_value(value: &Self::Value) -> Vec<u8>;
    // fn decode_value(bytes: &[u8]) -> Self::Value;
}

// Create individual structs with their corresponding key and value types
pub struct CanonicalBlockHashes;
impl FjallStorable for CanonicalBlockHashes {
    type Key = BlockNumber;
    type Value = BlockHashRLP;

    fn table_name() -> &'static str {
        "canonical_block_hashes"
    }
}

pub struct BlockNumbers;
impl FjallStorable for BlockNumbers {
    type Key = BlockHashRLP;
    type Value = BlockNumber;

    fn table_name() -> &'static str {
        "block_numbers"
    }
}

pub struct Headers;
impl FjallStorable for Headers {
    type Key = BlockHashRLP;
    type Value = BlockHeaderRLP;

    fn table_name() -> &'static str {
        "headers"
    }
}

pub struct Bodies;
impl FjallStorable for Bodies {
    type Key = BlockHashRLP;
    type Value = BlockBodyRLP;

    fn table_name() -> &'static str {
        "bodies"
    }
}

pub struct AccountCodes;
impl FjallStorable for AccountCodes {
    type Key = AccountCodeHashRLP;
    type Value = AccountCodeRLP;

    fn table_name() -> &'static str {
        "account_codes"
    }
}

pub struct Receipts;
impl FjallStorable for Receipts {
    type Key = TupleRLP<BlockHash, Index>;
    type Value = ReceiptRLP;

    fn table_name() -> &'static str {
        "receipts"
    }
}

pub struct StorageTriesNodes;
impl FjallStorable for StorageTriesNodes {
    type Key = ([u8; 32], [u8; 33]);
    type Value = Vec<u8>;

    fn table_name() -> &'static str {
        "storage_tries_nodes"
    }
}

pub struct TransactionLocations;
impl FjallStorable for TransactionLocations {
    type Key = TransactionHashRLP;
    type Value = Rlp<(BlockNumber, BlockHash, Index)>;

    fn table_name() -> &'static str {
        "transaction_locations"
    }
}

pub struct ChainData;
impl FjallStorable for ChainData {
    type Key = ChainDataIndex;
    type Value = Vec<u8>;

    fn table_name() -> &'static str {
        "chain_data"
    }
}

pub struct SnapState;
impl FjallStorable for SnapState {
    type Key = SnapStateIndex;
    type Value = Vec<u8>;

    fn table_name() -> &'static str {
        "snap_state"
    }
}

pub struct StateTrieNodes;
impl FjallStorable for StateTrieNodes {
    type Key = Vec<u8>;
    type Value = Vec<u8>;

    fn table_name() -> &'static str {
        "state_trie_nodes"
    }
}

pub struct Payloads;
impl FjallStorable for Payloads {
    type Key = u64;
    type Value = Rlp<PayloadBundle>;

    fn table_name() -> &'static str {
        "payloads"
    }
}

pub struct PendingBlocks;
impl FjallStorable for PendingBlocks {
    type Key = Rlp<BlockHash>;
    type Value = BlockRLP;

    fn table_name() -> &'static str {
        "pending_blocks"
    }
}

pub struct StateSnapShot;
impl FjallStorable for StateSnapShot {
    type Key = AccountHashRLP;
    type Value = AccountStateRLP;

    fn table_name() -> &'static str {
        "state_snapshot"
    }
}

pub struct StorageSnapshot;
pub struct AccountStorageKeyBytes(pub [u8; 32]);
pub struct AccountStorageValueBytes(pub [u8; 32]);
impl FjallStorable for StorageSnapshot {
    type Key = Rlp<H256>;
    type Value = (AccountStorageKeyBytes, AccountStorageValueBytes);

    fn table_name() -> &'static str {
        "storage_snapshot"
    }
}
