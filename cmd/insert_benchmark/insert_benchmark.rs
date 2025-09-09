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
use std::path::PathBuf;
use std::{alloc::System, time::SystemTime};
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

#[tokio::main]
async fn main() {
    let mut opts = Options::default();
    //opts.log_level = tracing::Level::INFO;
    init_tracing(&opts);
    info!("Starting");
    let store: Store = open_store("/home/admin/.local/share/benchmarks/");
    let _ = insert_accounts_into_db(store)
        .await
        .inspect_err(|err| error!("We had the error {err:?}"));
    info!("finishing");
}
