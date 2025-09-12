pub mod speedup;

use ethrex::cli::Options;
use ethrex::initializers::init_tracing;
use ethrex_common::{
    constants::EMPTY_TRIE_HASH,
    types::{AccountState, Genesis},
};
use ethrex_p2p::sync::SyncError;
use ethrex_rlp::{decode::RLPDecode, encode::RLPEncode};
use ethrex_storage::EngineType;
use ethrex_storage::Store;
use keccak_hash::H256;
use rocksdb::SstFileWriter;
use std::{alloc::System, time::SystemTime};
use std::{hash, path::PathBuf};
use tracing::{error, info};

/// Opens a pre-existing Store or creates a new one
pub fn open_store(data_dir: &str) -> Store {
    let path = PathBuf::from(data_dir);
    if path.ends_with("memory") {
        Store::new(data_dir, EngineType::InMemory).expect("Failed to create Store")
    } else {
        cfg_if::cfg_if! {
            if #[cfg(feature = "rocksdb")] {
                let engine_type = EngineType::RocksDB;
            } else if #[cfg(feature = "libmdbx")] {
                let engine_type = EngineType::Libmdbx;
            } else {
                error!("No database specified. The feature flag `rocksdb` or `libmdbx` should've been set while building.");
                panic!("Specify the desired database engine.");
            }
        };
        Store::new(data_dir, engine_type).expect("Failed to create Store")
    }
}

async fn insert_accounts_into_db(store: Store) -> Result<(), SyncError> {
    let mut computed_state_root = *EMPTY_TRIE_HASH;
    for entry in std::fs::read_dir("/home/admin/.local/share/ethrex/account_state_snapshots")
        .map_err(|_| SyncError::AccountStateSnapshotsDirNotFound)?
    {
        let entry = entry.map_err(|err| {
            SyncError::SnapshotReadError(
                "/home/admin/.local/share/ethrex/account_state_snapshots"
                    .clone()
                    .into(),
                err,
            )
        })?;
        info!("Reading account file from entry {entry:?}");
        let snapshot_path = entry.path();
        let snapshot_contents = std::fs::read(&snapshot_path)
            .map_err(|err| SyncError::SnapshotReadError(snapshot_path.clone(), err))?;
        let mut account_states_snapshot: Vec<(H256, AccountState)> =
            RLPDecode::decode(&snapshot_contents)
                .map_err(|_| SyncError::SnapshotDecodeError(snapshot_path.clone()))?;

        account_states_snapshot.sort_by_key(|(k, _)| *k);

        info!("Inserting accounts into the state trie");

        let store_clone = store.clone();
        let current_state_root: Result<H256, SyncError> =
            tokio::task::spawn_blocking(move || -> Result<H256, SyncError> {
                let mut trie = store_clone.open_state_trie(computed_state_root)?;

                for (account_hash, account) in account_states_snapshot {
                    trie.insert(account_hash.0.to_vec(), account.encode_to_vec())?;
                }
                info!("Comitting to disk");
                let current_state_root = trie.hash()?;
                Ok(current_state_root)
            })
            .await?;

        computed_state_root = current_state_root?;
    }
    Ok(())
}

async fn insert_accounts_into_db_v2() -> Result<(), SyncError> {
    let mut db_options = rocksdb::Options::default();
    db_options.create_if_missing(true);
    let db = rocksdb::DB::open(&db_options, "/home/admin/.local/share/snapshot").unwrap();
    let mut computed_state_root = *EMPTY_TRIE_HASH;
    let file_paths: Vec<PathBuf> = std::fs::read_dir("/home/admin/.local/share/ethrex/account_sst")
        .map_err(|_| SyncError::AccountStateSnapshotsDirNotFound)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| SyncError::AccountStateSnapshotsDirNotFound)?
        .into_iter()
        .map(|res| res.path())
        .collect();
    db.ingest_external_file(file_paths);
    let iter = db.full_iterator(rocksdb::IteratorMode::Start);
    for (key, value) in iter.map(|f| f.unwrap()) {
        let hash = H256::from_slice(&key);
        let account_state = AccountState::decode(&value).unwrap();

        println!("Key {hash} Value {account_state:?}");
        break;
    }
    Ok(())
}

async fn setup_files() -> Result<(), SyncError> {
    let writer_options = rocksdb::Options::default();
    let mut writer = SstFileWriter::create(&writer_options);
    let mut count = 0;
    for entry in std::fs::read_dir("/home/admin/.local/share/ethrex/account_state_snapshots")
        .map_err(|_| SyncError::AccountStateSnapshotsDirNotFound)?
    {
        let entry = entry.map_err(|err| {
            SyncError::SnapshotReadError(
                "/home/admin/.local/share/ethrex/account_state_snapshots"
                    .clone()
                    .into(),
                err,
            )
        })?;
        info!("Reading account file from entry {entry:?}");
        let snapshot_path = entry.path();
        let snapshot_contents = std::fs::read(&snapshot_path)
            .map_err(|err| SyncError::SnapshotReadError(snapshot_path.clone(), err))?;
        let mut account_states_snapshot: Vec<(H256, AccountState)> =
            RLPDecode::decode(&snapshot_contents)
                .map_err(|_| SyncError::SnapshotDecodeError(snapshot_path.clone()))?;

        account_states_snapshot.sort_by(|(hash_a, _), (hash_b, _)| hash_a.cmp(hash_b));
        writer
            .open(std::path::Path::new(&format!(
                "/home/admin/.local/share/ethrex/account_sst/account_{count}.sst"
            )))
            .expect("Failed to open file");
        for account in account_states_snapshot {
            writer
                .put(account.0, account.1.encode_to_vec())
                .expect("Failed to put file");
        }
        writer.finish().expect("Failed to finish file");
        count += 1;
    }
    Ok(())
}

async fn sub_main() {
    info!("Starting");
    let store: Store = open_store("/home/admin/.local/share/benchmarks/");
    let _ = insert_accounts_into_db(store)
        .await
        .inspect_err(|err| error!("We had the error {err:?}"));
}

#[tokio::main]
async fn main() {
    let opts = Options::default();
    init_tracing(&opts);
    insert_accounts_into_db_v2().await;
    info!("finishing");
}
