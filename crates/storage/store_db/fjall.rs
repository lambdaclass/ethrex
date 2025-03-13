use crate::{
    error::StoreError,
    rlp::{
        AccountCodeHashRLP, AccountCodeRLP, AccountHashRLP, AccountStateRLP, BlockBodyRLP,
        BlockHashRLP, BlockHeaderRLP, BlockRLP, PayloadBundleRLP, ReceiptRLP, Rlp,
        TransactionHashRLP, TupleRLP,
    },
    utils::{ChainDataIndex, SnapStateIndex},
};
use ethrex_common::{
    types::{payload::PayloadBundle, BlockHash, BlockNumber, Index},
    H256,
};
use fjall::{Config, Keyspace, PartitionCreateOptions, PersistMode};
use std::path::Path;


pub fn init_db<P: AsRef<Path>>(folder: P) -> Result<Keyspace, StoreError> {
    // Open the keyspace
    let keyspace = Config::new(folder).open().unwrap();

    // Initialize each table as a partition
    keyspace
        .open_partition(
            CanonicalBlockHashes::table_name(),
            PartitionCreateOptions::default(),
        )
        .unwrap();

    keyspace
        .open_partition(
            BlockNumbers::table_name(),
            PartitionCreateOptions::default(),
        )
        .unwrap();

    keyspace
        .open_partition(Headers::table_name(), PartitionCreateOptions::default())
        .unwrap();

    keyspace
        .open_partition(Bodies::table_name(), PartitionCreateOptions::default())
        .unwrap();

    keyspace
        .open_partition(
            AccountCodes::table_name(),
            PartitionCreateOptions::default(),
        )
        .unwrap();

    keyspace
        .open_partition(Receipts::table_name(), PartitionCreateOptions::default())
        .unwrap();

    keyspace
        .open_partition(
            StorageTriesNodes::table_name(),
            PartitionCreateOptions::default(),
        )
        .unwrap();

    keyspace
        .open_partition(
            TransactionLocations::table_name(),
            PartitionCreateOptions::default(),
        )
        .unwrap();

    keyspace
        .open_partition(ChainData::table_name(), PartitionCreateOptions::default())
        .unwrap();

    keyspace
        .open_partition(SnapState::table_name(), PartitionCreateOptions::default())
        .unwrap();

    keyspace
        .open_partition(
            StateTrieNodes::table_name(),
            PartitionCreateOptions::default(),
        )
        .unwrap();

    keyspace
        .open_partition(Payloads::table_name(), PartitionCreateOptions::default())
        .unwrap();

    keyspace
        .open_partition(
            PendingBlocks::table_name(),
            PartitionCreateOptions::default(),
        )
        .unwrap();

    keyspace
        .open_partition(
            StateSnapShot::table_name(),
            PartitionCreateOptions::default(),
        )
        .unwrap();

    keyspace
        .open_partition(
            StorageSnapshot::table_name(),
            PartitionCreateOptions::default(),
        )
        .unwrap();

    // Log initialization success
    tracing::log::info!("Database tables initialized successfully");

    Ok(keyspace)
}

pub struct Fjall {
    db: Keyspace,
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
