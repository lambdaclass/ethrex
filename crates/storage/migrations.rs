use std::io::Write;
use std::path::Path;

use crate::api::StorageBackend;
use crate::api::tables::RECEIPTS;
use crate::error::StoreError;
use crate::store::receipt_key;
use crate::{STORE_METADATA_FILENAME, STORE_SCHEMA_VERSION};

use ethrex_common::H256;
use ethrex_rlp::decode::RLPDecode;

use super::store::StoreMetadata;

/// A migration function that upgrades the database schema by one version.
///
/// Receives a reference to the storage backend and the database directory
/// path (for temporary files, etc.).
pub type MigrationFn = fn(backend: &dyn StorageBackend, db_path: &Path) -> Result<(), StoreError>;

/// Migration functions indexed by source version.
///
/// `MIGRATIONS[i]` upgrades the schema from version `(i + 1)` to `(i + 2)`.
/// For example:
/// - `MIGRATIONS[0]` upgrades v1 → v2
/// - `MIGRATIONS[1]` upgrades v2 → v3
///
/// **Invariant**: `MIGRATIONS.len() == (STORE_SCHEMA_VERSION - 1) as usize`
/// (empty when `STORE_SCHEMA_VERSION == 1`, one entry when it's 2, etc.)
pub const MIGRATIONS: &[MigrationFn] = &[migrate_1_to_2];

// Compile-time check: the number of migration functions must match the number
// of version gaps (i.e. STORE_SCHEMA_VERSION - 1).
const _: () = assert!(
    MIGRATIONS.len() == (STORE_SCHEMA_VERSION - 1) as usize,
    "MIGRATIONS length must equal STORE_SCHEMA_VERSION - 1"
);

/// Returns the migration function that upgrades from `version` to `version + 1`.
fn migration_for_version(version: u64) -> MigrationFn {
    MIGRATIONS[(version - 1) as usize]
}

/// Runs all pending migrations from `current_version` up to `STORE_SCHEMA_VERSION`.
///
/// Each migration is applied one version at a time, and the metadata file is
/// updated (with fsync) after each successful step for crash safety.
///
/// Returns `Ok(())` if `current_version == STORE_SCHEMA_VERSION` (no-op).
pub fn run_pending_migrations(
    backend: &dyn StorageBackend,
    db_path: &Path,
    current_version: u64,
) -> Result<(), StoreError> {
    for version in current_version..STORE_SCHEMA_VERSION {
        let target = version + 1;

        tracing::info!("Running migration v{version} → v{target}");

        migration_for_version(version)(backend, db_path).map_err(|e| StoreError::MigrationFailed {
            from: version,
            to: target,
            reason: e.to_string(),
        })?;

        // Persist the new version to metadata.json after each migration step
        write_metadata_version(db_path, target).map_err(|e| StoreError::MigrationFailed {
            from: version,
            to: target,
            reason: format!("failed to write metadata: {e}"),
        })?;

        tracing::info!("Migration v{version} → v{target} completed");
    }

    Ok(())
}

/// Writes the schema version to metadata.json using write-to-temp-then-rename
/// for crash safety. On POSIX filesystems `rename` is atomic, so the metadata
/// file is never left in a partial/truncated state.
// TODO: move metadata persistence into the StorageBackend abstraction so we
// don't need to pass `db_path` around.
fn write_metadata_version(db_path: &Path, version: u64) -> Result<(), StoreError> {
    let metadata_path = db_path.join(STORE_METADATA_FILENAME);
    let tmp_path = db_path.join(format!("{}.tmp", STORE_METADATA_FILENAME));
    let metadata = StoreMetadata::new(version);
    let serialized = serde_json::to_string_pretty(&metadata)?;

    let mut file = std::fs::File::create(&tmp_path)?;
    file.write_all(serialized.as_bytes())?;
    file.sync_all()?;
    std::fs::rename(&tmp_path, &metadata_path)?;

    Ok(())
}

/// Migrates the RECEIPTS table key format from RLP-encoded `(BlockHash, u64)`
/// to raw `block_hash (32B) || index (8B big-endian u64)`.
///
/// This enables efficient cursor-based prefix iteration by block hash.
///
/// The migration works in two phases to avoid both holding a read iterator
/// open during writes (snapshot semantics concern) and materializing all
/// entries in memory (153M+ entries ≈ 13 GB):
///
/// 1. Cursor scan dumps old-format keys to a temporary file, then closes
///    the iterator immediately.
/// 2. Keys are read back from the file in batches; each batch does point
///    lookups for values, writes new keys, and deletes old keys.
///
/// Crash safety: if interrupted, metadata still says v1, so the migration
/// restarts from scratch on next boot. The temp file is overwritten on
/// restart. Point lookups for already-deleted old keys return `None` and
/// are skipped.
fn migrate_1_to_2(backend: &dyn StorageBackend, db_path: &Path) -> Result<(), StoreError> {
    use std::io::Read;

    const BATCH_SIZE: usize = 10_000;
    let tmp_path = db_path.join("migration_v1_v2_keys.tmp");

    // Phase 1: Scan all old-format keys with a cursor, dump to temp file.
    // Only keys are written (length-prefixed), not values — keeps the file
    // small (~7 GB for 153M keys vs ~50+ GB with values).
    let key_count = {
        let txn = backend.begin_read()?;
        let iter = txn.prefix_iterator(RECEIPTS, &[])?;
        let mut file = std::io::BufWriter::new(std::fs::File::create(&tmp_path)?);
        let mut count: u64 = 0;
        for result in iter {
            let (key, _value) = result?;
            if key.len() == 40 {
                continue;
            }
            let len = key.len() as u32;
            file.write_all(&len.to_le_bytes())?;
            file.write_all(&key)?;
            count += 1;
        }
        file.into_inner()
            .map_err(|e| StoreError::Custom(format!("Failed to flush temp file: {e}")))?
            .sync_all()?;
        tracing::info!("Migration v1→v2: dumped {count} old-format keys to temp file");
        count
    };
    // Iterator and read transaction are dropped here.

    if key_count == 0 {
        let _ = std::fs::remove_file(&tmp_path);
        tracing::info!("Migration v1→v2 complete: nothing to migrate");
        return Ok(());
    }

    // Phase 2: Read keys back from temp file in batches, point-lookup
    // values, and re-key.
    let mut file = std::io::BufReader::new(std::fs::File::open(&tmp_path)?);
    let mut migrated: u64 = 0;
    let mut len_buf = [0u8; 4];

    loop {
        // Read a batch of keys from the temp file.
        let mut old_keys: Vec<Vec<u8>> = Vec::with_capacity(BATCH_SIZE);
        for _ in 0..BATCH_SIZE {
            match file.read_exact(&mut len_buf) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
                Err(e) => {
                    return Err(StoreError::Custom(format!(
                        "Failed to read temp file: {e}"
                    )))
                }
            }
            let key_len = u32::from_le_bytes(len_buf) as usize;
            let mut key = vec![0u8; key_len];
            file.read_exact(&mut key).map_err(|e| {
                StoreError::Custom(format!("Failed to read key from temp file: {e}"))
            })?;
            old_keys.push(key);
        }

        if old_keys.is_empty() {
            break;
        }

        // Point-lookup values and build the write batch.
        let txn = backend.begin_read()?;
        let mut batch: Vec<(Vec<u8>, Vec<u8>)> = Vec::with_capacity(old_keys.len());
        let mut delete_keys: Vec<Vec<u8>> = Vec::with_capacity(old_keys.len());

        for old_key in old_keys {
            let (block_hash, index) = match <(H256, u64)>::decode(&old_key) {
                Ok(decoded) => decoded,
                Err(_) => {
                    tracing::warn!(
                        "Skipping RECEIPTS key that failed RLP decode (len={})",
                        old_key.len()
                    );
                    continue;
                }
            };
            let value = match txn.get(RECEIPTS, &old_key)? {
                Some(v) => v.to_vec(),
                None => continue, // already deleted (crash-restart)
            };
            let new_key = receipt_key(&block_hash, index);
            batch.push((new_key, value));
            delete_keys.push(old_key);
        }
        drop(txn);

        // Write batch.
        if !batch.is_empty() {
            let count = batch.len() as u64;
            let mut tx = backend.begin_write()?;
            tx.put_batch(RECEIPTS, batch)?;
            for dk in &delete_keys {
                tx.delete(RECEIPTS, dk)?;
            }
            tx.commit()?;
            migrated += count;
            tracing::info!("Migration v1→v2: migrated {migrated} RECEIPTS entries so far");
        }
    }

    // Cleanup temp file.
    if let Err(e) = std::fs::remove_file(&tmp_path) {
        tracing::warn!("Failed to remove migration temp file: {e}");
    }

    tracing::info!("Migration v1→v2 complete: migrated {migrated} RECEIPTS entries total");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrations_length_matches_schema_version() {
        assert_eq!(
            MIGRATIONS.len(),
            (STORE_SCHEMA_VERSION - 1) as usize,
            "MIGRATIONS array length must be STORE_SCHEMA_VERSION - 1"
        );
    }

    #[test]
    fn run_pending_migrations_noop_when_current() {
        // When current_version == STORE_SCHEMA_VERSION, nothing should happen.
        // We use a dummy in-memory backend since no migrations will actually run.
        let backend = crate::backend::in_memory::InMemoryBackend::open().unwrap();
        let temp_dir = tempfile::tempdir().unwrap();

        // Write initial metadata
        write_metadata_version(temp_dir.path(), STORE_SCHEMA_VERSION).unwrap();

        let result = run_pending_migrations(&backend, temp_dir.path(), STORE_SCHEMA_VERSION);
        assert!(result.is_ok());
    }

    #[test]
    fn fresh_store_creates_correct_metadata() {
        let temp_dir = tempfile::tempdir().unwrap();

        write_metadata_version(temp_dir.path(), STORE_SCHEMA_VERSION).unwrap();

        let metadata_path = temp_dir.path().join(STORE_METADATA_FILENAME);
        let contents = std::fs::read_to_string(&metadata_path).unwrap();
        let metadata: StoreMetadata = serde_json::from_str(&contents).unwrap();
        assert_eq!(metadata.schema_version, STORE_SCHEMA_VERSION);
    }

    #[test]
    fn migrate_1_to_2_converts_rlp_keys_to_fixed_width() {
        use crate::api::{StorageBackend, StorageReadView};
        use ethrex_common::types::{Receipt, TxType};
        use ethrex_rlp::encode::RLPEncode;

        let backend = crate::backend::in_memory::InMemoryBackend::open().unwrap();

        let block_hash = H256::random();
        let receipts: Vec<Receipt> = (0..5)
            .map(|i| Receipt::new(TxType::Legacy, true, (i + 1) * 21000, vec![]))
            .collect();

        // Seed old-format RLP keys: (BlockHash, u64).encode_to_vec()
        {
            let mut tx = backend.begin_write().unwrap();
            let batch: Vec<(Vec<u8>, Vec<u8>)> = receipts
                .iter()
                .enumerate()
                .map(|(i, r)| {
                    let old_key = (block_hash, i as u64).encode_to_vec();
                    let value = r.encode_to_vec();
                    (old_key, value)
                })
                .collect();
            tx.put_batch(RECEIPTS, batch).unwrap();
            tx.commit().unwrap();
        }

        // Verify old keys exist
        {
            let txn = backend.begin_read().unwrap();
            let old_key = (block_hash, 0u64).encode_to_vec();
            assert!(txn.get(RECEIPTS, &old_key).unwrap().is_some());
        }

        // Run migration
        let temp_dir = tempfile::tempdir().unwrap();
        migrate_1_to_2(&backend, temp_dir.path()).unwrap();

        // Verify new fixed-width keys exist and old keys are gone
        let txn = backend.begin_read().unwrap();
        for i in 0..5u64 {
            let new_key = receipt_key(&block_hash, i);
            let value = txn
                .get(RECEIPTS, &new_key)
                .unwrap()
                .expect("new key should exist after migration");
            let decoded = Receipt::decode(value.as_ref()).unwrap();
            assert_eq!(decoded, receipts[i as usize]);

            let old_key = (block_hash, i).encode_to_vec();
            assert!(
                txn.get(RECEIPTS, &old_key).unwrap().is_none(),
                "old key should be deleted after migration"
            );
        }
    }
}
