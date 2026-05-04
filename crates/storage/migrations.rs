use std::io::Write;
use std::path::Path;

use crate::api::StorageBackend;
use crate::api::tables::{EXECUTION_WITNESSES, MISC_VALUES, STATE_BACKEND_FORMAT_KEY};
use crate::error::StoreError;
use crate::store::backend_kind_to_byte;
use crate::{STORE_METADATA_FILENAME, STORE_SCHEMA_VERSION};
use ethrex_state_backend::BackendKind;

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
pub const MIGRATIONS: &[MigrationFn] = &[
    // v1 → v2: shared-trie abstraction landed.
    //   - Stamps `STATE_BACKEND_FORMAT_KEY = MPT` in `MISC_VALUES` so the
    //     format-marker check on subsequent opens has something to compare
    //     against. v1 DBs are MPT by definition (no other backend existed).
    //   - Clears `EXECUTION_WITNESSES`: the rkyv layout of cached witnesses
    //     changed (`Option<Node>` + `BTreeMap<H256, Node>` → `Vec<Vec<u8>>`),
    //     so old rows can no longer be deserialized. The witnesses are
    //     regenerated on demand, so dropping them is safe.
    migrate_1_to_2,
];

fn migrate_1_to_2(backend: &dyn StorageBackend) -> Result<(), StoreError> {
    backend.clear_table(EXECUTION_WITNESSES)?;

    let mut tx = backend.begin_write()?;
    tx.put(
        MISC_VALUES,
        STATE_BACKEND_FORMAT_KEY,
        &[backend_kind_to_byte(BackendKind::Mpt)],
    )?;
    tx.commit()?;

    Ok(())
}

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
    fn migrate_1_to_2_stamps_marker_and_clears_witnesses() {
        let backend = crate::backend::in_memory::InMemoryBackend::open().unwrap();

        // Pre-populate: pretend a v1 DB with no marker and a stale witness row.
        let mut setup = backend.begin_write().unwrap();
        setup
            .put(EXECUTION_WITNESSES, b"some_block_hash", b"stale_rkyv_bytes")
            .unwrap();
        setup.commit().unwrap();

        let pre_marker = backend
            .begin_read()
            .unwrap()
            .get(MISC_VALUES, STATE_BACKEND_FORMAT_KEY)
            .unwrap();
        assert_eq!(pre_marker, None, "marker absent before migration");
        let pre_witness = backend
            .begin_read()
            .unwrap()
            .get(EXECUTION_WITNESSES, b"some_block_hash")
            .unwrap();
        assert!(
            pre_witness.is_some(),
            "witness row present before migration"
        );

        migrate_1_to_2(&backend).unwrap();

        let post_marker = backend
            .begin_read()
            .unwrap()
            .get(MISC_VALUES, STATE_BACKEND_FORMAT_KEY)
            .unwrap();
        assert_eq!(
            post_marker,
            Some(vec![backend_kind_to_byte(BackendKind::Mpt)]),
            "marker stamped to MPT after migration"
        );
        let post_witness = backend
            .begin_read()
            .unwrap()
            .get(EXECUTION_WITNESSES, b"some_block_hash")
            .unwrap();
        assert_eq!(post_witness, None, "witness row cleared after migration");
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
