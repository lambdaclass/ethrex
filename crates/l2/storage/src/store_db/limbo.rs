use std::fmt::Debug;

use crate::api::StoreEngineRollup;
use ethrex_common::{
    types::{Blob, BlockNumber},
    H256,
};
use ethrex_storage::error::StoreError;
use limbo::{Builder, Connection, Row, Value};

pub struct LimboStore {
    conn: Connection,
}

impl Debug for LimboStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("data")
    }
}

const DB_SCHEMA: [&str; 9] = [
    "CREATE TABLE blocks (block_number INT PRIMARY KEY, batch INT)",
    "CREATE TABLE withdrawals (batch INT, withdrawal_hash BLOB, PRIMARY KEY (batch, withdrawal_hash))",
    "CREATE TABLE deposits (batch INT PRIMARY KEY, deposit_hash BLOB)",
    "CREATE TABLE state_roots (batch INT PRIMARY KEY, state_root BLOB)",
    "CREATE TABLE blob_bundles (batch INT, blob_bundle BLOB, PRIMARY KEY (batch, blob_bundle))",
    "CREATE TABLE operation_count (_id INT PRIMARY KEY, transactions INT, deposits INT, withdrawals INT)",
    "INSERT INTO operation_count VALUES (0, 0, 0, 0)",
    "CREATE TABLE latest_sent (_id INT PRIMARY KEY, batch INT)",
    "INSERT INTO latest_sent VALUES (0, 0)",
];

impl LimboStore {
    pub async fn new(path: &str) -> Result<Self, StoreError> {
        let db = Builder::new_local(path).build().await?;
        let conn = db.connect()?;
        for line in DB_SCHEMA {
            conn.execute(line, ()).await?;
        }
        Ok(LimboStore { conn })
    }
}

fn read_from_row_int(row: &Row, index: usize) -> Result<u64, StoreError> {
    match row.get_value(index)? {
        Value::Integer(i) => {
            let val = i
                .try_into()
                .map_err(|e| StoreError::Custom(format!("conversion error: {e}")))?;
            return Ok(val);
        }
        _ => return Err(StoreError::LimboInvalidTypeError),
    }
}

fn read_from_row_blob(row: &Row, index: usize) -> Result<Vec<u8>, StoreError> {
    match row.get_value(index)? {
        Value::Blob(vec) => {
            return Ok(vec);
        }
        _ => return Err(StoreError::LimboInvalidTypeError),
    }
}

#[async_trait::async_trait]
impl StoreEngineRollup for LimboStore {
    async fn get_batch_number_by_block(
        &self,
        block_number: BlockNumber,
    ) -> Result<Option<u64>, StoreError> {
        let mut rows = self
            .conn
            .query(
                "SELECT * from blocks WHERE block_number = ?1",
                vec![block_number],
            )
            .await?;
        while let Some(row) = rows.next().await? {
            return Ok(Some(read_from_row_int(&row, 1)?));
        }
        Ok(None)
    }

    /// Stores the batch number by a given block number.
    async fn store_batch_number_by_block(
        &self,
        block_number: BlockNumber,
        batch_number: u64,
    ) -> Result<(), StoreError> {
        self.conn.execute("DELETE FROM blocks WHERE block_number = ?1", vec![block_number]).await?;
        self.conn
            .execute(
                "INSERT INTO blocks VALUES (?1, ?2)",
                vec![block_number, batch_number],
            )
            .await?;
        Ok(())
    }

    /// Gets the withdrawal hashes by a given batch number.
    async fn get_withdrawal_hashes_by_batch(
        &self,
        batch_number: u64,
    ) -> Result<Option<Vec<H256>>, StoreError> {
        let mut hashes = vec![];
        let mut rows = self
            .conn
            .query(
                "SELECT * from withdrawals WHERE batch = ?1",
                vec![batch_number],
            )
            .await?;
        while let Some(row) = rows.next().await? {
            let vec = read_from_row_blob(&row, 1)?;
            hashes.push(H256::from_slice(&vec));
        }
        if hashes.is_empty() {
            Ok(None)
        } else {
            Ok(Some(hashes))
        }
    }

    /// Stores the withdrawal hashes by a given batch number.
    async fn store_withdrawal_hashes_by_batch(
        &self,
        batch_number: u64,
        withdrawal_hashes: Vec<H256>,
    ) -> Result<(), StoreError> {
        self.conn.execute("DELETE FROM withdrawals WHERE batch = ?1", vec![batch_number]).await?;
        for hash in withdrawal_hashes {
            self.conn
                .execute(
                    "INSERT INTO withdrawals VALUES (?1, ?2)",
                    (batch_number, Vec::from(hash.to_fixed_bytes())),
                )
                .await?;
        }
        Ok(())
    }

    /// Stores the block numbers by a given batch_number
    async fn store_block_numbers_by_batch(
        &self,
        batch_number: u64,
        block_numbers: Vec<BlockNumber>,
    ) -> Result<(), StoreError> {
        for block_number in block_numbers {
            self.store_batch_number_by_block(block_number, batch_number)
                .await?;
        }
        Ok(())
    }

    /// Returns the block numbers by a given batch_number
    async fn get_block_numbers_by_batch(
        &self,
        batch_number: u64,
    ) -> Result<Option<Vec<BlockNumber>>, StoreError> {
        let mut blocks = Vec::new();
        let mut rows = self
            .conn
            .query("SELECT * from blocks WHERE batch = ?1", vec![batch_number])
            .await?;
        while let Some(row) = rows.next().await? {
            let val = read_from_row_int(&row, 0)?;
            blocks.push(val);
        }
        if blocks.is_empty() {
            Ok(None)
        } else {
            Ok(Some(blocks))
        }
    }

    async fn store_deposit_logs_hash_by_batch_number(
        &self,
        batch_number: u64,
        deposit_logs_hash: H256,
    ) -> Result<(), StoreError> {
        self.conn.execute("DELETE FROM deposits WHERE batch = ?1", vec![batch_number]).await?;
        self.conn
            .execute(
                "INSERT INTO deposits VALUES (?1, ?2)",
                (batch_number, Vec::from(deposit_logs_hash.to_fixed_bytes())),
            )
            .await?;
        Ok(())
    }

    async fn get_deposit_logs_hash_by_batch_number(
        &self,
        batch_number: u64,
    ) -> Result<Option<H256>, StoreError> {
        let mut rows = self
            .conn
            .query(
                "SELECT * from deposits WHERE batch = ?1",
                vec![batch_number],
            )
            .await?;
        while let Some(row) = rows.next().await? {
            let vec = read_from_row_blob(&row, 1)?;
            return Ok(Some(H256::from_slice(&vec)));
        }
        Ok(None)
    }

    async fn store_state_root_by_batch_number(
        &self,
        batch_number: u64,
        state_root: H256,
    ) -> Result<(), StoreError> {
        self.conn.execute("DELETE FROM state_roots WHERE batch = ?1", vec![batch_number]).await?;
        self.conn
            .execute(
                "INSERT INTO state_roots VALUES (?1, ?2)",
                (batch_number, Vec::from(state_root.to_fixed_bytes())),
            )
            .await?;
        Ok(())
    }

    async fn get_state_root_by_batch_number(
        &self,
        batch_number: u64,
    ) -> Result<Option<H256>, StoreError> {
        let mut rows = self
            .conn
            .query(
                "SELECT * from state_roots WHERE batch = ?1",
                vec![batch_number],
            )
            .await?;
        while let Some(row) = rows.next().await? {
            let vec = read_from_row_blob(&row, 1)?;
            return Ok(Some(H256::from_slice(&vec)));
        }
        Ok(None)
    }

    async fn store_blob_bundle_by_batch_number(
        &self,
        batch_number: u64,
        state_diff: Vec<Blob>,
    ) -> Result<(), StoreError> {
        self.conn.execute("DELETE FROM blob_bundles WHERE batch = ?1", vec![batch_number]).await?;
        for blob in state_diff {
            self.conn
                .execute(
                    "INSERT INTO blob_bundles VALUES (?1, ?2)",
                    (batch_number, blob.to_vec()),
                )
                .await?;
        }
        Ok(())
    }

    async fn get_blob_bundle_by_batch_number(
        &self,
        batch_number: u64,
    ) -> Result<Option<Vec<Blob>>, StoreError> {
        let mut bundles = Vec::new();
        let mut rows = self
            .conn
            .query(
                "SELECT * from blob_bundles WHERE batch = ?1",
                vec![batch_number],
            )
            .await?;
        while let Some(row) = rows.next().await? {
            let val = read_from_row_blob(&row, 1)?;
            bundles.push(
                Blob::try_from(val)
                    .map_err(|_| StoreError::Custom(format!("error converting to Blob")))?,
            );
        }
        if bundles.is_empty() {
            Ok(None)
        } else {
            Ok(Some(bundles))
        }
    }

    async fn update_operations_count(
        &self,
        transaction_inc: u64,
        deposits_inc: u64,
        withdrawals_inc: u64,
    ) -> Result<(), StoreError> {
        self.conn.execute(
            "UPDATE operation_count SET transactions = transactions + ?1, deposits = deposits + ?2, withdrawals = withdrawals + ?3", 
            (transaction_inc, deposits_inc, withdrawals_inc)).await?;
        Ok(())
    }

    async fn get_operations_count(&self) -> Result<[u64; 3], StoreError> {
        let mut rows = self.conn.query("SELECT * from operation_count", ()).await?;
        if let Some(row) = rows.next().await? {
            return Ok([
                read_from_row_int(&row, 1)?,
                read_from_row_int(&row, 2)?,
                read_from_row_int(&row, 3)?,
            ]);
        }
        Err(StoreError::Custom(
            "missing operation_count row".to_string(),
        ))
    }

    /// Returns whether the batch with the given number is present.
    async fn contains_batch(&self, batch_number: &u64) -> Result<bool, StoreError> {
        let mut row = self
            .conn
            .query("SELECT * from blocks WHERE batch = ?1", vec![*batch_number])
            .await?;
        Ok(row.next().await?.is_some())
    }

    async fn get_lastest_sent_batch_proof(&self) -> Result<u64, StoreError> {
        let mut rows = self.conn.query("SELECT * from latest_sent", ()).await?;
        if let Some(row) = rows.next().await? {
            return Ok(read_from_row_int(&row, 1)?);
        }
        Err(StoreError::Custom(
            "missing operation_count row".to_string(),
        ))
    }

    async fn set_lastest_sent_batch_proof(&self, batch_number: u64) -> Result<(), StoreError> {
        self.conn
            .execute("UPDATE latest_sent SET batch = ?1", (0, batch_number))
            .await?;
        Ok(())
    }
}
