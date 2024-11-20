use super::api::StoreEngine;
use crate::error::StoreError;
use crate::rlp::{
    AccountCodeHashRLP, AccountCodeRLP, BlockBodyRLP, BlockHashRLP, BlockHeaderRLP, BlockRLP,
    BlockTotalDifficultyRLP, ReceiptRLP, Rlp, TransactionHashRLP, TupleRLP,
};
use anyhow::Result;
use bytes::Bytes;
use ethereum_types::{H256, U256};
use ethrex_core::types::{
    Block, BlockBody, BlockHash, BlockHeader, BlockNumber, ChainConfig, Index, Receipt, Transaction,
};
use ethrex_rlp::decode::RLPDecode;
use ethrex_rlp::encode::RLPEncode;
use ethrex_trie::{LibmdbxDupsortTrieDB, LibmdbxTrieDB, Trie};
use libmdbx::orm::{Decodable, Encodable, Table};
use libmdbx::{
    dupsort,
    orm::{table, Database},
    table_info,
};
use libmdbx::{DatabaseOptions, Mode, ReadWriteOptions};
use serde_json;
use std::fmt::{Debug, Formatter};
use std::path::Path;
use std::sync::Arc;

pub struct Store {
    db: Arc<Database>,
}
impl Store {
    pub fn new(path: &str) -> Result<Self, StoreError> {
        Ok(Self {
            db: Arc::new(init_db(Some(path))),
        })
    }

    // Helper method to write into a libmdbx table
    fn write<T: Table>(&self, key: T::Key, value: T::Value) -> Result<(), StoreError> {
        let txn = self
            .db
            .begin_readwrite()
            .map_err(StoreError::LibmdbxError)?;
        txn.upsert::<T>(key, value)
            .map_err(StoreError::LibmdbxError)?;
        txn.commit().map_err(StoreError::LibmdbxError)
    }

    // Helper method to read from a libmdbx table
    fn read<T: Table>(&self, key: T::Key) -> Result<Option<T::Value>, StoreError> {
        let txn = self.db.begin_read().map_err(StoreError::LibmdbxError)?;
        txn.get::<T>(key).map_err(StoreError::LibmdbxError)
    }

    fn get_block_hash_by_block_number(
        &self,
        number: BlockNumber,
    ) -> Result<Option<BlockHash>, StoreError> {
        Ok(self.read::<CanonicalBlockHashes>(number)?.map(|a| a.to()))
    }
}

impl StoreEngine for Store {
    fn add_block_header(
        &self,
        block_hash: BlockHash,
        block_header: BlockHeader,
    ) -> std::result::Result<(), StoreError> {
        self.write::<Headers>(block_hash.into(), block_header.into())
    }

    fn get_block_header(
        &self,
        block_number: BlockNumber,
    ) -> Result<Option<BlockHeader>, StoreError> {
        if let Some(hash) = self.get_block_hash_by_block_number(block_number)? {
            Ok(self.read::<Headers>(hash.into())?.map(|b| b.to()))
        } else {
            Ok(None)
        }
    }

    fn add_block_body(
        &self,
        block_hash: BlockHash,
        block_body: BlockBody,
    ) -> std::result::Result<(), StoreError> {
        self.write::<Bodies>(block_hash.into(), block_body.into())
    }

    fn get_block_body(
        &self,
        block_number: BlockNumber,
    ) -> std::result::Result<Option<BlockBody>, StoreError> {
        if let Some(hash) = self.get_block_hash_by_block_number(block_number)? {
            self.get_block_body_by_hash(hash)
        } else {
            Ok(None)
        }
    }

    fn get_block_body_by_hash(
        &self,
        block_hash: BlockHash,
    ) -> Result<Option<BlockBody>, StoreError> {
        Ok(self.read::<Bodies>(block_hash.into())?.map(|b| b.to()))
    }

    fn get_block_header_by_hash(
        &self,
        block_hash: BlockHash,
    ) -> Result<Option<BlockHeader>, StoreError> {
        Ok(self.read::<Headers>(block_hash.into())?.map(|b| b.to()))
    }

    fn add_block_number(
        &self,
        block_hash: BlockHash,
        block_number: BlockNumber,
    ) -> std::result::Result<(), StoreError> {
        self.write::<BlockNumbers>(block_hash.into(), block_number)
    }

    fn get_block_number(
        &self,
        block_hash: BlockHash,
    ) -> std::result::Result<Option<BlockNumber>, StoreError> {
        self.read::<BlockNumbers>(block_hash.into())
    }
    fn add_block_total_difficulty(
        &self,
        block_hash: BlockHash,
        block_total_difficulty: U256,
    ) -> std::result::Result<(), StoreError> {
        self.write::<BlockTotalDifficulties>(block_hash.into(), block_total_difficulty.into())
    }

    fn get_block_total_difficulty(
        &self,
        block_hash: BlockHash,
    ) -> std::result::Result<Option<U256>, StoreError> {
        Ok(self
            .read::<BlockTotalDifficulties>(block_hash.into())?
            .map(|b| b.to()))
    }

    fn add_account_code(&self, code_hash: H256, code: Bytes) -> Result<(), StoreError> {
        self.write::<AccountCodes>(code_hash.into(), code.into())
    }

    fn get_account_code(&self, code_hash: H256) -> Result<Option<Bytes>, StoreError> {
        Ok(self.read::<AccountCodes>(code_hash.into())?.map(|b| b.to()))
    }

    fn add_receipt(
        &self,
        block_hash: BlockHash,
        index: Index,
        receipt: Receipt,
    ) -> Result<(), StoreError> {
        self.write::<Receipts>((block_hash, index).into(), receipt.into())
    }

    fn get_receipt(
        &self,
        block_number: BlockNumber,
        index: Index,
    ) -> Result<Option<Receipt>, StoreError> {
        if let Some(hash) = self.get_block_hash_by_block_number(block_number)? {
            Ok(self.read::<Receipts>((hash, index).into())?.map(|b| b.to()))
        } else {
            Ok(None)
        }
    }

    fn add_transaction_location(
        &self,
        transaction_hash: H256,
        block_number: BlockNumber,
        block_hash: BlockHash,
        index: Index,
    ) -> Result<(), StoreError> {
        self.write::<TransactionLocations>(
            transaction_hash.into(),
            (block_number, block_hash, index).into(),
        )
    }

    fn get_transaction_location(
        &self,
        transaction_hash: H256,
    ) -> Result<Option<(BlockNumber, BlockHash, Index)>, StoreError> {
        let txn = self.db.begin_read().map_err(StoreError::LibmdbxError)?;
        let cursor = txn
            .cursor::<TransactionLocations>()
            .map_err(StoreError::LibmdbxError)?;
        Ok(cursor
            .walk_key(transaction_hash.into(), None)
            .map_while(|res| res.ok().map(|t| t.to()))
            .find(|(number, hash, _index)| {
                self.get_block_hash_by_block_number(*number)
                    .is_ok_and(|o| o == Some(*hash))
            }))
    }

    /// Stores the chain config serialized as json
    fn set_chain_config(&self, chain_config: &ChainConfig) -> Result<(), StoreError> {
        self.write::<ChainData>(
            ChainDataIndex::ChainConfig,
            serde_json::to_string(chain_config)
                .map_err(|_| StoreError::DecodeError)?
                .into_bytes(),
        )
    }

    fn get_chain_config(&self) -> Result<ChainConfig, StoreError> {
        match self.read::<ChainData>(ChainDataIndex::ChainConfig)? {
            None => Err(StoreError::Custom("Chain config not found".to_string())),
            Some(bytes) => {
                let json = String::from_utf8(bytes).map_err(|_| StoreError::DecodeError)?;
                let chain_config: ChainConfig =
                    serde_json::from_str(&json).map_err(|_| StoreError::DecodeError)?;
                Ok(chain_config)
            }
        }
    }

    fn update_earliest_block_number(&self, block_number: BlockNumber) -> Result<(), StoreError> {
        self.write::<ChainData>(
            ChainDataIndex::EarliestBlockNumber,
            block_number.encode_to_vec(),
        )
    }

    fn get_earliest_block_number(&self) -> Result<Option<BlockNumber>, StoreError> {
        match self.read::<ChainData>(ChainDataIndex::EarliestBlockNumber)? {
            None => Ok(None),
            Some(ref rlp) => RLPDecode::decode(rlp)
                .map(Some)
                .map_err(|_| StoreError::DecodeError),
        }
    }

    fn update_finalized_block_number(&self, block_number: BlockNumber) -> Result<(), StoreError> {
        self.write::<ChainData>(
            ChainDataIndex::FinalizedBlockNumber,
            block_number.encode_to_vec(),
        )
    }

    fn get_finalized_block_number(&self) -> Result<Option<BlockNumber>, StoreError> {
        match self.read::<ChainData>(ChainDataIndex::FinalizedBlockNumber)? {
            None => Ok(None),
            Some(ref rlp) => RLPDecode::decode(rlp)
                .map(Some)
                .map_err(|_| StoreError::DecodeError),
        }
    }

    fn update_safe_block_number(&self, block_number: BlockNumber) -> Result<(), StoreError> {
        self.write::<ChainData>(
            ChainDataIndex::SafeBlockNumber,
            block_number.encode_to_vec(),
        )
    }

    fn get_safe_block_number(&self) -> Result<Option<BlockNumber>, StoreError> {
        match self.read::<ChainData>(ChainDataIndex::SafeBlockNumber)? {
            None => Ok(None),
            Some(ref rlp) => RLPDecode::decode(rlp)
                .map(Some)
                .map_err(|_| StoreError::DecodeError),
        }
    }

    fn update_latest_block_number(&self, block_number: BlockNumber) -> Result<(), StoreError> {
        self.write::<ChainData>(
            ChainDataIndex::LatestBlockNumber,
            block_number.encode_to_vec(),
        )
    }

    fn get_latest_block_number(&self) -> Result<Option<BlockNumber>, StoreError> {
        match self.read::<ChainData>(ChainDataIndex::LatestBlockNumber)? {
            None => Ok(None),
            Some(ref rlp) => RLPDecode::decode(rlp)
                .map(Some)
                .map_err(|_| StoreError::DecodeError),
        }
    }

    fn update_latest_total_difficulty(
        &self,
        latest_total_difficulty: U256,
    ) -> std::result::Result<(), StoreError> {
        self.write::<ChainData>(
            ChainDataIndex::LatestTotalDifficulty,
            latest_total_difficulty.encode_to_vec(),
        )
    }

    fn get_latest_total_difficulty(&self) -> Result<Option<U256>, StoreError> {
        match self.read::<ChainData>(ChainDataIndex::LatestTotalDifficulty)? {
            None => Ok(None),
            Some(ref rlp) => RLPDecode::decode(rlp)
                .map(Some)
                .map_err(|_| StoreError::DecodeError),
        }
    }

    fn update_pending_block_number(&self, block_number: BlockNumber) -> Result<(), StoreError> {
        self.write::<ChainData>(
            ChainDataIndex::PendingBlockNumber,
            block_number.encode_to_vec(),
        )
    }

    fn get_pending_block_number(&self) -> Result<Option<BlockNumber>, StoreError> {
        match self.read::<ChainData>(ChainDataIndex::PendingBlockNumber)? {
            None => Ok(None),
            Some(ref rlp) => RLPDecode::decode(rlp)
                .map(Some)
                .map_err(|_| StoreError::DecodeError),
        }
    }

    fn open_storage_trie(&self, hashed_address: H256, storage_root: H256) -> Trie {
        let db = Box::new(LibmdbxDupsortTrieDB::<StorageTriesNodes, [u8; 32]>::new(
            self.db.clone(),
            hashed_address.0,
        ));
        Trie::open(db, storage_root)
    }

    fn open_state_trie(&self, state_root: H256) -> Trie {
        let db = Box::new(LibmdbxTrieDB::<StateTrieNodes>::new(self.db.clone()));
        Trie::open(db, state_root)
    }

    fn set_canonical_block(&self, number: BlockNumber, hash: BlockHash) -> Result<(), StoreError> {
        self.write::<CanonicalBlockHashes>(number, hash.into())
    }

    fn get_canonical_block_hash(
        &self,
        number: BlockNumber,
    ) -> Result<Option<BlockHash>, StoreError> {
        self.read::<CanonicalBlockHashes>(number)
            .map(|o| o.map(|hash_rlp| hash_rlp.to()))
    }

    fn add_payload(&self, payload_id: u64, block: Block) -> Result<(), StoreError> {
        self.write::<Payloads>(payload_id, block.into())
    }

    fn get_payload(&self, payload_id: u64) -> Result<Option<Block>, StoreError> {
        Ok(self.read::<Payloads>(payload_id)?.map(|b| b.to()))
    }

    fn get_transaction_by_hash(
        &self,
        transaction_hash: H256,
    ) -> std::result::Result<Option<Transaction>, StoreError> {
        let (_block_number, block_hash, index) =
            match self.get_transaction_location(transaction_hash)? {
                Some(location) => location,
                None => return Ok(None),
            };
        self.get_transaction_by_location(block_hash, index)
    }

    fn get_transaction_by_location(
        &self,
        block_hash: H256,
        index: u64,
    ) -> std::result::Result<Option<Transaction>, StoreError> {
        let block_body = match self.get_block_body_by_hash(block_hash)? {
            Some(body) => body,
            None => return Ok(None),
        };
        Ok(index
            .try_into()
            .ok()
            .and_then(|index: usize| block_body.transactions.get(index).cloned()))
    }

    fn get_block_by_hash(
        &self,
        block_hash: BlockHash,
    ) -> std::result::Result<Option<Block>, StoreError> {
        let header = match self.get_block_header_by_hash(block_hash)? {
            Some(header) => header,
            None => return Ok(None),
        };
        let body = match self.get_block_body_by_hash(block_hash)? {
            Some(body) => body,
            None => return Ok(None),
        };
        Ok(Some(Block::new(header, body)))
    }

    fn unset_canonical_block(&self, number: BlockNumber) -> Result<(), StoreError> {
        self.db
            .begin_readwrite()
            .map_err(StoreError::LibmdbxError)?
            .delete::<CanonicalBlockHashes>(number, None)
            .map(|_| ())
            .map_err(StoreError::LibmdbxError)
    }

    fn add_pending_block(&self, block: Block) -> std::result::Result<(), StoreError> {
        self.write::<PendingBlocks>(block.header.compute_block_hash().into(), block.into())
    }

    fn get_pending_block(
        &self,
        block_hash: BlockHash,
    ) -> std::result::Result<Option<Block>, StoreError> {
        Ok(self
            .read::<PendingBlocks>(block_hash.into())?
            .map(|b| b.to()))
    }
}

impl Debug for Store {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Libmdbx Store").finish()
    }
}

// Define tables

table!(
    /// The canonical block hash for each block number. It represents the canonical chain.
    ( CanonicalBlockHashes ) BlockNumber => BlockHashRLP
);

table!(
    /// Block hash to number table.
    ( BlockNumbers ) BlockHashRLP => BlockNumber
);

// TODO (#307): Remove TotalDifficulty.
table!(
    /// Block hash to total difficulties table.
    ( BlockTotalDifficulties ) BlockHashRLP => BlockTotalDifficultyRLP
);

table!(
    /// Block headers table.
    ( Headers ) BlockHashRLP => BlockHeaderRLP
);
table!(
    /// Block bodies table.
    ( Bodies ) BlockHashRLP => BlockBodyRLP
);
table!(
    /// Account codes table.
    ( AccountCodes ) AccountCodeHashRLP => AccountCodeRLP
);

dupsort!(
    /// Receipts table.
    ( Receipts ) TupleRLP<BlockHash, Index>[Index] => ReceiptRLP
);

dupsort!(
    /// Table containing all storage trie's nodes
    /// Each node is stored by hashed account address and node hash in order to keep different storage trie's nodes separate
    ( StorageTriesNodes ) ([u8;32], [u8;33])[[u8;32]] => Vec<u8>
);

dupsort!(
    /// Transaction locations table.
    ( TransactionLocations ) TransactionHashRLP => Rlp<(BlockNumber, BlockHash, Index)>
);

table!(
    /// Stores chain data, each value is unique and stored as its rlp encoding
    /// See [ChainDataIndex] for available chain values
    ( ChainData ) ChainDataIndex => Vec<u8>
);

// Trie storages

table!(
    /// state trie nodes
    ( StateTrieNodes ) Vec<u8> => Vec<u8>
);

// Local Blocks

table!(
    /// payload id to payload block table
    ( Payloads ) u64 => BlockRLP
);

table!(
    /// Stores blocks that are pending validation.
    ( PendingBlocks ) BlockHashRLP => BlockRLP
);

// Storage values are stored as bytes instead of using their rlp encoding
// As they are stored in a dupsort table, they need to have a fixed size, and encoding them doesn't preserve their size
pub struct AccountStorageKeyBytes(pub [u8; 32]);
pub struct AccountStorageValueBytes(pub [u8; 32]);

impl Encodable for AccountStorageKeyBytes {
    type Encoded = [u8; 32];

    fn encode(self) -> Self::Encoded {
        self.0
    }
}

impl Decodable for AccountStorageKeyBytes {
    fn decode(b: &[u8]) -> anyhow::Result<Self> {
        Ok(AccountStorageKeyBytes(b.try_into()?))
    }
}

impl Encodable for AccountStorageValueBytes {
    type Encoded = [u8; 32];

    fn encode(self) -> Self::Encoded {
        self.0
    }
}

impl Decodable for AccountStorageValueBytes {
    fn decode(b: &[u8]) -> anyhow::Result<Self> {
        Ok(AccountStorageValueBytes(b.try_into()?))
    }
}

impl From<H256> for AccountStorageKeyBytes {
    fn from(value: H256) -> Self {
        AccountStorageKeyBytes(value.0)
    }
}

impl From<U256> for AccountStorageValueBytes {
    fn from(value: U256) -> Self {
        let mut value_bytes = [0; 32];
        value.to_big_endian(&mut value_bytes);
        AccountStorageValueBytes(value_bytes)
    }
}

impl From<AccountStorageKeyBytes> for H256 {
    fn from(value: AccountStorageKeyBytes) -> Self {
        H256(value.0)
    }
}

impl From<AccountStorageValueBytes> for U256 {
    fn from(value: AccountStorageValueBytes) -> Self {
        U256::from_big_endian(&value.0)
    }
}

/// Represents the key for each unique value of the chain data stored in the db
// (TODO: Remove this comment once full) Will store chain-specific data such as chain id and latest finalized/pending/safe block number
pub enum ChainDataIndex {
    ChainConfig = 0,
    EarliestBlockNumber = 1,
    FinalizedBlockNumber = 2,
    SafeBlockNumber = 3,
    LatestBlockNumber = 4,
    PendingBlockNumber = 5,
    // TODO (#307): Remove TotalDifficulty.
    LatestTotalDifficulty = 6,
}

impl Encodable for ChainDataIndex {
    type Encoded = [u8; 4];

    fn encode(self) -> Self::Encoded {
        (self as u32).encode()
    }
}

/// Initializes a new database with the provided path. If the path is `None`, the database
/// will be temporary.
pub fn init_db(path: Option<impl AsRef<Path>>) -> Database {
    let tables = [
        table_info!(BlockNumbers),
        // TODO (#307): Remove TotalDifficulty.
        table_info!(BlockTotalDifficulties),
        table_info!(Headers),
        table_info!(Bodies),
        table_info!(AccountCodes),
        table_info!(Receipts),
        table_info!(TransactionLocations),
        table_info!(ChainData),
        table_info!(StateTrieNodes),
        table_info!(StorageTriesNodes),
        table_info!(CanonicalBlockHashes),
        table_info!(Payloads),
        table_info!(PendingBlocks),
    ]
    .into_iter()
    .collect();
    let path = path.map(|p| p.as_ref().to_path_buf());
    let options = DatabaseOptions {
        mode: Mode::ReadWrite(ReadWriteOptions {
            // Set max DB size to 1TB
            max_size: Some(1024_isize.pow(4)),
            ..Default::default()
        }),
        ..Default::default()
    };
    Database::create_with_options(path, options, &tables).unwrap()
}

#[cfg(test)]
mod tests {
    use libmdbx::{
        dupsort,
        orm::{table, Database, Decodable, Encodable},
        table_info,
    };

    #[test]
    fn mdbx_smoke_test() {
        // Declare tables used for the smoke test
        table!(
            /// Example table.
            ( Example ) String => String
        );

        // Assemble database chart
        let tables = [table_info!(Example)].into_iter().collect();

        let key = "Hello".to_string();
        let value = "World!".to_string();

        let db = Database::create(None, &tables).unwrap();

        // Write values
        {
            let txn = db.begin_readwrite().unwrap();
            txn.upsert::<Example>(key.clone(), value.clone()).unwrap();
            txn.commit().unwrap();
        }
        // Read written values
        let read_value = {
            let txn = db.begin_read().unwrap();
            txn.get::<Example>(key).unwrap()
        };
        assert_eq!(read_value, Some(value));
    }

    #[test]
    fn mdbx_structs_smoke_test() {
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub struct ExampleKey([u8; 32]);

        impl Encodable for ExampleKey {
            type Encoded = [u8; 32];

            fn encode(self) -> Self::Encoded {
                Encodable::encode(self.0)
            }
        }

        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub struct ExampleValue {
            x: u64,
            y: [u8; 32],
        }

        impl Encodable for ExampleValue {
            type Encoded = [u8; 40];

            fn encode(self) -> Self::Encoded {
                let mut encoded = [0u8; 40];
                encoded[..8].copy_from_slice(&self.x.to_ne_bytes());
                encoded[8..].copy_from_slice(&self.y);
                encoded
            }
        }

        impl Decodable for ExampleValue {
            fn decode(b: &[u8]) -> anyhow::Result<Self> {
                let x = u64::from_ne_bytes(b[..8].try_into()?);
                let y = b[8..].try_into()?;
                Ok(Self { x, y })
            }
        }

        // Declare tables used for the smoke test
        table!(
            /// Example table.
            ( StructsExample ) ExampleKey => ExampleValue
        );

        // Assemble database chart
        let tables = [table_info!(StructsExample)].into_iter().collect();
        let key = ExampleKey([151; 32]);
        let value = ExampleValue { x: 42, y: [42; 32] };

        let db = Database::create(None, &tables).unwrap();

        // Write values
        {
            let txn = db.begin_readwrite().unwrap();
            txn.upsert::<StructsExample>(key, value).unwrap();
            txn.commit().unwrap();
        }
        // Read written values
        let read_value = {
            let txn = db.begin_read().unwrap();
            txn.get::<StructsExample>(key).unwrap()
        };
        assert_eq!(read_value, Some(value));
    }

    #[test]
    fn mdbx_dupsort_smoke_test() {
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub struct ExampleKey(u8);

        impl Encodable for ExampleKey {
            type Encoded = [u8; 1];

            fn encode(self) -> Self::Encoded {
                [self.0]
            }
        }
        impl Decodable for ExampleKey {
            fn decode(b: &[u8]) -> anyhow::Result<Self> {
                if b.len() != 1 {
                    anyhow::bail!("Invalid length");
                }
                Ok(Self(b[0]))
            }
        }

        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub struct ExampleValue {
            x: u64,
            y: [u8; 32],
        }

        impl Encodable for ExampleValue {
            type Encoded = [u8; 40];

            fn encode(self) -> Self::Encoded {
                let mut encoded = [0u8; 40];
                encoded[..8].copy_from_slice(&self.x.to_ne_bytes());
                encoded[8..].copy_from_slice(&self.y);
                encoded
            }
        }

        impl Decodable for ExampleValue {
            fn decode(b: &[u8]) -> anyhow::Result<Self> {
                let x = u64::from_ne_bytes(b[..8].try_into()?);
                let y = b[8..].try_into()?;
                Ok(Self { x, y })
            }
        }

        // Declare tables used for the smoke test
        dupsort!(
            /// Example table.
            ( DupsortExample ) ExampleKey => (ExampleKey, ExampleValue) [ExampleKey]
        );

        // Assemble database chart
        let tables = [table_info!(DupsortExample)].into_iter().collect();
        let key = ExampleKey(151);
        let subkey1 = ExampleKey(16);
        let subkey2 = ExampleKey(42);
        let value = ExampleValue { x: 42, y: [42; 32] };

        let db = Database::create(None, &tables).unwrap();

        // Write values
        {
            let txn = db.begin_readwrite().unwrap();
            txn.upsert::<DupsortExample>(key, (subkey1, value)).unwrap();
            txn.upsert::<DupsortExample>(key, (subkey2, value)).unwrap();
            txn.commit().unwrap();
        }
        // Read written values
        {
            let txn = db.begin_read().unwrap();
            let mut cursor = txn.cursor::<DupsortExample>().unwrap();
            let value1 = cursor.seek_exact(key).unwrap().unwrap();
            assert_eq!(value1, (key, (subkey1, value)));
            let value2 = cursor.seek_value(key, subkey2).unwrap().unwrap();
            assert_eq!(value2, (subkey2, value));
        };

        // Walk through duplicates
        {
            let txn = db.begin_read().unwrap();
            let cursor = txn.cursor::<DupsortExample>().unwrap();
            let mut acc = 0;
            for key in cursor.walk_key(key, None).map(|r| r.unwrap().0 .0) {
                acc += key;
            }

            assert_eq!(acc, 58);
        }
    }
}
