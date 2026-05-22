use std::io::Write;
use std::path::Path;
use std::time::{Duration, Instant};

use ethrex_common::H256;
use ethrex_common::types::{BlockHash, BlockNumber, Index};
use ethrex_rlp::decode::RLPDecode;
use ethrex_rlp::encode::RLPEncode;

use crate::api::tables::{RECEIPTS, RECEIPTS_V2, TRANSACTION_LOCATIONS};
use crate::api::{StorageBackend, StorageWriteBatch};
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
pub const MIGRATIONS: &[MigrationFn] = &[migrate_1_to_2, migrate_2_to_3];

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

/// Minimum interval between migration progress log lines.
const PROGRESS_LOG_INTERVAL: Duration = Duration::from_secs(10);

/// Per-second processing rate for progress logs. Returns 0 when `elapsed` is
/// zero so the division can never produce `inf` or `NaN`.
fn entries_per_second(count: u64, elapsed: Duration) -> f64 {
    let secs = elapsed.as_secs_f64();
    if secs > 0.0 { count as f64 / secs } else { 0.0 }
}

/// Runs all pending migrations from `current_version` up to `STORE_SCHEMA_VERSION`.
///
/// Each migration is applied one version at a time, and the metadata file is
/// updated (with fsync) after each successful step for crash safety.
///
/// Returns `Ok(())` if `current_version == STORE_SCHEMA_VERSION` (no-op).
/// If `current_version > STORE_SCHEMA_VERSION` (older binary against a newer
/// database), it warns and returns `Ok(())` without migrating.
pub fn run_pending_migrations(
    backend: &dyn StorageBackend,
    db_path: &Path,
    current_version: u64,
) -> Result<(), StoreError> {
    if current_version > STORE_SCHEMA_VERSION {
        tracing::warn!(
            "Database schema is at v{current_version}, ahead of this binary's v{STORE_SCHEMA_VERSION}; \
             running an older binary against a newer database is unsupported. Upgrade the binary"
        );
    }

    let pending = STORE_SCHEMA_VERSION.saturating_sub(current_version);
    if pending == 0 {
        return Ok(());
    }

    tracing::info!(
        "Database schema is at v{current_version}, latest is v{STORE_SCHEMA_VERSION}; running {pending} migration(s). This may take a while on large databases"
    );

    for version in current_version..STORE_SCHEMA_VERSION {
        let target = version + 1;

        tracing::info!("Running schema migration v{version} → v{target}");
        let start = Instant::now();

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

        tracing::info!(
            "Schema migration v{version} → v{target} completed in {:.1}s",
            start.elapsed().as_secs_f64()
        );
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

/// Migrates the RECEIPTS table from RLP-encoded `(BlockHash, u64)` keys
/// to raw `block_hash (32B) || index (8B big-endian u64)` keys in a new
/// `receipts_v2` column family.
///
/// This two-CF approach copies entries from the old `receipts` CF to
/// `receipts_v2` with the new key format. The old `receipts` CF is **not**
/// deleted here — `Store::new()` calls `drop_obsolete_cfs()` right after
/// this migration returns, which drops it in the same startup.
///
/// Crash safety: if interrupted, metadata still says v1, so the migration
/// restarts from scratch on next boot. Duplicate puts to `receipts_v2` are
/// idempotent.
fn migrate_1_to_2(backend: &dyn StorageBackend) -> Result<(), StoreError> {
    const BATCH_SIZE: usize = 10_000;

    let txn = backend.begin_read()?;
    let iter = txn.prefix_iterator(RECEIPTS, &[])?;

    let mut batch: Vec<(Vec<u8>, Vec<u8>)> = Vec::with_capacity(BATCH_SIZE);
    let mut migrated: u64 = 0;
    let start = Instant::now();
    let mut last_progress_log = Instant::now();

    for result in iter {
        let (key, value) = result?;

        let (block_hash, index) = match <(H256, u64)>::decode(&key) {
            Ok(decoded) => decoded,
            Err(_) => {
                tracing::warn!(
                    "Schema migration v1 → v2: skipping receipts key that failed RLP decode (len={})",
                    key.len()
                );
                continue;
            }
        };

        let new_key = receipt_key(&block_hash, index);
        batch.push((new_key, value.to_vec()));

        if batch.len() >= BATCH_SIZE {
            let count = batch.len() as u64;
            let mut tx = backend.begin_write()?;
            tx.put_batch(RECEIPTS_V2, std::mem::take(&mut batch))?;
            tx.commit()?;
            migrated += count;
            if last_progress_log.elapsed() >= PROGRESS_LOG_INTERVAL {
                let rate = entries_per_second(migrated, start.elapsed());
                tracing::info!(
                    "Schema migration v1 → v2: {migrated} receipt entries migrated so far ({rate:.0} entries/s)"
                );
                last_progress_log = Instant::now();
            }
        }
    }

    // Flush remaining entries.
    if !batch.is_empty() {
        let count = batch.len() as u64;
        let mut tx = backend.begin_write()?;
        tx.put_batch(RECEIPTS_V2, batch)?;
        tx.commit()?;
        migrated += count;
    }

    tracing::info!("Schema migration v1 → v2: migrated {migrated} receipt entries in total");
    Ok(())
}

type TxLocation = (BlockNumber, BlockHash, Index);

/// Rewrites `TRANSACTION_LOCATIONS` from the v2 composite-key schema
/// (`key = tx_hash || block_hash`, `value = (block_number, block_hash, index)`)
/// to the v3 schema (`key = tx_hash`, `value = Vec<(block_number, block_hash, index)>`).
///
/// Streams the old table in lex order, grouping consecutive entries by tx_hash
/// (composite keys with the same 32-byte prefix are adjacent — both backends
/// iterate sorted). Flushes each group as an atomic write batch (merge the new
/// key + delete the old composite keys), chunking commits to bound memory.
/// Skips any already-migrated 32-byte keys it encounters.
///
/// Crash-resume is safe by construction: the new value is written with `merge`,
/// not `put`, so if a tx_hash ever has both a v3 value and leftover v2 siblings
/// (e.g. a non-atomic backend), the resumed run unions them (deduping by
/// block_hash) instead of overwriting. The merge operator that already backs
/// the live write path makes this free.
fn migrate_2_to_3(backend: &dyn StorageBackend) -> Result<(), StoreError> {
    const GROUPS_PER_COMMIT: usize = 50_000;

    let read = backend.begin_read()?;
    // Empty prefix → full-table scan. Both backends yield keys in sorted order,
    // which the same-prefix grouping below relies on.
    let iter = read.prefix_iterator(TRANSACTION_LOCATIONS, &[])?;

    let mut write_batch = backend.begin_write()?;
    let mut groups_in_batch: usize = 0;
    let mut current: Option<(H256, Vec<TxLocation>, Vec<Vec<u8>>)> = None;
    let mut total_groups: u64 = 0;
    let mut total_old_entries: u64 = 0;
    let start = Instant::now();
    let mut last_progress_log = Instant::now();

    for result in iter {
        let (key, value) = result?;

        // Already-migrated entries (32-byte tx_hash keys, from a prior partial run): skip.
        if key.len() == 32 {
            continue;
        }
        if key.len() != 64 {
            return Err(StoreError::Custom(format!(
                "unexpected TRANSACTION_LOCATIONS key length {} during migration",
                key.len()
            )));
        }

        total_old_entries += 1;
        if last_progress_log.elapsed() >= PROGRESS_LOG_INTERVAL {
            let rate = entries_per_second(total_old_entries, start.elapsed());
            tracing::info!(
                "Schema migration v2 → v3: {total_old_entries} transaction location entries processed so far ({rate:.0} entries/s)"
            );
            last_progress_log = Instant::now();
        }

        let tx_hash = H256::from_slice(&key[..32]);
        let location = TxLocation::decode(&value)?;
        let key_vec = key.into_vec();

        match &mut current {
            Some((h, locs, keys_to_delete)) if *h == tx_hash => {
                locs.push(location);
                keys_to_delete.push(key_vec);
            }
            _ => {
                if let Some((h, locs, keys_to_delete)) = current.take() {
                    flush_tx_location_group(&mut *write_batch, h, locs, keys_to_delete)?;
                    total_groups += 1;
                    groups_in_batch += 1;
                    if groups_in_batch >= GROUPS_PER_COMMIT {
                        write_batch.commit()?;
                        // Re-acquire instead of relying on post-commit reuse
                        // of the trait object (works today via mem::take in
                        // RocksDB and a no-op InMemory commit, but it's not
                        // a documented contract on `StorageWriteBatch`).
                        write_batch = backend.begin_write()?;
                        groups_in_batch = 0;
                    }
                }
                current = Some((tx_hash, vec![location], vec![key_vec]));
            }
        }
    }

    if let Some((h, locs, keys_to_delete)) = current {
        flush_tx_location_group(&mut *write_batch, h, locs, keys_to_delete)?;
        total_groups += 1;
    }

    // Final commit. `groups_in_batch` is not bumped/reset here intentionally
    // — the post-loop flush is followed immediately by a commit, after which
    // the variable goes out of scope.
    write_batch.commit()?;

    tracing::info!(
        "Schema migration v2 → v3: rewrote {} transaction location entries into {} transaction records",
        total_old_entries,
        total_groups
    );
    Ok(())
}

fn flush_tx_location_group(
    write_batch: &mut dyn StorageWriteBatch,
    tx_hash: H256,
    locations: Vec<TxLocation>,
    composite_keys: Vec<Vec<u8>>,
) -> Result<(), StoreError> {
    // Use `merge`, not `put`: the operand is the same `Vec` type as the value,
    // so a re-processed group unions with any existing v3 value (dedup by
    // block_hash) instead of overwriting it. The composite-key deletes ride in
    // the same batch, so the group is applied atomically.
    write_batch.merge(
        TRANSACTION_LOCATIONS,
        tx_hash.as_bytes(),
        &locations.encode_to_vec(),
    )?;
    for key in composite_keys {
        write_batch.delete(TRANSACTION_LOCATIONS, &key)?;
    }
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
        use crate::api::StorageBackend;
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
        migrate_1_to_2(&backend).unwrap();

        // Verify new fixed-width keys exist in RECEIPTS_V2
        let txn = backend.begin_read().unwrap();
        for i in 0..5u64 {
            let new_key = receipt_key(&block_hash, i);
            let value = txn
                .get(RECEIPTS_V2, &new_key)
                .unwrap()
                .expect("new key should exist in RECEIPTS_V2 after migration");
            let decoded = Receipt::decode(value.as_ref()).unwrap();
            assert_eq!(decoded, receipts[i as usize]);

            // Old keys should still be in RECEIPTS (drop_obsolete_cfs runs after migration)
            let old_key = (block_hash, i).encode_to_vec();
            assert!(
                txn.get(RECEIPTS, &old_key).unwrap().is_some(),
                "old key should still exist in RECEIPTS (dropped after migration)"
            );
        }
    }

    /// Seeds the backend with one entry under the v2 composite-key schema:
    /// `key = tx_hash || block_hash`, `value = (block_number, block_hash, index)`.
    fn seed_old_entry(
        backend: &dyn StorageBackend,
        tx_hash: H256,
        block_number: BlockNumber,
        block_hash: BlockHash,
        index: Index,
    ) {
        let mut composite_key = Vec::with_capacity(64);
        composite_key.extend_from_slice(tx_hash.as_bytes());
        composite_key.extend_from_slice(block_hash.as_bytes());
        let value = (block_number, block_hash, index).encode_to_vec();

        let mut batch = backend.begin_write().unwrap();
        batch
            .put(TRANSACTION_LOCATIONS, &composite_key, &value)
            .unwrap();
        batch.commit().unwrap();
    }

    fn read_new_entry(
        backend: &dyn StorageBackend,
        tx_hash: H256,
    ) -> Option<Vec<(BlockNumber, BlockHash, Index)>> {
        let read = backend.begin_read().unwrap();
        let bytes = read
            .get(TRANSACTION_LOCATIONS, tx_hash.as_bytes())
            .unwrap()?;
        Some(<Vec<(BlockNumber, BlockHash, Index)>>::decode(&bytes).unwrap())
    }

    fn h256(byte: u8) -> H256 {
        H256::from_low_u64_be(byte as u64)
    }

    #[test]
    fn migrate_2_to_3_empty_table() {
        let backend = crate::backend::in_memory::InMemoryBackend::open().unwrap();
        migrate_2_to_3(&backend).unwrap();
        // Nothing to assert other than no error and no spurious entries.
        assert!(read_new_entry(&backend, h256(1)).is_none());
    }

    #[test]
    fn migrate_2_to_3_single_entry_per_hash() {
        let backend = crate::backend::in_memory::InMemoryBackend::open().unwrap();
        seed_old_entry(&backend, h256(1), 100, h256(0x10), 0);
        seed_old_entry(&backend, h256(2), 101, h256(0x11), 5);
        seed_old_entry(&backend, h256(3), 102, h256(0x12), 7);

        migrate_2_to_3(&backend).unwrap();

        assert_eq!(
            read_new_entry(&backend, h256(1)).unwrap(),
            vec![(100u64, h256(0x10), 0u64)]
        );
        assert_eq!(
            read_new_entry(&backend, h256(2)).unwrap(),
            vec![(101u64, h256(0x11), 5u64)]
        );
        assert_eq!(
            read_new_entry(&backend, h256(3)).unwrap(),
            vec![(102u64, h256(0x12), 7u64)]
        );

        // Old composite-key entries are gone.
        let read = backend.begin_read().unwrap();
        let iter = read.prefix_iterator(TRANSACTION_LOCATIONS, &[]).unwrap();
        for entry in iter {
            let (key, _) = entry.unwrap();
            assert_eq!(key.len(), 32, "leftover non-migrated key: {:?}", key);
        }
    }

    #[test]
    fn migrate_2_to_3_multi_block_per_hash() {
        let backend = crate::backend::in_memory::InMemoryBackend::open().unwrap();
        // Same tx hash appears in three different blocks (reorg scenario).
        seed_old_entry(&backend, h256(0xAA), 100, h256(0x10), 3);
        seed_old_entry(&backend, h256(0xAA), 100, h256(0x11), 4);
        seed_old_entry(&backend, h256(0xAA), 101, h256(0x12), 5);

        migrate_2_to_3(&backend).unwrap();

        let mut got = read_new_entry(&backend, h256(0xAA)).unwrap();
        got.sort();
        let mut expected = vec![
            (100u64, h256(0x10), 3u64),
            (100u64, h256(0x11), 4u64),
            (101u64, h256(0x12), 5u64),
        ];
        expected.sort();
        assert_eq!(got, expected);
    }

    #[test]
    fn migrate_2_to_3_is_idempotent_on_partial_state() {
        // Simulate a crash-resume: the backend already has a v3 32-byte entry
        // for one tx hash (from a previously-completed chunk), and v2 composite
        // entries for another tx hash (still pending).
        let backend = crate::backend::in_memory::InMemoryBackend::open().unwrap();

        // Already-migrated v3 entry for h256(1).
        {
            let v3_value: Vec<(BlockNumber, BlockHash, Index)> =
                vec![(100, h256(0x10), 0), (100, h256(0x11), 0)];
            let mut batch = backend.begin_write().unwrap();
            batch
                .put(
                    TRANSACTION_LOCATIONS,
                    h256(1).as_bytes(),
                    &v3_value.encode_to_vec(),
                )
                .unwrap();
            batch.commit().unwrap();
        }
        // Pending v2 entries for h256(2).
        seed_old_entry(&backend, h256(2), 200, h256(0x20), 0);
        seed_old_entry(&backend, h256(2), 200, h256(0x21), 1);

        migrate_2_to_3(&backend).unwrap();

        // h256(1)'s already-migrated entry is unchanged.
        assert_eq!(
            read_new_entry(&backend, h256(1)).unwrap(),
            vec![(100u64, h256(0x10), 0u64), (100u64, h256(0x11), 0u64)]
        );

        // h256(2) is now migrated.
        let mut got = read_new_entry(&backend, h256(2)).unwrap();
        got.sort();
        let mut expected = vec![(200u64, h256(0x20), 0u64), (200u64, h256(0x21), 1u64)];
        expected.sort();
        assert_eq!(got, expected);

        // No leftover 64-byte keys.
        let read = backend.begin_read().unwrap();
        let iter = read.prefix_iterator(TRANSACTION_LOCATIONS, &[]).unwrap();
        for entry in iter {
            let (key, _) = entry.unwrap();
            assert_eq!(key.len(), 32);
        }
    }

    /// The pathological case flagged in review: a single tx_hash has BOTH a v3
    /// value (from a prior partial run) AND leftover v2 composite keys. Because
    /// `flush_tx_location_group` uses `merge` (not `put`), the resumed migration
    /// must UNION the leftover entries into the existing v3 value, not overwrite
    /// it — no locations may be lost.
    #[test]
    fn migrate_2_to_3_unions_same_hash_mixed_state() {
        let backend = crate::backend::in_memory::InMemoryBackend::open().unwrap();
        let tx = h256(0x42);

        // Pre-existing v3 value for `tx` (one block already migrated).
        {
            let v3_value: Vec<(BlockNumber, BlockHash, Index)> = vec![(100, h256(0x10), 0)];
            let mut batch = backend.begin_write().unwrap();
            batch
                .merge(
                    TRANSACTION_LOCATIONS,
                    tx.as_bytes(),
                    &v3_value.encode_to_vec(),
                )
                .unwrap();
            batch.commit().unwrap();
        }
        // Leftover v2 composite entries for the SAME tx (different blocks).
        seed_old_entry(&backend, tx, 101, h256(0x11), 3);
        seed_old_entry(&backend, tx, 102, h256(0x12), 7);

        migrate_2_to_3(&backend).unwrap();

        let mut got = read_new_entry(&backend, tx).unwrap();
        got.sort();
        let mut expected = vec![
            (100u64, h256(0x10), 0u64), // pre-existing v3 entry survives
            (101u64, h256(0x11), 3u64),
            (102u64, h256(0x12), 7u64),
        ];
        expected.sort();
        assert_eq!(
            got, expected,
            "merge must union, not overwrite, on mixed state"
        );
    }
}
