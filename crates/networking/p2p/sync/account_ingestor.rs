//! Streaming SST ingestor for account snapshot files.
//!
//! During the account-range download the dispatcher dumps each completed
//! chunk of accounts to an SST file. Previously every file was ingested into
//! the temporary RocksDB only after the whole download finished; this module
//! ingests each file as soon as it is written, overlapping the ingest I/O
//! with the rest of the download. The sorted trie build still runs once, in
//! `insert_accounts`, after both the download and this task complete.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use tokio::task::JoinHandle;
use tracing::warn;

use crate::snap::async_fs;
use crate::sync::SyncError;
use crate::utils::get_rocksdb_temp_accounts_dir;

/// Handle to the background ingest task; joining it yields the temp DB with
/// every file received over the channel already ingested.
pub(super) type AccountIngestHandle = JoinHandle<Result<rocksdb::DB, SyncError>>;

/// Opens (or creates) the temporary RocksDB the account snapshot files are
/// ingested into.
pub(super) fn open_temp_accounts_db(datadir: &Path) -> Result<rocksdb::DB, SyncError> {
    let mut db_options = rocksdb::Options::default();
    db_options.create_if_missing(true);
    // Throwaway ingest DB read once to build the account trie. Bulk-load mode
    // disables auto compaction and lifts the L0 stall/stop triggers, so
    // incrementally ingesting many overlapping SST files cannot build a
    // compaction backlog and throttle writes (see open_temp_storage_db). The
    // reader (`insert_accounts`) compacts once after ingestion, before the
    // build scan, so it isn't merging across thousands of L0 files per step.
    db_options.prepare_for_bulk_load();
    db_options.create_if_missing(true);
    rocksdb::DB::open(&db_options, get_rocksdb_temp_accounts_dir(datadir))
        .map_err(|e| SyncError::AccountTempDBDirNotFound(e.to_string()))
}

/// Ingests a single account snapshot file into the temp DB, moving it out of
/// the snapshot dir (so the files remaining there are exactly the ones not
/// yet ingested).
pub(super) fn ingest_snapshot_file(db: &rocksdb::DB, path: &Path) -> Result<(), SyncError> {
    // An empty chunk produces no file at all (`dump_accounts_to_rocks_db`
    // skips empty contents because RocksDB rejects empty SSTs); skip it
    // while keeping the chunk sequence contiguous.
    if !path.exists() {
        return Ok(());
    }
    let mut ingest_opts = rocksdb::IngestExternalFileOptions::default();
    ingest_opts.set_move_files(true);
    db.ingest_external_file_opts(&ingest_opts, vec![path.to_path_buf()])
        .map_err(|err| SyncError::RocksDBError(err.into_string()))
}

/// Ingests any snapshot files still sitting in the snapshot dir, in
/// ascending chunk order. Ingested files were moved out of the dir, so what
/// remains is exactly the set the ingest task never consumed: chunks whose
/// send failed because the task had already died, or leftovers from a
/// previous run.
pub(super) async fn ingest_remaining_snapshot_files(
    db: &rocksdb::DB,
    account_state_snapshots_dir: &Path,
) -> Result<(), SyncError> {
    let mut leftover_files = async_fs::read_dir_paths(account_state_snapshots_dir).await?;
    if leftover_files.is_empty() {
        return Ok(());
    }
    // Ascending chunk order preserves last-write-wins for accounts that were
    // re-delivered with a newer value after a pivot update. The chunk index
    // is the trailing `.{index}` of the file name; `read_dir_paths` sorts
    // lexicographically, which misorders multi-digit indices.
    leftover_files.sort_by_key(|path| chunk_index_of(path));
    warn!(
        count = leftover_files.len(),
        "Ingesting account snapshot files left in the snapshot dir"
    );
    for path in leftover_files {
        ingest_snapshot_file(db, &path)?;
    }
    Ok(())
}

fn chunk_index_of(path: &Path) -> u64 {
    path.extension()
        .and_then(|index| index.to_str())
        .and_then(|index| index.parse().ok())
        // Files without a parseable index sort last and keep their relative
        // lexicographic order (the sort is stable).
        .unwrap_or(u64::MAX)
}

/// Spawns the background ingest task that owns the temporary accounts
/// RocksDB. Send each finished account snapshot file over the returned
/// channel as `(chunk_index, path)`; drop the sender once the download is
/// done, then await the handle to obtain the temp DB with every received
/// file ingested.
pub(super) fn spawn_account_snapshot_ingestor(
    datadir: &Path,
) -> (UnboundedSender<(u64, PathBuf)>, AccountIngestHandle) {
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    let datadir = datadir.to_path_buf();
    // `ingest_external_file` blocks on disk I/O, so the whole loop runs on a
    // blocking thread instead of a tokio worker.
    let handle = tokio::task::spawn_blocking(move || run(&datadir, rx));
    (tx, handle)
}

fn run(
    datadir: &Path,
    mut file_rx: UnboundedReceiver<(u64, PathBuf)>,
) -> Result<rocksdb::DB, SyncError> {
    let db = open_temp_accounts_db(datadir)?;

    // The same account hash can appear in more than one file with different
    // values: a chunk re-downloaded after a pivot update delivers the newer
    // value in a later file. RocksDB resolves duplicate keys across ingested
    // files by ingestion recency, so files must be ingested in ascending
    // chunk order. Dump tasks run concurrently and can finish (and send) out
    // of order, so buffer arrivals and only ingest the contiguous prefix.
    let mut pending: BTreeMap<u64, PathBuf> = BTreeMap::new();
    let mut next_chunk: u64 = 0;
    while let Some((chunk_index, path)) = file_rx.blocking_recv() {
        pending.insert(chunk_index, path);
        while let Some(path) = pending.remove(&next_chunk) {
            ingest_snapshot_file(&db, &path)?;
            next_chunk += 1;
        }
    }

    // Channel closed: ingest whatever is still buffered, in ascending chunk
    // order. A gap before a buffered chunk can only come from a dump task
    // that failed, in which case the missing file was never written and the
    // download itself errors out.
    for (_, path) in pending {
        ingest_snapshot_file(&db, &path)?;
    }
    Ok(db)
}
