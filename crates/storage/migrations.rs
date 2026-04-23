use std::io::Write;
use std::path::Path;

use crate::api::StorageBackend;
use crate::error::StoreError;
use crate::{STORE_METADATA_FILENAME, STORE_SCHEMA_VERSION};

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
    // Currently empty — no migrations exist yet.
    // When STORE_SCHEMA_VERSION is bumped to 2, add migrate_1_to_2 here.
];

// Compile-time check: the number of migration functions must match the number
// of version gaps (i.e. STORE_SCHEMA_VERSION - 1).
const _: () = assert!(
    MIGRATIONS.len() == (STORE_SCHEMA_VERSION - 1) as usize,
    "MIGRATIONS length must equal STORE_SCHEMA_VERSION - 1"
);

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
    assert!(
        MIGRATIONS.len() == (STORE_SCHEMA_VERSION - 1) as usize,
        "MIGRATIONS length must equal STORE_SCHEMA_VERSION - 1"
    );

    for version in current_version..STORE_SCHEMA_VERSION {
        let idx = (version - 1) as usize; // v1→v2 is index 0
        let target = version + 1;

        tracing::info!("Running migration v{version} → v{target}");

        MIGRATIONS[idx](backend).map_err(|e| StoreError::MigrationFailed {
            from: version,
            to: target,
            reason: e.to_string(),
        })?;

        // Persist the new version to metadata.json after each migration step
        write_metadata_version(db_path, target)?;

        tracing::info!("Migration v{version} → v{target} completed");
    }

    Ok(())
}

/// Writes the schema version to metadata.json using write-to-temp-then-rename
/// for crash safety. On POSIX filesystems `rename` is atomic, so the metadata
/// file is never left in a partial/truncated state.
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
    fn fresh_store_creates_correct_metadata() {
        let temp_dir = tempfile::tempdir().unwrap();

        write_metadata_version(temp_dir.path(), STORE_SCHEMA_VERSION).unwrap();

        let metadata_path = temp_dir.path().join(STORE_METADATA_FILENAME);
        let contents = std::fs::read_to_string(&metadata_path).unwrap();
        let metadata: StoreMetadata = serde_json::from_str(&contents).unwrap();
        assert_eq!(metadata.schema_version, STORE_SCHEMA_VERSION);
    }
}
