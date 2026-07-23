//! Integration test for the state-bench recording backend + undo log.
//!
//! Exercises the full round-trip that the run orchestration relies on:
//!   1. wrap a fresh RocksDB backend in a `RecordingBackend`,
//!   2. build a `Store` via the bench-only `from_backend_bench` constructor,
//!   3. record a handful of writes routed through the `Store`,
//!   4. serialize the undo log to a file and read it back,
//!   5. replay it and assert the state digest byte-equals the pristine digest
//!      and the newly-created keys are gone.
//!
//! Uses a minimal ad-hoc DB (not the multi-GB fixture): an empty RocksDB in a
//! throwaway tempdir plus one default block.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;

use ethrex_common::types::Block;
use ethrex_rlp::encode::RLPEncode;
use ethrex_storage::api::StorageBackend;
use ethrex_storage::api::tables::HEADERS;
use ethrex_storage::backend::rocksdb::{RocksDBBackend, RocksDbOpenOpts};
use ethrex_storage::{DB_COMMIT_THRESHOLD, STORE_METADATA_FILENAME, STORE_SCHEMA_VERSION, Store};

use state_bench::recording_backend::{
    RecordingBackend, apply_undo_log, load_undo_log, save_undo_log, state_digest,
};

/// A disk-backed scratch base so the DB doesn't land on a small tmpfs `/tmp`.
fn scratch_base() -> PathBuf {
    if let Ok(home) = std::env::var("HOME") {
        let cache = PathBuf::from(home).join(".cache").join("tmp");
        if std::fs::create_dir_all(&cache).is_ok() {
            return cache;
        }
    }
    std::env::temp_dir()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn recording_backend_undo_restores_pristine_state() -> Result<()> {
    let tmp = tempfile::Builder::new()
        .prefix("csb-undo-")
        .tempdir_in(scratch_base())?;
    let db_path = tmp.path().to_path_buf();

    // Open the raw RocksDB backend, then stamp the schema-version metadata so
    // `from_backend_bench` accepts the datadir (it refuses to bootstrap).
    let rocks = RocksDBBackend::open(&db_path, RocksDbOpenOpts::new(64 * 1024 * 1024))?;
    std::fs::write(
        db_path.join(STORE_METADATA_FILENAME),
        format!("{{\"schema_version\":{STORE_SCHEMA_VERSION}}}"),
    )?;

    // Wrap in the recorder; keep the concrete Arc for control and hand a trait
    // object to the Store so writes route through the recording layer.
    let backend = RecordingBackend::new(Arc::new(rocks));
    let store = Store::from_backend_bench(
        backend.clone() as Arc<dyn StorageBackend>,
        db_path.clone(),
        DB_COMMIT_THRESHOLD,
        16,
    )?;

    // Pristine digest of the empty state.
    let pristine = state_digest(backend.as_ref())?;

    // Record writes routed through the Store.
    backend.set_recording(true);
    let mut block = Block::default();
    block.header.number = 1;
    let block_hash = block.hash();
    store.add_block(block).await?;
    store.wait_for_persistence_idle().await?;

    let header_key = block_hash.encode_to_vec();

    // The write changed the digest, produced undo entries, and created the key.
    let after_write = state_digest(backend.as_ref())?;
    assert_ne!(
        pristine, after_write,
        "recorded writes did not change the state digest"
    );
    assert!(backend.undo_log_len() > 0, "no undo entries were recorded");
    assert!(
        backend.begin_read()?.get(HEADERS, &header_key)?.is_some(),
        "header key should exist after the write"
    );

    // Drain the log, round-trip it through a file, and confirm it survives.
    let log = backend.take_undo_log();
    let log_len = log.len();
    let log_path = db_path.join("undo.bin");
    save_undo_log(&log, &log_path)?;
    let loaded = load_undo_log(&log_path)?;
    assert_eq!(
        log_len,
        loaded.len(),
        "undo log length changed across the file round-trip"
    );

    // Replay the loaded log; recording is off so the undo isn't itself recorded.
    backend.set_recording(false);
    apply_undo_log(backend.as_ref(), &loaded)?;

    // State is byte-identical to pristine and the new key is gone.
    let after_undo = state_digest(backend.as_ref())?;
    assert_eq!(
        pristine, after_undo,
        "state digest not restored to pristine after undo"
    );
    assert!(
        backend.begin_read()?.get(HEADERS, &header_key)?.is_none(),
        "newly-created header key should be gone after undo"
    );

    Ok(())
}

/// Directly exercises the `prev = Some(..)` restore and delete-of-pre-existing
/// paths (the empty-block test above only covers new-key deletion). No `Store`
/// needed: drive the `RecordingBackend` write API straight through.
#[test]
fn undo_restores_overwritten_and_deleted_preexisting_keys() -> Result<()> {
    let tmp = tempfile::Builder::new()
        .prefix("csb-undo-preexist-")
        .tempdir_in(scratch_base())?;
    let rocks = RocksDBBackend::open(tmp.path(), RocksDbOpenOpts::new(64 * 1024 * 1024))?;
    let backend = RecordingBackend::new(Arc::new(rocks));

    let (k_overwrite, k_delete, k_new) = (
        b"key-ow".as_slice(),
        b"key-del".as_slice(),
        b"key-new".as_slice(),
    );

    // Seed two pre-existing keys with recording OFF (they are the pristine state).
    {
        let mut tx = backend.begin_write()?;
        tx.put(HEADERS, k_overwrite, b"A")?;
        tx.put(HEADERS, k_delete, b"X")?;
        tx.commit()?;
    }
    let pristine = state_digest(backend.as_ref())?;

    // Recording ON: overwrite one pre-existing key, delete another, create a new one.
    backend.set_recording(true);
    {
        let mut tx = backend.begin_write()?;
        tx.put(HEADERS, k_overwrite, b"B")?;
        tx.delete(HEADERS, k_delete)?;
        tx.put(HEADERS, k_new, b"C")?;
        tx.commit()?;
    }
    assert_ne!(pristine, state_digest(backend.as_ref())?);

    // Undo and assert every key is back to its pristine value/absence.
    let log = backend.take_undo_log();
    backend.set_recording(false);
    apply_undo_log(backend.as_ref(), &log)?;

    let view = backend.begin_read()?;
    assert_eq!(
        view.get(HEADERS, k_overwrite)?,
        Some(b"A".to_vec()),
        "overwritten key not restored to its prior value"
    );
    assert_eq!(
        view.get(HEADERS, k_delete)?,
        Some(b"X".to_vec()),
        "deleted pre-existing key not restored"
    );
    assert!(
        view.get(HEADERS, k_new)?.is_none(),
        "newly-created key should be gone after undo"
    );
    assert_eq!(
        pristine,
        state_digest(backend.as_ref())?,
        "digest not restored to pristine after undo"
    );
    Ok(())
}
