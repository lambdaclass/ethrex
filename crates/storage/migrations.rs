use std::io::Write;
use std::path::Path;

use crate::api::StorageBackend;
use crate::error::StoreError;
use crate::{STORE_METADATA_FILENAME, STORE_SCHEMA_VERSION};

use super::store::StoreMetadata;

use crate::api::tables::{
    BLOCK_HASHES_BY_NUMBER, BODIES, CANONICAL_BLOCK_HASHES, CHAIN_DATA, HEADERS,
};
use crate::utils::{ChainDataIndex, block_hashes_by_number_key, chain_data_key};
use ethrex_common::H256;
use ethrex_common::types::BlockHeader;
use ethrex_rlp::decode::RLPDecode;
use ethrex_rlp::encode::RLPEncode;

const BACKFILL_BATCH_SIZE: usize = 10_000;

/// Backfill `BLOCK_HASHES_BY_NUMBER` from every entry in `HEADERS`,
/// streaming reads against batched writes so heap stays bounded by
/// `BACKFILL_BATCH_SIZE`.
///
/// Uses [`StorageReadView::full_scan`] not `prefix_iterator(_, &[])` —
/// prefix iteration on a CF with a prefix extractor can silently terminate
/// early on RocksDB.
///
/// Idempotent: re-running rewrites the same keys.
pub(crate) fn migrate_1_to_2(backend: &dyn StorageBackend) -> Result<(), StoreError> {
    migrate_1_to_2_with_batch_size(backend, BACKFILL_BATCH_SIZE)
}

/// Same as [`migrate_1_to_2`] but with a configurable batch size, so tests
/// can exercise the flush-boundary path without 10,001 headers.
fn migrate_1_to_2_with_batch_size(
    backend: &dyn StorageBackend,
    batch_size: usize,
) -> Result<(), StoreError> {
    let rtxn = backend.begin_read()?;
    let iter = rtxn.full_scan(HEADERS)?;

    let mut bw = backend.begin_write()?;
    let mut written: usize = 0;

    for item in iter {
        let (hash_key, header_bytes) = item?;
        // The HEADERS key is `block_hash.encode_to_vec()` — RLP-encoded H256,
        // which is `0xa0` + 32 raw hash bytes. Slicing out the raw bytes
        // avoids 20M+ Keccak256 recomputations during the mainnet migration.
        if hash_key.len() != 33 {
            return Err(StoreError::Custom(format!(
                "backfill: unexpected HEADERS key length {} (expected 33)",
                hash_key.len()
            )));
        }
        let hash = H256::from_slice(&hash_key[1..33]);
        let header = BlockHeader::decode(&header_bytes)
            .map_err(|e| StoreError::Custom(format!("backfill decode: {e}")))?;

        let index_key = block_hashes_by_number_key(header.number, hash);
        bw.put(BLOCK_HASHES_BY_NUMBER, &index_key, &[])?;
        written += 1;

        if written >= batch_size {
            bw.commit()?;
            bw = backend.begin_write()?;
            written = 0;
        }
    }

    // Final flush of any tail entries. Also handles the 0-header case
    // (commits an empty batch, which is a harmless no-op on both backends).
    bw.commit()?;

    fix_earliest_block_number(backend)?;

    Ok(())
}

/// Restores the v2 invariant `EarliestBlockNumber = lowest canonical block
/// with a body` on snap-synced v1 DBs (where it was never written and sits
/// at `0` despite a body gap below the pivot). Full-sync nodes (body at
/// block 0) are left alone. Idempotent.
fn fix_earliest_block_number(backend: &dyn StorageBackend) -> Result<(), StoreError> {
    let txn = backend.begin_read()?;

    let body_exists = |n: u64| -> Result<bool, StoreError> {
        let hash_bytes = match txn.get(CANONICAL_BLOCK_HASHES, &n.to_le_bytes())? {
            Some(b) => b,
            None => return Ok(false),
        };
        let hash = H256::decode(hash_bytes.as_slice()).map_err(StoreError::from)?;
        Ok(txn.get(BODIES, &hash.encode_to_vec())?.is_some())
    };

    // Full-sync node — nothing to do.
    if body_exists(0)? {
        return Ok(());
    }

    let latest_bytes = match txn.get(
        CHAIN_DATA,
        &chain_data_key(ChainDataIndex::LatestBlockNumber),
    )? {
        Some(b) => b,
        // Uninitialized node — nothing to fix.
        None => return Ok(()),
    };
    let latest_array: [u8; 8] = latest_bytes
        .try_into()
        .map_err(|_| StoreError::Custom("invalid LatestBlockNumber bytes".into()))?;
    let latest = u64::from_le_bytes(latest_array);

    let mut lo = 0u64;
    let mut hi = latest;
    let mut floor: Option<u64> = None;
    while lo <= hi {
        let mid = lo + (hi - lo) / 2;
        if body_exists(mid)? {
            floor = Some(mid);
            if mid == 0 {
                break;
            }
            hi = mid - 1;
        } else {
            if mid == u64::MAX {
                break;
            }
            lo = mid + 1;
        }
    }

    drop(txn);

    if let Some(n) = floor {
        let mut wtxn = backend.begin_write()?;
        wtxn.put(
            CHAIN_DATA,
            &chain_data_key(ChainDataIndex::EarliestBlockNumber),
            &n.to_le_bytes(),
        )?;
        wtxn.commit()?;
    }

    Ok(())
}

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrate_1_to_2_backfills_block_hashes_index() {
        use crate::api::tables::{BLOCK_HASHES_BY_NUMBER, HEADERS};
        use crate::rlp::BlockHeaderRLP;
        use ethrex_common::types::BlockHeader;
        use ethrex_rlp::encode::RLPEncode;

        let backend = crate::backend::in_memory::InMemoryBackend::open().unwrap();

        // Populate HEADERS with several entries representing a "v1 state".
        // Each header has a distinct `number` so their hashes differ.
        let headers: Vec<BlockHeader> = (10u64..14)
            .map(|n| BlockHeader {
                number: n,
                ..BlockHeader::default()
            })
            .collect();

        {
            let mut txn = backend.begin_write().unwrap();
            for h in &headers {
                let hash = h.hash();
                let hash_key = hash.encode_to_vec();
                let header_rlp = BlockHeaderRLP::from(h.clone()).into_vec();
                txn.put(HEADERS, &hash_key, &header_rlp).unwrap();
            }
            txn.commit().unwrap();
        }

        // Run the migration
        super::migrate_1_to_2(&backend).unwrap();

        // Assert index entries exist for every header
        let txn = backend.begin_read().unwrap();
        for h in &headers {
            let hash = h.hash();
            let mut key = Vec::with_capacity(40);
            key.extend_from_slice(&h.number.to_be_bytes());
            key.extend_from_slice(hash.as_bytes());
            let found = txn.get(BLOCK_HASHES_BY_NUMBER, &key).unwrap();
            assert!(
                found.is_some(),
                "index missing for block {} hash {:?}",
                h.number,
                hash
            );
        }
    }

    #[test]
    fn migrate_1_to_2_empty_headers_is_noop() {
        // 0-header v1 DB (e.g. a fresh genesis with no blocks yet): the
        // migration must succeed without writing anything.
        use crate::api::tables::BLOCK_HASHES_BY_NUMBER;

        let backend = crate::backend::in_memory::InMemoryBackend::open().unwrap();
        super::migrate_1_to_2(&backend).unwrap();

        let txn = backend.begin_read().unwrap();
        let count = txn.full_scan(BLOCK_HASHES_BY_NUMBER).unwrap().count();
        assert_eq!(
            count, 0,
            "no index entries should exist after empty backfill"
        );
    }

    #[test]
    fn migrate_1_to_2_exercises_batch_boundary() {
        // Force the mid-loop flush + final flush paths by using a tiny
        // batch size and a count that straddles a boundary
        // (batch_size + 1 = 11 entries with batch_size = 10).
        use crate::api::tables::{BLOCK_HASHES_BY_NUMBER, HEADERS};
        use crate::rlp::BlockHeaderRLP;
        use ethrex_common::types::BlockHeader;
        use ethrex_rlp::encode::RLPEncode;

        let backend = crate::backend::in_memory::InMemoryBackend::open().unwrap();

        let headers: Vec<BlockHeader> = (100u64..111)
            .map(|n| BlockHeader {
                number: n,
                ..BlockHeader::default()
            })
            .collect();
        assert_eq!(headers.len(), 11);

        {
            let mut txn = backend.begin_write().unwrap();
            for h in &headers {
                let hash = h.hash();
                let hash_key = hash.encode_to_vec();
                let header_rlp = BlockHeaderRLP::from(h.clone()).into_vec();
                txn.put(HEADERS, &hash_key, &header_rlp).unwrap();
            }
            txn.commit().unwrap();
        }

        super::migrate_1_to_2_with_batch_size(&backend, 10).unwrap();

        let txn = backend.begin_read().unwrap();
        for h in &headers {
            let hash = h.hash();
            let mut key = Vec::with_capacity(40);
            key.extend_from_slice(&h.number.to_be_bytes());
            key.extend_from_slice(hash.as_bytes());
            assert!(
                txn.get(BLOCK_HASHES_BY_NUMBER, &key).unwrap().is_some(),
                "index missing for block {} after batched backfill",
                h.number
            );
        }
        let total = txn.full_scan(BLOCK_HASHES_BY_NUMBER).unwrap().count();
        assert_eq!(total, headers.len());
    }

    #[test]
    fn migrate_1_to_2_is_idempotent() {
        // Re-running after a crash must not error. The final state must be
        // identical to a single-run state.
        use crate::api::tables::{BLOCK_HASHES_BY_NUMBER, HEADERS};
        use crate::rlp::BlockHeaderRLP;
        use ethrex_common::types::BlockHeader;
        use ethrex_rlp::encode::RLPEncode;

        let backend = crate::backend::in_memory::InMemoryBackend::open().unwrap();

        let headers: Vec<BlockHeader> = (1u64..6)
            .map(|n| BlockHeader {
                number: n,
                ..BlockHeader::default()
            })
            .collect();

        {
            let mut txn = backend.begin_write().unwrap();
            for h in &headers {
                let hash = h.hash();
                txn.put(
                    HEADERS,
                    &hash.encode_to_vec(),
                    &BlockHeaderRLP::from(h.clone()).into_vec(),
                )
                .unwrap();
            }
            txn.commit().unwrap();
        }

        super::migrate_1_to_2(&backend).unwrap();
        // Second invocation should overwrite the same keys without error.
        super::migrate_1_to_2(&backend).unwrap();

        let txn = backend.begin_read().unwrap();
        let total = txn.full_scan(BLOCK_HASHES_BY_NUMBER).unwrap().count();
        assert_eq!(total, headers.len(), "idempotent re-run must not duplicate");
    }

    #[test]
    fn migrate_1_to_2_fixes_earliest_block_number_on_snap_synced_state() {
        use crate::api::tables::{BODIES, CANONICAL_BLOCK_HASHES, CHAIN_DATA, HEADERS};
        use crate::rlp::{BlockBodyRLP, BlockHeaderRLP};
        use crate::utils::{ChainDataIndex, chain_data_key};
        use ethrex_common::types::{BlockBody, BlockHeader};
        use ethrex_rlp::encode::RLPEncode;

        let backend = crate::backend::in_memory::InMemoryBackend::open().unwrap();

        // Build a simulated v1 snap-synced state:
        //  - Canonical hashes 0..=10 all set.
        //  - Headers stored for all 10 (so the index backfill picks them up).
        //  - Bodies stored ONLY for 5..=10 (simulates pivot = 5).
        //  - EarliestBlockNumber NOT written (legacy v1 default).
        //  - LatestBlockNumber = 10 (read by the migration's binary search upper bound).
        let headers: Vec<BlockHeader> = (0u64..=10)
            .map(|n| BlockHeader {
                number: n,
                ..BlockHeader::default()
            })
            .collect();
        {
            let mut txn = backend.begin_write().unwrap();
            for h in &headers {
                let hash = h.hash();
                let hash_key = hash.encode_to_vec();
                let header_rlp = BlockHeaderRLP::from(h.clone()).into_vec();
                txn.put(HEADERS, &hash_key, &header_rlp).unwrap();
                txn.put(CANONICAL_BLOCK_HASHES, &h.number.to_le_bytes(), &hash_key)
                    .unwrap();
                if h.number >= 5 {
                    let body_rlp = BlockBodyRLP::from_bytes(BlockBody::default().encode_to_vec());
                    txn.put(BODIES, &hash_key, body_rlp.bytes()).unwrap();
                }
            }
            // LatestBlockNumber = 10
            txn.put(
                CHAIN_DATA,
                &chain_data_key(ChainDataIndex::LatestBlockNumber),
                &10u64.to_le_bytes(),
            )
            .unwrap();
            txn.commit().unwrap();
        }

        super::migrate_1_to_2(&backend).unwrap();

        // EarliestBlockNumber should now be 5 (lowest block with a body).
        let txn = backend.begin_read().unwrap();
        let earliest_bytes = txn
            .get(
                CHAIN_DATA,
                &chain_data_key(ChainDataIndex::EarliestBlockNumber),
            )
            .unwrap()
            .expect("earliest must be written");
        let earliest_array: [u8; 8] = earliest_bytes.try_into().unwrap();
        let earliest = u64::from_le_bytes(earliest_array);
        assert_eq!(
            earliest, 5,
            "migration should set EarliestBlockNumber to pivot"
        );
    }

    #[test]
    fn migrate_1_to_2_preserves_earliest_block_number_on_full_synced_state() {
        use crate::api::tables::{BODIES, CANONICAL_BLOCK_HASHES, CHAIN_DATA, HEADERS};
        use crate::rlp::{BlockBodyRLP, BlockHeaderRLP};
        use crate::utils::{ChainDataIndex, chain_data_key};
        use ethrex_common::types::{BlockBody, BlockHeader};
        use ethrex_rlp::encode::RLPEncode;

        let backend = crate::backend::in_memory::InMemoryBackend::open().unwrap();

        // Full-sync state: bodies present for ALL blocks 0..=5.
        let headers: Vec<BlockHeader> = (0u64..=5)
            .map(|n| BlockHeader {
                number: n,
                ..BlockHeader::default()
            })
            .collect();
        {
            let mut txn = backend.begin_write().unwrap();
            for h in &headers {
                let hash = h.hash();
                let hash_key = hash.encode_to_vec();
                let header_rlp = BlockHeaderRLP::from(h.clone()).into_vec();
                txn.put(HEADERS, &hash_key, &header_rlp).unwrap();
                txn.put(CANONICAL_BLOCK_HASHES, &h.number.to_le_bytes(), &hash_key)
                    .unwrap();
                let body_rlp = BlockBodyRLP::from_bytes(BlockBody::default().encode_to_vec());
                txn.put(BODIES, &hash_key, body_rlp.bytes()).unwrap();
            }
            txn.put(
                CHAIN_DATA,
                &chain_data_key(ChainDataIndex::LatestBlockNumber),
                &5u64.to_le_bytes(),
            )
            .unwrap();
            txn.commit().unwrap();
        }

        super::migrate_1_to_2(&backend).unwrap();

        // EarliestBlockNumber should be 0 (genesis body present → no fix).
        let txn = backend.begin_read().unwrap();
        let earliest_bytes = txn
            .get(
                CHAIN_DATA,
                &chain_data_key(ChainDataIndex::EarliestBlockNumber),
            )
            .unwrap();
        // It's acceptable for the migration to either leave it unset (legacy v1 default)
        // or write 0 explicitly. Both mean "earliest=0".
        let earliest = match earliest_bytes {
            Some(b) => u64::from_le_bytes(b.try_into().unwrap()),
            None => 0,
        };
        assert_eq!(earliest, 0);
    }

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
