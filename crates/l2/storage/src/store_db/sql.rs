use std::{fmt::Debug, ops::Range, sync::Arc, time::Duration};
use tokio::sync::Mutex;

use crate::{RollupStoreError, api::StoreEngineRollup};
use ethrex_common::{
    H256,
    types::{AccountUpdate, Blob, BlockNumber, batch::Batch},
};
use ethrex_l2_common::prover::{BatchProof, ProverType};

use libsql::{
    Builder, Connection, Row, Rows, Transaction, Value,
    params::{IntoParams, IntoValue, Params},
};

/// ### SQLStore
/// - `read_conn`: a connection to the database to be used for read only statements
/// - `write_conn`: a connection to the database to be used for writing, protected by a Mutex to enforce a maximum of 1 writer.
///   If writes are done using the read only connection `SQLite failure: database is locked` problems will arise
pub struct SQLStore {
    read_conn: Connection,
    write_conn: Arc<Mutex<Connection>>,
}

impl Debug for SQLStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SQLStore")
    }
}

// Change version if the DB_SCHEMA changes
const MIGRATION_VERSION: u64 = 1;
const DB_SCHEMA: [&str; 10] = [
    "CREATE TABLE batches (number INT PRIMARY KEY, first_block INT NOT NULL, last_block INT NOT NULL, privileged_transactions_hash BLOB, state_root BLOB NOT NULL, commit_tx BLOB, verify_tx BLOB, signature BLOB)",
    "CREATE TABLE messages (batch INT, idx INT, message_hash BLOB, PRIMARY KEY (batch, idx))",
    "CREATE TABLE blob_bundles (batch INT, idx INT, blob_bundle BLOB, PRIMARY KEY (batch, idx))",
    "CREATE TABLE account_updates (block_number INT PRIMARY KEY, updates BLOB)",
    "CREATE TABLE batch_proofs (batch INT, prover_type INT, proof BLOB, PRIMARY KEY (batch, prover_type))",
    "CREATE TABLE block_signatures (block_hash BLOB PRIMARY KEY, signature BLOB)",
    "CREATE TABLE precommit_privileged (start INT, end INT)",
    "CREATE TABLE operation_count (transactions INT, privileged_transactions INT, messages INT)",
    "INSERT INTO operation_count VALUES (0, 0, 0)",
    "CREATE TABLE migrations (version INT PRIMARY KEY)",
];

impl SQLStore {
    pub fn new(path: &str) -> Result<Self, RollupStoreError> {
        futures::executor::block_on(async {
            let db = Builder::new_local(path).build().await?;
            let write_conn = db.connect()?;
            // From libsql documentation:
            // Newly created connections currently have a default busy timeout of
            // 5000ms, but this may be subject to change.
            write_conn.busy_timeout(Duration::from_millis(5000))?;
            let store = SQLStore {
                read_conn: db.connect()?,
                write_conn: Arc::new(Mutex::new(write_conn)),
            };

            let current_version = store.get_version().await?;
            if current_version != MIGRATION_VERSION {
                return Err(RollupStoreError::VersionMismatch {
                    current: current_version,
                    expected: MIGRATION_VERSION,
                });
            }

            store.init_db().await?;
            Ok(store)
        })
    }

    async fn execute<T: IntoParams>(&self, sql: &str, params: T) -> Result<(), RollupStoreError> {
        let conn = self.write_conn.lock().await;
        conn.execute(sql, params).await?;
        Ok(())
    }

    async fn query<T: IntoParams>(&self, sql: &str, params: T) -> Result<Rows, RollupStoreError> {
        Ok(self.read_conn.query(sql, params).await?)
    }

    async fn init_db(&self) -> Result<(), RollupStoreError> {
        // We use WAL for better concurrency
        // "readers do not block writers and a writer does not block readers. Reading and writing can proceed concurrently"
        // https://sqlite.org/wal.html#concurrency
        // still a limit of only 1 writer is imposed by sqlite databases
        self.query("PRAGMA journal_mode=WAL;", ()).await?;
        let mut rows = self
            .query(
                "SELECT name FROM sqlite_schema WHERE type='table' AND name='batches'",
                (),
            )
            .await?;
        if rows.next().await?.is_none() {
            let empty_param = ().into_params()?;
            let queries = DB_SCHEMA
                .iter()
                .map(|v| (*v, empty_param.clone()))
                .collect();
            self.execute_in_tx(queries, None).await?;
        }
        Ok(())
    }

    /// Executes a set of queries in a SQL transaction
    /// if the db_tx parameter is Some then it uses that transaction and does not commit to the DB after execution
    /// if the db_tx parameter is None then it creates a transaction and commits to the DB after execution
    async fn execute_in_tx(
        &self,
        queries: Vec<(&str, Params)>,
        db_tx: Option<&Transaction>,
    ) -> Result<(), RollupStoreError> {
        if let Some(existing_tx) = db_tx {
            for (query, params) in queries {
                existing_tx.execute(query, params).await?;
            }
        } else {
            let conn = self.write_conn.lock().await;
            let tx = conn.transaction().await?;
            for (query, params) in queries {
                tx.execute(query, params).await?;
            }
            tx.commit().await?;
        }
        Ok(())
    }

    async fn store_message_hashes_by_batch_in_tx(
        &self,
        batch_number: u64,
        message_hashes: Vec<H256>,
        db_tx: Option<&Transaction>,
    ) -> Result<(), RollupStoreError> {
        let mut queries = vec![];
        for (index, hash) in message_hashes.iter().enumerate() {
            let index = u64::try_from(index)
                .map_err(|e| RollupStoreError::Custom(format!("conversion error: {e}")))?;
            queries.push((
                "INSERT OR REPLACE INTO messages (batch, idx, message_hash) VALUES (?1, ?2, ?3)",
                (batch_number, index, Vec::from(hash.to_fixed_bytes())).into_params()?,
            ));
        }
        self.execute_in_tx(queries, db_tx).await
    }

    async fn store_blob_bundle_by_batch_number_in_tx(
        &self,
        batch_number: u64,
        state_diff: Vec<Blob>,
        db_tx: Option<&Transaction>,
    ) -> Result<(), RollupStoreError> {
        let mut queries = vec![];
        for (index, blob) in state_diff.iter().enumerate() {
            let index = u64::try_from(index)
                .map_err(|e| RollupStoreError::Custom(format!("conversion error: {e}")))?;
            queries.push((
                "INSERT OR REPLACE INTO blob_bundles (batch, idx, blob_bundle) VALUES (?1, ?2, ?3)",
                (batch_number, index, blob.to_vec()).into_params()?,
            ));
        }
        self.execute_in_tx(queries, db_tx).await
    }

    async fn store_account_updates_by_block_number_in_tx(
        &self,
        block_number: BlockNumber,
        account_updates: Vec<AccountUpdate>,
        db_tx: Option<&Transaction>,
    ) -> Result<(), RollupStoreError> {
        let serialized = bincode::serialize(&account_updates)?;
        let query = vec![(
            "INSERT OR REPLACE INTO account_updates (block_number, updates) VALUES (?1, ?2)",
            (block_number, serialized).into_params()?,
        )];
        self.execute_in_tx(query, db_tx).await
    }

    async fn get_version(&self) -> Result<u64, RollupStoreError> {
        let mut rows = self
            .query("SELECT MAX(version) FROM migrations", ())
            .await?;
        rows.next()
            .await?
            .map(|row| read_from_row_int(&row, 0))
            .ok_or(RollupStoreError::Custom(
                "Migration version not found".to_string(),
            ))?
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
                "SELECT number FROM batches WHERE first_block <= ?1 AND last_block >= ?1",
                vec![block_number],
            )
            .await?;

        rows.next()
            .await?
            .map(|row| read_from_row_int(&row, 0))
            .transpose()
    }

    /// Gets the message hashes by a given batch number.
    async fn get_message_hashes_by_batch(
        &self,
        batch_number: u64,
    ) -> Result<Option<Vec<H256>>, RollupStoreError> {
        let mut hashes = vec![];
        let mut rows = self
            .query(
                "SELECT message_hash FROM messages WHERE batch = ?1 ORDER BY idx ASC",
                vec![batch_number],
            )
            .await?;
        while let Some(row) = rows.next().await? {
            let vec = read_from_row_blob(&row, 0)?;
            hashes.push(H256::from_slice(&vec));
        }
        if hashes.is_empty() {
            Ok(None)
        } else {
            Ok(Some(hashes))
        }
    }

    /// Returns the block numbers by a given batch_number
    async fn get_block_numbers_by_batch(
        &self,
        batch_number: u64,
    ) -> Result<Option<(BlockNumber, BlockNumber)>, RollupStoreError> {
        let mut rows = self
            .query(
                "SELECT first_block, last_block FROM batches WHERE number = ?1",
                vec![batch_number],
            )
            .await?;

        let Some(row) = rows.next().await? else {
            return Ok(None);
        };

        Ok(Some((
            read_from_row_int(&row, 0)?,
            read_from_row_int(&row, 1)?,
        )))
    }

    async fn get_privileged_transactions_hash_by_batch_number(
        &self,
        batch_number: u64,
    ) -> Result<Option<H256>, RollupStoreError> {
        let mut rows = self
            .query(
                "SELECT privileged_transactions_hash FROM batches WHERE number = ?1",
                vec![batch_number],
            )
            .await?;

        let Some(row) = rows.next().await? else {
            return Ok(None);
        };

        let vec = read_from_row_blob(&row, 0)?;
        Ok(Some(H256::from_slice(&vec)))
    }

    async fn get_state_root_by_batch_number(
        &self,
        batch_number: u64,
    ) -> Result<Option<H256>, RollupStoreError> {
        let mut rows = self
            .query(
                "SELECT state_root FROM batches WHERE number = ?1",
                vec![batch_number],
            )
            .await?;

        let Some(row) = rows.next().await? else {
            return Ok(None);
        };

        let vec = read_from_row_blob(&row, 0)?;
        Ok(Some(H256::from_slice(&vec)))
    }

    async fn get_blob_bundle_by_batch_number(
        &self,
        batch_number: u64,
    ) -> Result<Option<Vec<Blob>>, RollupStoreError> {
        let mut bundles = Vec::new();
        let mut rows = self
            .query(
                "SELECT blob_bundle FROM blob_bundles WHERE batch = ?1 ORDER BY idx ASC",
                vec![batch_number],
            )
            .await?;
        while let Some(row) = rows.next().await? {
            let val = read_from_row_blob(&row, 0)?;
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

    async fn store_commit_tx_by_batch(
        &self,
        batch_number: u64,
        commit_tx: H256,
    ) -> Result<(), RollupStoreError> {
        self.execute(
            "UPDATE batches SET commit_tx = ?1 WHERE number = ?2",
            (Vec::from(commit_tx.to_fixed_bytes()), batch_number),
        )
        .await
    }

    async fn get_commit_tx_by_batch(
        &self,
        batch_number: u64,
    ) -> Result<Option<H256>, RollupStoreError> {
        let mut rows = self
            .query(
                "SELECT commit_tx FROM batches WHERE number = ?1",
                vec![batch_number],
            )
            .await?;

        let Some(row) = rows.next().await? else {
            return Ok(None);
        };

        // If commit_tx is NULL
        let Ok(vec) = read_from_row_blob(&row, 0) else {
            return Ok(None);
        };

        Ok(Some(H256::from_slice(&vec)))
    }

    async fn store_verify_tx_by_batch(
        &self,
        batch_number: u64,
        verify_tx: H256,
    ) -> Result<(), RollupStoreError> {
        self.execute(
            "UPDATE batches SET verify_tx = ?1 WHERE number = ?2",
            (Vec::from(verify_tx.to_fixed_bytes()), batch_number),
        )
        .await
    }

    async fn get_verify_tx_by_batch(
        &self,
        batch_number: u64,
    ) -> Result<Option<H256>, RollupStoreError> {
        let mut rows = self
            .query(
                "SELECT verify_tx FROM batches WHERE number = ?1",
                vec![batch_number],
            )
            .await?;

        let Some(row) = rows.next().await? else {
            return Ok(None);
        };

        // If verify_tx is NULL
        let Ok(vec) = read_from_row_blob(&row, 0) else {
            return Ok(None);
        };

        Ok(Some(H256::from_slice(&vec)))
    }

    async fn update_operations_count(
        &self,
        transaction_inc: u64,
        privileged_transactions_inc: u64,
        messages_inc: u64,
    ) -> Result<(), RollupStoreError> {
        self.execute(
            "UPDATE operation_count SET transactions = transactions + ?1, privileged_transactions = privileged_transactions + ?2, messages = messages + ?3",
            (transaction_inc, privileged_transactions_inc, messages_inc)).await?;
        Ok(())
    }

    async fn get_operations_count(&self) -> Result<[u64; 3], RollupStoreError> {
        let mut rows = self.query("SELECT * from operation_count", ()).await?;
        if let Some(row) = rows.next().await? {
            return Ok([
                read_from_row_int(&row, 0)?,
                read_from_row_int(&row, 1)?,
                read_from_row_int(&row, 2)?,
            ]);
        }
        Err(RollupStoreError::Custom(
            "missing operation_count row".to_string(),
        ))
    }

    /// Returns whether the batch with the given number is present.
    async fn contains_batch(&self, batch_number: &u64) -> Result<bool, RollupStoreError> {
        let mut row = self
            .query(
                "SELECT number FROM batches WHERE number = ?1",
                vec![*batch_number],
            )
            .await?;
        Ok(row.next().await?.is_some())
    }

    async fn get_lastest_sent_batch_proof(&self) -> Result<u64, RollupStoreError> {
        let mut rows = self
            .query(
                "SELECT MAX(number) FROM batches WHERE verify_tx IS NOT NULL",
                (),
            )
            .await?;

        let Some(row) = rows.next().await? else {
            return Ok(0);
        };

        read_from_row_int(&row, 0)
    }

    async fn get_account_updates_by_block_number(
        &self,
        block_number: BlockNumber,
    ) -> Result<Option<Vec<AccountUpdate>>, RollupStoreError> {
        let mut rows = self
            .query(
                "SELECT updates FROM account_updates WHERE block_number = ?1",
                vec![block_number],
            )
            .await?;

        let Some(row) = rows.next().await? else {
            return Ok(None);
        };

        let vec = read_from_row_blob(&row, 0)?;
        Ok(Some(bincode::deserialize(&vec)?))
    }

    async fn store_account_updates_by_block_number(
        &self,
        block_number: BlockNumber,
        account_updates: Vec<AccountUpdate>,
    ) -> Result<(), RollupStoreError> {
        self.store_account_updates_by_block_number_in_tx(block_number, account_updates, None)
            .await
    }

    async fn revert_to_batch(&self, batch_number: u64) -> Result<(), RollupStoreError> {
        let queries = vec![
            (
                "DELETE FROM batches WHERE batch > ?1",
                [batch_number].into_params()?,
            ),
            (
                "DELETE FROM messages WHERE batch > ?1",
                [batch_number].into_params()?,
            ),
            (
                "DELETE FROM blob_bundles WHERE batch > ?1",
                [batch_number].into_params()?,
            ),
            (
                "DELETE FROM batch_proofs WHERE batch > ?1",
                [batch_number].into_params()?,
            ),
            ("DELETE FROM precommit_privileged", Params::None),
        ];
        self.execute_in_tx(queries, None).await
    }

    async fn store_proof_by_batch_and_type(
        &self,
        batch_number: u64,
        prover_type: ProverType,
        proof: BatchProof,
    ) -> Result<(), RollupStoreError> {
        let serialized_proof = bincode::serialize(&proof)?;
        let prover_type: u32 = prover_type.into();
        self.execute(
            "INSERT OR REPLACE INTO batch_proofs (batch, prover_type, proof) VALUES (?1, ?2, ?3)",
            (batch_number, prover_type, serialized_proof).into_params()?,
        )
        .await
    }

    async fn get_proof_by_batch_and_type(
        &self,
        batch_number: u64,
        prover_type: ProverType,
    ) -> Result<Option<BatchProof>, RollupStoreError> {
        let prover_type: u32 = prover_type.into();
        let mut rows = self
            .query(
                "SELECT proof FROM batch_proofs WHERE batch = ?1 AND prover_type = ?2",
                (batch_number, prover_type),
            )
            .await?;

        let Some(row) = rows.next().await? else {
            return Ok(None);
        };

        let vec = read_from_row_blob(&row, 0)?;
        Ok(Some(bincode::deserialize(&vec)?))
    }

    async fn seal_batch(&self, batch: Batch) -> Result<(), RollupStoreError> {
        let conn = self.write_conn.lock().await;
        let transaction = conn.transaction().await?;

        let commit_tx = match batch.commit_tx {
            Some(hash) => Vec::from(hash.to_fixed_bytes()).into_value()?,
            None => Value::Null,
        };

        let verify_tx = match batch.verify_tx {
            Some(hash) => Vec::from(hash.to_fixed_bytes()).into_value()?,
            None => Value::Null,
        };

        self.execute_in_tx(vec![(
            "INSERT INTO batches (number, first_block, last_block, privileged_transactions_hash, state_root, commit_tx, verify_tx) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            (
                batch.number,
                batch.first_block,
                batch.last_block,
                Vec::from(batch.privileged_transactions_hash.to_fixed_bytes()).into_value()?,
                Vec::from(batch.state_root.to_fixed_bytes()).into_value()?,
                commit_tx,
                verify_tx,
            ).into_params()?
        )], Some(&transaction)).await?;
        self.store_message_hashes_by_batch_in_tx(
            batch.number,
            batch.message_hashes,
            Some(&transaction),
        )
        .await?;
        self.store_blob_bundle_by_batch_number_in_tx(
            batch.number,
            batch.blobs_bundle.blobs,
            Some(&transaction),
        )
        .await?;

        transaction.commit().await.map_err(RollupStoreError::from)
    }

    async fn store_signature_by_block(
        &self,
        block_hash: H256,
        signature: ethereum_types::Signature,
    ) -> Result<(), RollupStoreError> {
        self.execute_in_tx(
            vec![
                (
                    "DELETE FROM block_signatures WHERE block_hash = ?1",
                    vec![Vec::from(block_hash.to_fixed_bytes())].into_params()?,
                ),
                (
                    "INSERT INTO block_signatures VALUES (?1, ?2)",
                    (
                        Vec::from(block_hash.to_fixed_bytes()),
                        Vec::from(signature.as_fixed_bytes()),
                    )
                        .into_params()?,
                ),
            ],
            None,
        )
        .await
    }

    async fn get_signature_by_block(
        &self,
        block_hash: H256,
    ) -> Result<Option<ethereum_types::Signature>, RollupStoreError> {
        let mut rows = self
            .query(
                "SELECT signature FROM block_signatures WHERE block_hash = ?1",
                vec![Vec::from(block_hash.to_fixed_bytes())],
            )
            .await?;
        rows.next()
            .await?
            .map(|row| {
                read_from_row_blob(&row, 0)
                    .map(|vec| ethereum_types::Signature::from_slice(vec.as_slice()))
            })
            .transpose()
    }

    async fn store_signature_by_batch(
        &self,
        batch_number: u64,
        signature: ethereum_types::Signature,
    ) -> Result<(), RollupStoreError> {
        self.execute(
            "UPDATE batches SET signature=?1 WHERE batch = ?2)",
            (Vec::from(signature.to_fixed_bytes()), batch_number).into_params()?,
        )
        .await
    }

    async fn get_signature_by_batch(
        &self,
        batch_number: u64,
    ) -> Result<Option<ethereum_types::Signature>, RollupStoreError> {
        let mut rows = self
            .query(
                "SELECT signature FROM batches WHERE batch = ?1",
                vec![batch_number],
            )
            .await?;
        rows.next()
            .await?
            .map(|row| {
                read_from_row_blob(&row, 0)
                    .map(|vec| ethereum_types::Signature::from_slice(vec.as_slice()))
            })
            .transpose()
    }

    async fn delete_proof_by_batch_and_type(
        &self,
        batch_number: u64,
        proof_type: ProverType,
    ) -> Result<(), RollupStoreError> {
        let prover_type: u32 = proof_type.into();
        self.execute_in_tx(
            vec![(
                "DELETE FROM batch_proofs WHERE batch = ?1 AND prover_type = ?2",
                (batch_number, prover_type).into_params()?,
            )],
            None,
        )
        .await
    }

    async fn precommit_privileged(&self) -> Result<Option<Range<u64>>, RollupStoreError> {
        let mut rows = self.query("SELECT * from precommit_privileged", ()).await?;
        if let Some(row) = rows.next().await? {
            let start = read_from_row_int(&row, 0)?;
            let end = read_from_row_int(&row, 1)?;
            return Ok(Some(start..end));
        }
        Ok(None)
    }

    async fn update_precommit_privileged(
        &self,
        range: Option<Range<u64>>,
    ) -> Result<(), RollupStoreError> {
        let mut queries = vec![("DELETE FROM precommit_privileged", ().into_params()?)];
        if let Some(range) = range {
            queries.push((
                "INSERT INTO precommit_privileged VALUES (?1, ?2)",
                (range.start, range.end).into_params()?,
            ));
        }
        self.execute_in_tx(queries, None).await
    }

    async fn get_last_batch_number(&self) -> Result<Option<u64>, RollupStoreError> {
        let mut rows = self.query("SELECT MAX(number) FROM batches", ()).await?;
        rows.next()
            .await?
            .map(|row| read_from_row_int(&row, 0))
            .transpose()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_schema_tables() -> anyhow::Result<()> {
        let store = SQLStore::new(":memory:")?;
        let tables = [
            "batches",
            "messages",
            "blob_bundles",
            "account_updates",
            "operation_count",
            "batch_proofs",
            "block_signatures",
            "precommit_privileged",
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
                ("batches", "number") => "INT",
                ("batches", "first_block") => "INT",
                ("batches", "last_block") => "INT",
                ("batches", "privileged_transactions_hash") => "BLOB",
                ("batches", "state_root") => "BLOB",
                ("batches", "commit_tx") => "BLOB",
                ("batches", "verify_tx") => "BLOB",
                ("batches", "signature") => "BLOB",
                ("messages", "batch") => "INT",
                ("messages", "idx") => "INT",
                ("messages", "message_hash") => "BLOB",
                ("blob_bundles", "batch") => "INT",
                ("blob_bundles", "idx") => "INT",
                ("blob_bundles", "blob_bundle") => "BLOB",
                ("account_updates", "block_number") => "INT",
                ("account_updates", "updates") => "BLOB",
                ("operation_count", "transactions") => "INT",
                ("operation_count", "privileged_transactions") => "INT",
                ("operation_count", "messages") => "INT",
                ("batch_proofs", "batch") => "INT",
                ("batch_proofs", "prover_type") => "INT",
                ("batch_proofs", "proof") => "BLOB",
                ("block_signatures", "block_hash") => "BLOB",
                ("block_signatures", "signature") => "BLOB",
                ("precommit_privileged", "start") => "INT",
                ("precommit_privileged", "end") => "INT",
                ("migrations", "version") => "INT",
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
