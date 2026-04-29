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
/// Receives a reference to the storage backend so it can read/write data
/// as needed for the migration.
pub type MigrationFn = fn(backend: &dyn StorageBackend) -> Result<(), StoreError>;

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

        migration_for_version(version)(backend).map_err(|e| StoreError::MigrationFailed {
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
/// Crash safety: if interrupted mid-migration, metadata still says v1,
/// so the migration restarts from scratch on next boot. Keys that fail
/// RLP decode are assumed to be already migrated and are skipped.
fn migrate_1_to_2(backend: &dyn StorageBackend) -> Result<(), StoreError> {
    const BATCH_SIZE: usize = 10_000;

    let txn = backend.begin_read()?;
    let iter = txn.prefix_iterator(RECEIPTS, &[])?;

    let mut batch: Vec<(Vec<u8>, Vec<u8>)> = Vec::with_capacity(BATCH_SIZE);
    let mut delete_keys: Vec<Vec<u8>> = Vec::with_capacity(BATCH_SIZE);
    let mut migrated: u64 = 0;

    for result in iter {
        let (old_key, value) = result?;

        // If this key is already 40 bytes (32B hash + 8B index), it's already
        // in the new format — skip it. This handles crash-restart scenarios.
        if old_key.len() == 40 {
            continue;
        }

        // Try to decode the old RLP key as (H256, u64)
        let (block_hash, index) = match <(H256, u64)>::decode(old_key.as_ref()) {
            Ok(decoded) => decoded,
            Err(_) => {
                // If RLP decode fails, skip — could be corrupted or already migrated
                tracing::warn!(
                    "Skipping RECEIPTS key that failed RLP decode (len={})",
                    old_key.len()
                );
                continue;
            }
        };

        let new_key = receipt_key(&block_hash, index);
        batch.push((new_key, value.to_vec()));
        delete_keys.push(old_key.to_vec());

        if batch.len() >= BATCH_SIZE {
            let mut tx = backend.begin_write()?;
            tx.put_batch(RECEIPTS, batch.clone())?;
            for dk in &delete_keys {
                tx.delete(RECEIPTS, dk)?;
            }
            tx.commit()?;
            migrated += batch.len() as u64;
            tracing::info!("Migration v1→v2: migrated {migrated} RECEIPTS entries so far");
            batch.clear();
            delete_keys.clear();
        }
    }

    // Flush remaining entries
    if !batch.is_empty() {
        let mut tx = backend.begin_write()?;
        tx.put_batch(RECEIPTS, batch.clone())?;
        for dk in &delete_keys {
            tx.delete(RECEIPTS, dk)?;
        }
        tx.commit()?;
        migrated += batch.len() as u64;
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
}
