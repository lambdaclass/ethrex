use std::{alloc::System, time::SystemTime};

use ethrex::initializers::{init_store, open_store};
use ethrex_common::{
    constants::EMPTY_TRIE_HASH,
    types::{AccountState, Genesis},
};
use ethrex_p2p::sync::SyncError;
use ethrex_rlp::{decode::RLPDecode, encode::RLPEncode};
use ethrex_storage::Store;
use keccak_hash::H256;

fn insert_accounts_into_db(store: Store) -> Result<(), SyncError> {
    let mut computed_state_root = *EMPTY_TRIE_HASH;
    for entry in std::fs::read_dir("home/admin/.local/share/ethrex/account_snapshot_dir")
        .map_err(|_| SyncError::AccountStateSnapshotsDirNotFound)?
    {
        let entry = entry.map_err(|err| {
            SyncError::SnapshotReadError(
                "home/admin/.local/share/ethrex/account_snapshot_dir"
                    .clone()
                    .into(),
                err,
            )
        })?;
        println!("Reading account file from entry {entry:?}");
        let snapshot_path = entry.path();
        let snapshot_contents = std::fs::read(&snapshot_path)
            .map_err(|err| SyncError::SnapshotReadError(snapshot_path.clone(), err))?;
        let account_states_snapshot: Vec<(H256, AccountState)> =
            RLPDecode::decode(&snapshot_contents)
                .map_err(|_| SyncError::SnapshotDecodeError(snapshot_path.clone()))?;

        let (account_hashes, account_states): (Vec<H256>, Vec<AccountState>) =
            account_states_snapshot.iter().cloned().unzip();

        println!("Inserting accounts into the state trie");

        let store_clone = store.clone();
        let current_state_root: Result<H256, SyncError> = {
            let mut trie = store_clone.open_state_trie(computed_state_root)?;

            for (account_hash, account) in account_states_snapshot {
                trie.insert(account_hash.0.to_vec(), account.encode_to_vec())?;
            }
            let current_state_root = trie.hash()?;
            Ok(current_state_root)
        };

        computed_state_root = current_state_root?;
    }
    Ok(())
}

fn main() {
    println!("{:?}", SystemTime::now());
    let store: Store = open_store("home/admin/.local/share/ethrex/");
    let _ = insert_accounts_into_db(store).inspect_err(|err| println!("We had the error {err:?}"));
    println!("{:?}", SystemTime::now());
}
