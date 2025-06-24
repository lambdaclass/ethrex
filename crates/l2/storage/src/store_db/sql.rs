use std::fmt::Debug;

use crate::api::StoreEngineRollup;
use crate::error::RollupStoreError;
use ethrex_common::{
    H256,
    types::{AccountUpdate, Blob, BlockNumber},
};

use libsql::{
    Builder, Connection, Row, Rows, Value,
    params::{IntoParams, Params},
};

pub struct SQLStore {
    conn: Connection,
}

impl Debug for SQLStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SQLStore")
    }
}

const DB_SCHEMA: [&str; 10] = [
    "CREATE TABLE blocks (block_number INT PRIMARY KEY, batch INT)",
    "CREATE TABLE messages (batch INT, idx INT, message_hash BLOB, PRIMARY KEY (batch, idx))",
    "CREATE TABLE deposits (batch INT PRIMARY KEY, deposit_hash BLOB)",
    "CREATE TABLE state_roots (batch INT PRIMARY KEY, state_root BLOB)",
    "CREATE TABLE blob_bundles (batch INT, idx INT, blob_bundle BLOB, PRIMARY KEY (batch, idx))",
    "CREATE TABLE account_updates (block_number INT PRIMARY KEY, updates BLOB)",
    "CREATE TABLE operation_count (_id INT PRIMARY KEY, transactions INT, deposits INT, messages INT)",
    "INSERT INTO operation_count VALUES (0, 0, 0, 0)",
    "CREATE TABLE latest_sent (_id INT PRIMARY KEY, batch INT)",
    "INSERT INTO latest_sent VALUES (0, 0)",
];

impl SQLStore {
    pub fn new(path: &str) -> Result<Self, RollupStoreError> {
        futures::executor::block_on(async {
            let db = Builder::new_local(path).build().await?;
            let conn = db.connect()?;
            let store = SQLStore { conn };
            store.init_db().await?;
            Ok(store)
        })
    }
    async fn execute<T: IntoParams>(&self, sql: &str, params: T) -> Result<(), RollupStoreError> {
        self.conn.execute(sql, params).await?;
        Ok(())
    }
    async fn query<T: IntoParams>(&self, sql: &str, params: T) -> Result<Rows, RollupStoreError> {
        Ok(self.conn.query(sql, params).await?)
    }
    async fn init_db(&self) -> Result<(), RollupStoreError> {
        let mut rows = self
            .query(
                "SELECT name FROM sqlite_schema WHERE type='table' AND name='blocks'",
                (),
            )
            .await?;
        if rows.next().await?.is_none() {
            let empty_param = ().into_params()?;
            let queries = DB_SCHEMA
                .iter()
                .map(|v| (*v, empty_param.clone()))
                .collect();
            self.execute_in_tx(queries).await?;
        }
        Ok(())
    }
    async fn execute_in_tx(&self, queries: Vec<(&str, Params)>) -> Result<(), RollupStoreError> {
        self.execute("BEGIN TRANSACTION", ()).await?;
        for (query, params) in queries {
            self.execute(query, params).await?;
        }
        self.execute("COMMIT TRANSACTION", ()).await?;
        Ok(())
    }
}

fn read_from_row_int(row: &Row, index: i32) -> Result<u64, RollupStoreError> {
    match row.get_value(index)? {
        Value::Integer(i) => {
            let val = i
                .try_into()
                .map_err(|e| RollupStoreError::Custom(format!("conversion error: {e}")))?;
            Ok(val)
        }
        _ => Err(RollupStoreError::SQLInvalidTypeError),
    }
}

fn read_from_row_blob(row: &Row, index: i32) -> Result<Vec<u8>, RollupStoreError> {
    match row.get_value(index)? {
        Value::Blob(vec) => Ok(vec),
        _ => Err(RollupStoreError::SQLInvalidTypeError),
    }
}

#[async_trait::async_trait]
impl StoreEngineRollup for SQLStore {
    async fn get_batch_number_by_block(
        &self,
        block_number: BlockNumber,
    ) -> Result<Option<u64>, RollupStoreError> {
        let mut rows = self
            .query(
                "SELECT * from blocks WHERE block_number = ?1",
                vec![block_number],
            )
            .await?;
        if let Some(row) = rows.next().await? {
            return Ok(Some(read_from_row_int(&row, 1)?));
        }
        Ok(None)
    }

    /// Stores the batch number by a given block number.
    async fn store_batch_number_by_block(
        &self,
        block_number: BlockNumber,
        batch_number: u64,
    ) -> Result<(), RollupStoreError> {
        self.execute_in_tx(vec![
            (
                "DELETE FROM blocks WHERE block_number = ?1",
                vec![block_number].into_params()?,
            ),
            (
                "INSERT INTO blocks VALUES (?1, ?2)",
                vec![block_number, batch_number].into_params()?,
            ),
        ])
        .await
    }

    /// Gets the message hashes by a given batch number.
    async fn get_message_hashes_by_batch(
        &self,
        batch_number: u64,
    ) -> Result<Option<Vec<H256>>, RollupStoreError> {
        let mut hashes = vec![];
        let mut rows = self
            .query(
                "SELECT * from messages WHERE batch = ?1 ORDER BY idx ASC",
                vec![batch_number],
            )
            .await?;
        while let Some(row) = rows.next().await? {
            let vec = read_from_row_blob(&row, 2)?;
            hashes.push(H256::from_slice(&vec));
        }
        if hashes.is_empty() {
            Ok(None)
        } else {
            Ok(Some(hashes))
        }
    }

    /// Stores the withdrawal hashes by a given batch number.
    async fn store_message_hashes_by_batch(
        &self,
        batch_number: u64,
        message_hashes: Vec<H256>,
    ) -> Result<(), RollupStoreError> {
        let mut queries = vec![(
            "DELETE FROM messages WHERE batch = ?1",
            vec![batch_number].into_params()?,
        )];
        for (index, hash) in message_hashes.iter().enumerate() {
            let index = u64::try_from(index)
                .map_err(|e| RollupStoreError::Custom(format!("conversion error: {e}")))?;
            queries.push((
                "INSERT INTO messages VALUES (?1, ?2, ?3)",
                (batch_number, index, Vec::from(hash.to_fixed_bytes())).into_params()?,
            ));
        }
        self.execute_in_tx(queries).await?;
        Ok(())
    }

    /// Stores the block numbers by a given batch_number
    async fn store_block_numbers_by_batch(
        &self,
        batch_number: u64,
        block_numbers: Vec<BlockNumber>,
    ) -> Result<(), RollupStoreError> {
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
    ) -> Result<Option<Vec<BlockNumber>>, RollupStoreError> {
        let mut blocks = Vec::new();
        let mut rows = self
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
    ) -> Result<(), RollupStoreError> {
        self.execute_in_tx(vec![
            (
                "DELETE FROM deposits WHERE batch = ?1",
                vec![batch_number].into_params()?,
            ),
            (
                "INSERT INTO deposits VALUES (?1, ?2)",
                (batch_number, Vec::from(deposit_logs_hash.to_fixed_bytes())).into_params()?,
            ),
        ])
        .await
    }

    async fn get_deposit_logs_hash_by_batch_number(
        &self,
        batch_number: u64,
    ) -> Result<Option<H256>, RollupStoreError> {
        let mut rows = self
            .query(
                "SELECT * from deposits WHERE batch = ?1",
                vec![batch_number],
            )
            .await?;
        if let Some(row) = rows.next().await? {
            let vec = read_from_row_blob(&row, 1)?;
            return Ok(Some(H256::from_slice(&vec)));
        }
        Ok(None)
    }

    async fn store_state_root_by_batch_number(
        &self,
        batch_number: u64,
        state_root: H256,
    ) -> Result<(), RollupStoreError> {
        self.execute_in_tx(vec![
            (
                "DELETE FROM state_roots WHERE batch = ?1",
                vec![batch_number].into_params()?,
            ),
            (
                "INSERT INTO state_roots VALUES (?1, ?2)",
                (batch_number, Vec::from(state_root.to_fixed_bytes())).into_params()?,
            ),
        ])
        .await
    }

    async fn get_state_root_by_batch_number(
        &self,
        batch_number: u64,
    ) -> Result<Option<H256>, RollupStoreError> {
        let mut rows = self
            .query(
                "SELECT * FROM state_roots WHERE batch = ?1",
                vec![batch_number],
            )
            .await?;
        if let Some(row) = rows.next().await? {
            let vec = read_from_row_blob(&row, 1)?;
            return Ok(Some(H256::from_slice(&vec)));
        }
        Ok(None)
    }

    async fn store_blob_bundle_by_batch_number(
        &self,
        batch_number: u64,
        state_diff: Vec<Blob>,
    ) -> Result<(), RollupStoreError> {
        let mut queries = vec![(
            "DELETE FROM blob_bundles WHERE batch = ?1",
            vec![batch_number].into_params()?,
        )];
        for (index, blob) in state_diff.iter().enumerate() {
            let index = u64::try_from(index)
                .map_err(|e| RollupStoreError::Custom(format!("conversion error: {e}")))?;
            queries.push((
                "INSERT INTO blob_bundles VALUES (?1, ?2, ?3)",
                (batch_number, index, blob.to_vec()).into_params()?,
            ));
        }
        self.execute_in_tx(queries).await?;
        Ok(())
    }

    async fn get_blob_bundle_by_batch_number(
        &self,
        batch_number: u64,
    ) -> Result<Option<Vec<Blob>>, RollupStoreError> {
        let mut bundles = Vec::new();
        let mut rows = self
            .query(
                "SELECT * FROM blob_bundles WHERE batch = ?1 ORDER BY idx ASC",
                vec![batch_number],
            )
            .await?;
        while let Some(row) = rows.next().await? {
            let val = read_from_row_blob(&row, 2)?;
            bundles.push(
                Blob::try_from(val).map_err(|_| {
                    RollupStoreError::Custom("error converting to Blob".to_string())
                })?,
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
        messages_inc: u64,
    ) -> Result<(), RollupStoreError> {
        self.execute(
            "UPDATE operation_count SET transactions = transactions + ?1, deposits = deposits + ?2, messages = withdrawals + ?3", 
            (transaction_inc, deposits_inc, messages_inc)).await?;
        Ok(())
    }

    async fn get_operations_count(&self) -> Result<[u64; 3], RollupStoreError> {
        let mut rows = self.query("SELECT * from operation_count", ()).await?;
        if let Some(row) = rows.next().await? {
            return Ok([
                read_from_row_int(&row, 1)?,
                read_from_row_int(&row, 2)?,
                read_from_row_int(&row, 3)?,
            ]);
        }
        Err(RollupStoreError::Custom(
            "missing operation_count row".to_string(),
        ))
    }

    /// Returns whether the batch with the given number is present.
    async fn contains_batch(&self, batch_number: &u64) -> Result<bool, RollupStoreError> {
        let mut row = self
            .query("SELECT * from blocks WHERE batch = ?1", vec![*batch_number])
            .await?;
        Ok(row.next().await?.is_some())
    }

    async fn get_lastest_sent_batch_proof(&self) -> Result<u64, RollupStoreError> {
        let mut rows = self.query("SELECT * from latest_sent", ()).await?;
        if let Some(row) = rows.next().await? {
            return read_from_row_int(&row, 1);
        }
        Err(RollupStoreError::Custom(
            "missing operation_count row".to_string(),
        ))
    }

    async fn set_lastest_sent_batch_proof(
        &self,
        batch_number: u64,
    ) -> Result<(), RollupStoreError> {
        self.execute("UPDATE latest_sent SET batch = ?1", (0, batch_number))
            .await?;
        Ok(())
    }

    async fn get_account_updates_by_block_number(
        &self,
        block_number: BlockNumber,
    ) -> Result<Option<Vec<AccountUpdate>>, RollupStoreError> {
        let mut rows = self
            .query(
                "SELECT * FROM account_updates WHERE block_number = ?1",
                vec![block_number],
            )
            .await?;
        if let Some(row) = rows.next().await? {
            let vec = read_from_row_blob(&row, 1)?;
            return Ok(Some(bincode::deserialize(&vec)?));
        }
        Ok(None)
    }

    async fn store_account_updates_by_block_number(
        &self,
        block_number: BlockNumber,
        account_updates: Vec<AccountUpdate>,
    ) -> Result<(), RollupStoreError> {
        let serialized = bincode::serialize(&account_updates)?;
        self.execute_in_tx(vec![
            (
                "DELETE FROM account_updates WHERE block_number = ?1",
                vec![block_number].into_params()?,
            ),
            (
                "INSERT INTO account_updates VALUES (?1, ?2)",
                (block_number, serialized).into_params()?,
            ),
        ])
        .await
    }

    async fn revert_to_batch(&self, batch_number: u64) -> Result<(), RollupStoreError> {
        self.execute_in_tx(vec![
            (
                "DELETE FROM blocks WHERE batch > ?1",
                [batch_number].into_params()?,
            ),
            (
                "DELETE FROM messages WHERE batch > ?1",
                [batch_number].into_params()?,
            ),
            (
                "DELETE FROM deposits WHERE batch > ?1",
                [batch_number].into_params()?,
            ),
            (
                "DELETE FROM state_roots WHERE batch > ?1",
                [batch_number].into_params()?,
            ),
            (
                "DELETE FROM blob_bundles WHERE batch > ?1",
                [batch_number].into_params()?,
            ),
        ])
        .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_schema_tables() -> anyhow::Result<()> {
        let store = SQLStore::new(":memory:")?;
        let tables = [
            "blocks",
            "messages",
            "deposits",
            "state_roots",
            "blob_bundles",
            "account_updates",
            "operation_count",
            "latest_sent",
        ];
        let mut attributes = Vec::new();
        for table in tables {
            let mut rows = store
                .query(format!("PRAGMA table_info({table})").as_str(), ())
                .await?;
            while let Some(row) = rows.next().await? {
                // (table, name, type)
                attributes.push((
                    table.to_string(),
                    row.get_str(1)?.to_string(),
                    row.get_str(2)?.to_string(),
                ))
            }
        }
        for (table, name, given_type) in attributes {
            let expected_type = match (table.as_str(), name.as_str()) {
                ("blocks", "block_number") => "INT",
                ("blocks", "batch") => "INT",
                ("messages", "batch") => "INT",
                ("messages", "idx") => "INT",
                ("messages", "message_hash") => "BLOB",
                ("deposits", "batch") => "INT",
                ("deposits", "deposit_hash") => "BLOB",
                ("state_roots", "batch") => "INT",
                ("state_roots", "state_root") => "BLOB",
                ("blob_bundles", "batch") => "INT",
                ("blob_bundles", "idx") => "INT",
                ("blob_bundles", "blob_bundle") => "BLOB",
                ("account_updates", "block_number") => "INT",
                ("account_updates", "updates") => "BLOB",
                ("operation_count", "_id") => "INT",
                ("operation_count", "transactions") => "INT",
                ("operation_count", "deposits") => "INT",
                ("operation_count", "messages") => "INT",
                ("latest_sent", "_id") => "INT",
                ("latest_sent", "batch") => "INT",
                _ => {
                    return Err(anyhow::Error::msg(
                        "unexpected attribute {name} in table {table}",
                    ));
                }
            };
            assert_eq!(given_type, expected_type);
        }
        Ok(())
    }
}
