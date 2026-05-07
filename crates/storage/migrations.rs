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

        migration_for_version(version)(backend, db_path).map_err(|e| {
            StoreError::MigrationFailed {
                from: version,
                to: target,
                reason: e.to_string(),
            }
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
                Err(e) => return Err(StoreError::Custom(format!("Failed to read temp file: {e}"))),
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

// ===== Alternative migration strategies for benchmarking =====
//
// These live next to `migrate_1_to_2` so they can be A/B-compared in the
// `migrate_1_to_2_synthetic_load` test. Only the baseline is wired into
// the production migration table; the others are research code.

#[cfg(feature = "rocksdb")]
pub(crate) fn migrate_1_to_2_seek_resume(
    _backend: &crate::backend::rocksdb::RocksDBBackend,
) -> Result<(), StoreError> {
    unimplemented!("seek-resume strategy not implemented yet")
}

#[cfg(feature = "rocksdb")]
pub(crate) fn migrate_1_to_2_cursor_held(
    _backend: &crate::backend::rocksdb::RocksDBBackend,
) -> Result<(), StoreError> {
    unimplemented!("cursor-held strategy not implemented yet")
}

/// Two-CF strategy: copy re-keyed entries to a fresh `receipts_v2` CF using a
/// single open read cursor on the old `receipts` CF, flushing write batches
/// periodically. After the full pass, drop the old CF.
///
/// Pros: no read/write interleaving on the same CF; old CF's SSTs never get
/// rewritten or tombstoned (drop_cf is O(metadata)); cleanest pattern in RocksDB.
/// Cons: needs ~1× the receipts CF size of extra disk during the migration.
#[cfg(feature = "rocksdb")]
pub(crate) fn migrate_1_to_2_two_cf(db_path: &Path) -> Result<(), StoreError> {
    use crate::backend::rocksdb::RocksDBBackend;
    use rocksdb::{DBCompressionType, IteratorMode, Options, WriteBatch};

    const BATCH_SIZE: usize = 50_000;
    let backend = RocksDBBackend::open(db_path)?;
    let db = backend.raw_db();

    // Create the destination CF with the same compression as `receipts`.
    let mut cf_opts = Options::default();
    cf_opts.set_compression_type(DBCompressionType::Lz4);
    if db.cf_handle("receipts_v2").is_none() {
        db.create_cf("receipts_v2", &cf_opts)
            .map_err(|e| StoreError::Custom(format!("create_cf receipts_v2: {e}")))?;
    }

    let cf_old = db
        .cf_handle("receipts")
        .ok_or_else(|| StoreError::Custom("receipts CF missing".into()))?;
    let cf_new = db
        .cf_handle("receipts_v2")
        .ok_or_else(|| StoreError::Custom("receipts_v2 CF missing".into()))?;

    let mut migrated: u64 = 0;
    let mut batch = WriteBatch::default();
    let mut batch_count = 0usize;
    let mut iter = db.iterator_cf(&cf_old, IteratorMode::Start);
    while let Some(item) = iter.next() {
        let (key, value) =
            item.map_err(|e| StoreError::Custom(format!("iterate receipts: {e}")))?;
        if key.len() == 40 {
            continue; // already-new-format (no-op safety)
        }
        let (block_hash, index) = match <(H256, u64)>::decode(&key) {
            Ok(d) => d,
            Err(_) => continue,
        };
        let new_key = receipt_key(&block_hash, index);
        batch.put_cf(&cf_new, &new_key, &value);
        batch_count += 1;
        migrated += 1;

        if batch_count >= BATCH_SIZE {
            db.write(std::mem::take(&mut batch))
                .map_err(|e| StoreError::Custom(format!("write batch: {e}")))?;
            batch_count = 0;
            if migrated.is_multiple_of(1_000_000) {
                tracing::info!("two-cf: migrated {migrated} entries");
            }
        }
    }
    if batch_count > 0 {
        db.write(batch)
            .map_err(|e| StoreError::Custom(format!("final write batch: {e}")))?;
    }
    drop(iter);
    drop(cf_old);
    drop(cf_new);

    db.drop_cf("receipts")
        .map_err(|e| StoreError::Custom(format!("drop_cf receipts: {e}")))?;

    tracing::info!("two-cf migration complete: {migrated} entries copied");
    Ok(())
}

#[cfg(feature = "rocksdb")]
pub(crate) fn migrate_1_to_2_delete_range(
    _backend: &crate::backend::rocksdb::RocksDBBackend,
) -> Result<(), StoreError> {
    unimplemented!("delete-range strategy not implemented yet")
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

    /// Synthetic load + migration benchmark for the v1→v2 RECEIPTS re-key.
    ///
    /// Loads N synthetic receipts in old-format (RLP) keys, then runs one of
    /// several migration strategies and reports timing. Strategies share the
    /// same load path so timings are directly comparable.
    ///
    /// Strategy is selected via `ETHREX_MIG_STRATEGY`:
    /// - `baseline` (default) — current PR: temp-file dump + point lookups
    /// - `seek-resume` — drop & re-open cursor after each batch flush
    /// - `cursor-held` — single cursor open across writes
    /// - `two-cf` — copy to receipts_v2 CF, drop old CF
    /// - `delete-range` — like cursor-held, single range-delete at the end
    ///
    /// Configuration via env vars:
    /// - `ETHREX_MIG_RECEIPTS` — number of synthetic receipts (default 100k).
    /// - `ETHREX_MIG_VALUE_BYTES` — payload bytes per receipt (default 480).
    /// - `ETHREX_MIG_DIR` — preserved DB directory; otherwise tempdir.
    /// - `ETHREX_MIG_TXS_PER_BLOCK` — txs per synthetic block (default 200).
    /// - `ETHREX_MIG_LOAD_ONLY=1` — load only, skip migration.
    /// - `ETHREX_MIG_MIGRATE_ONLY=1` — skip load, run strategy only.
    /// - `ETHREX_MIG_RESULTS_FILE` — append a CSV row to this file.
    ///
    /// Run with:
    ///   cargo test -p ethrex-storage --features rocksdb --release \
    ///       migrate_1_to_2_synthetic_load -- --ignored --nocapture
    #[test]
    #[ignore = "writes a large RocksDB; run manually with --ignored"]
    #[cfg(feature = "rocksdb")]
    fn migrate_1_to_2_synthetic_load() {
        use crate::backend::rocksdb::RocksDBBackend;
        use ethrex_rlp::encode::RLPEncode;
        use std::time::Instant;

        fn env_usize(name: &str, default: usize) -> usize {
            std::env::var(name)
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(default)
        }
        fn env_flag(name: &str) -> bool {
            matches!(std::env::var(name).as_deref(), Ok("1") | Ok("true"))
        }

        let n_receipts = env_usize("ETHREX_MIG_RECEIPTS", 100_000);
        let value_bytes = env_usize("ETHREX_MIG_VALUE_BYTES", 480);
        let txs_per_block = env_usize("ETHREX_MIG_TXS_PER_BLOCK", 200).max(1);
        let load_only = env_flag("ETHREX_MIG_LOAD_ONLY");
        let migrate_only = env_flag("ETHREX_MIG_MIGRATE_ONLY");
        let strategy =
            std::env::var("ETHREX_MIG_STRATEGY").unwrap_or_else(|_| "baseline".to_string());

        // Either use an explicit dir (preserved across runs) or a tempdir.
        let (db_path, _keep_alive) = match std::env::var("ETHREX_MIG_DIR") {
            Ok(p) => {
                let path = std::path::PathBuf::from(p);
                std::fs::create_dir_all(&path).unwrap();
                (path, None)
            }
            Err(_) => {
                let td = tempfile::tempdir().unwrap();
                (td.path().to_path_buf(), Some(td))
            }
        };

        eprintln!(
            "synthetic migration test: strategy={strategy} receipts={n_receipts} \
             value_bytes={value_bytes} txs_per_block={txs_per_block} db_path={db_path:?}"
        );

        let backend = RocksDBBackend::open(&db_path).unwrap();

        // ------- Load phase: write synthetic old-format entries -------
        if !migrate_only {
            // Persist v1 metadata so the migration runner sees a "needs upgrade" state.
            write_metadata_version(&db_path, 1).unwrap();

            let load_start = Instant::now();
            let n_blocks = n_receipts.div_ceil(txs_per_block);
            const FLUSH_EVERY: usize = 50_000;
            let mut buf: Vec<(Vec<u8>, Vec<u8>)> = Vec::with_capacity(FLUSH_EVERY);
            let mut written: usize = 0;

            // Deterministic but unique block hashes via a counter-based "hash":
            // first 8 bytes = block_number (BE), rest zero-padded — keeps the
            // synthetic dataset reproducible across invocations.
            //
            // Build a high-entropy value template so LZ4 compression matches
            // mainnet's ~4× ratio. Each byte derives from a multiply-rotate
            // mix of a per-receipt seed; mostly random, but we'll insert a
            // small structured prefix per receipt for the seed.
            let mix_byte = |seed: u64, i: usize| -> u8 {
                let m = seed
                    .wrapping_mul(0x9E3779B97F4A7C15)
                    .wrapping_add(i as u64)
                    .rotate_left((i & 31) as u32);
                let m2 = m
                    .wrapping_mul(0xBF58476D1CE4E5B9)
                    .wrapping_add((i as u64).wrapping_mul(0x94D049BB133111EB));
                (m2 ^ m.rotate_right(13)) as u8
            };

            for block_n in 0..n_blocks {
                let mut hash_bytes = [0u8; 32];
                hash_bytes[0..8].copy_from_slice(&(block_n as u64).to_be_bytes());
                hash_bytes[24..32].copy_from_slice(&((block_n as u64).wrapping_mul(0x9E3779B1)).to_be_bytes());
                let block_hash = H256(hash_bytes);

                let txs_in_block = txs_per_block.min(n_receipts - written);
                for idx in 0..txs_in_block {
                    let old_key = (block_hash, idx as u64).encode_to_vec();
                    let seed = (written as u64) ^ ((block_n as u64) << 16);
                    // Mainnet-shaped value: alternating runs of zeros (status,
                    // low-bit gas, mostly-empty bloom filter) and high-entropy
                    // bytes (log topics, addresses). Targets ~3-4× LZ4 ratio.
                    let mut value = vec![0u8; value_bytes];
                    let mut i = 0;
                    while i < value_bytes {
                        let entropy_run = 32.min(value_bytes - i);
                        for j in 0..entropy_run {
                            value[i + j] = mix_byte(seed, i + j);
                        }
                        i += entropy_run;
                        // Skip a zero-run of comparable size (already 0).
                        i += 64.min(value_bytes - i);
                    }
                    buf.push((old_key, value));
                    written += 1;
                }

                if buf.len() >= FLUSH_EVERY || written == n_receipts {
                    let mut tx = backend.begin_write().unwrap();
                    tx.put_batch(RECEIPTS, std::mem::take(&mut buf)).unwrap();
                    tx.commit().unwrap();
                    if written % (FLUSH_EVERY * 4) == 0 || written == n_receipts {
                        eprintln!(
                            "  loaded {written}/{n_receipts} ({:.1}%) in {:.1}s",
                            (written as f64) / (n_receipts as f64) * 100.0,
                            load_start.elapsed().as_secs_f64()
                        );
                    }
                }
            }

            eprintln!(
                "load complete: {n_receipts} receipts in {:.1}s ({:.0} receipts/s)",
                load_start.elapsed().as_secs_f64(),
                n_receipts as f64 / load_start.elapsed().as_secs_f64()
            );
        }

        if load_only {
            eprintln!("ETHREX_MIG_LOAD_ONLY set — skipping migration");
            return;
        }

        // Drop and re-open to flush memtables and force the migration to read
        // from on-disk SSTs (more representative).
        drop(backend);

        // Snapshot pre-migration disk size for reporting.
        let pre_migration_bytes = dir_size_bytes(&db_path);

        // ------- Migration phase: dispatch on strategy -------
        let mig_start = Instant::now();
        match strategy.as_str() {
            "baseline" => {
                let backend = RocksDBBackend::open(&db_path).unwrap();
                migrate_1_to_2(&backend, &db_path).unwrap();
            }
            "seek-resume" => {
                let backend = RocksDBBackend::open(&db_path).unwrap();
                migrate_1_to_2_seek_resume(&backend).unwrap();
            }
            "cursor-held" => {
                let backend = RocksDBBackend::open(&db_path).unwrap();
                migrate_1_to_2_cursor_held(&backend).unwrap();
            }
            "two-cf" => {
                migrate_1_to_2_two_cf(&db_path).unwrap();
            }
            "delete-range" => {
                let backend = RocksDBBackend::open(&db_path).unwrap();
                migrate_1_to_2_delete_range(&backend).unwrap();
            }
            other => panic!("unknown ETHREX_MIG_STRATEGY: {other}"),
        }
        let mig_elapsed = mig_start.elapsed();

        let post_migration_bytes = dir_size_bytes(&db_path);
        let temp_path = db_path.join("migration_v1_v2_keys.tmp");
        let temp_leftover = std::fs::metadata(&temp_path).map(|m| m.len()).unwrap_or(0);
        eprintln!(
            "migration complete (strategy={strategy}): {:.1}s ({:.0} receipts/s); \
             pre={:.2} GB → post={:.2} GB; temp_leftover={} bytes",
            mig_elapsed.as_secs_f64(),
            n_receipts as f64 / mig_elapsed.as_secs_f64(),
            pre_migration_bytes as f64 / 1e9,
            post_migration_bytes as f64 / 1e9,
            temp_leftover
        );

        // Re-open for spot-check.
        let backend = RocksDBBackend::open(&db_path).unwrap();

        // ------- Spot-check correctness on a few entries -------
        if !migrate_only {
            let txn = backend.begin_read().unwrap();
            let mut checks_ok = 0;
            for sample_block in [0usize, 1, n_receipts.div_ceil(txs_per_block) / 2, n_receipts.div_ceil(txs_per_block).saturating_sub(1)] {
                let mut hash_bytes = [0u8; 32];
                hash_bytes[0..8].copy_from_slice(&(sample_block as u64).to_be_bytes());
                hash_bytes[24..32].copy_from_slice(&((sample_block as u64).wrapping_mul(0x9E3779B1)).to_be_bytes());
                let block_hash = H256(hash_bytes);
                let new_key = receipt_key(&block_hash, 0);
                let old_key = (block_hash, 0u64).encode_to_vec();
                let table = if strategy == "two-cf" { "receipts_v2" } else { RECEIPTS };
                if txn.get(table, &new_key).unwrap().is_some()
                    && (strategy == "two-cf"
                        || txn.get(RECEIPTS, &old_key).unwrap().is_none())
                {
                    checks_ok += 1;
                }
            }
            eprintln!("correctness spot-check: {checks_ok}/4 sample blocks re-keyed");
            assert!(checks_ok > 0, "migration produced no new-format keys");
        }

        // Append a CSV row if requested.
        if let Ok(results_file) = std::env::var("ETHREX_MIG_RESULTS_FILE") {
            use std::io::Write as _;
            let need_header = !std::path::Path::new(&results_file).exists();
            let mut f = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&results_file)
                .unwrap();
            if need_header {
                writeln!(
                    f,
                    "strategy,receipts,value_bytes,migration_secs,migration_per_s,pre_gb,post_gb"
                )
                .unwrap();
            }
            writeln!(
                f,
                "{},{},{},{:.3},{:.0},{:.3},{:.3}",
                strategy,
                n_receipts,
                value_bytes,
                mig_elapsed.as_secs_f64(),
                n_receipts as f64 / mig_elapsed.as_secs_f64(),
                pre_migration_bytes as f64 / 1e9,
                post_migration_bytes as f64 / 1e9
            )
            .unwrap();
        }
    }

    #[cfg(feature = "rocksdb")]
    fn dir_size_bytes(path: &std::path::Path) -> u64 {
        fn walk(p: &std::path::Path, total: &mut u64) {
            if let Ok(entries) = std::fs::read_dir(p) {
                for e in entries.flatten() {
                    let path = e.path();
                    if let Ok(md) = e.metadata() {
                        if md.is_dir() {
                            walk(&path, total);
                        } else {
                            *total += md.len();
                        }
                    }
                }
            }
        }
        let mut total = 0;
        walk(path, &mut total);
        total
    }

    #[test]
    fn migrate_1_to_2_converts_rlp_keys_to_fixed_width() {
        use crate::api::StorageReadView;
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
