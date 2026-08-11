//! The flat state a snap/2 sync accumulates before it rebuilds the tries.
//!
//! From devp2p `caps/snap.md`, "Synchronization algorithm":
//!
//! > In isolation, this process would not result in a consistent state because
//! > the resulting state is a sequence of key-value pairs from states `R₀`,
//! > `R₁`, ... `Rₙ`. To make it consistent with the final root `Rₙ`, the state
//! > has to be patched using BALs
//! >
//! > Once the flat state is consistent with the latest pivot, reconstruct all
//! > tries locally and verify the resulting root against the last header.
//!
//! Patching means reading a leaf the download already produced, changing it,
//! and writing it back, so the download's output has to stay addressable for
//! the whole sync rather than being consumed once at the end. Range responses
//! still arrive as sorted chunk files and are absorbed in bulk; access-list
//! diffs are applied on top as individual writes.
//!
//! Absorbing a chunk cannot clobber a diff already written. A chunk only ever
//! covers keys the download had not reached, and
//! [`super::DownloadCursor`] refuses to patch exactly those.

use std::path::Path;

use ethrex_common::{H256, U256, types::AccountState};
use ethrex_rlp::{decode::RLPDecode, encode::RLPEncode};

use crate::snap::async_fs;
use crate::sync::SyncError;

/// Key of a storage slot: the account hash followed by the slot hash, the same
/// layout the download's chunk files use.
fn slot_key(account_hash: H256, slot_hash: H256) -> [u8; 64] {
    let mut key = [0u8; 64];
    key[..32].copy_from_slice(&account_hash.0);
    key[32..].copy_from_slice(&slot_hash.0);
    key
}

#[cfg(feature = "rocksdb")]
pub use rocksdb_impl::FlatState;

#[cfg(not(feature = "rocksdb"))]
pub use memory_impl::FlatState;

#[cfg(feature = "rocksdb")]
mod rocksdb_impl {
    use super::*;
    use crate::utils::{get_rocksdb_temp_accounts_dir, get_rocksdb_temp_storage_dir};
    use std::path::PathBuf;

    /// Flat state held in two temporary column stores, one keyed by account
    /// hash and one by account hash followed by slot hash.
    pub struct FlatState {
        accounts: rocksdb::DB,
        storages: rocksdb::DB,
        accounts_dir: PathBuf,
        storages_dir: PathBuf,
    }

    impl FlatState {
        pub fn open(datadir: &Path) -> Result<Self, SyncError> {
            let accounts_dir = get_rocksdb_temp_accounts_dir(datadir);
            let storages_dir = get_rocksdb_temp_storage_dir(datadir);
            let mut options = rocksdb::Options::default();
            options.create_if_missing(true);
            let accounts = rocksdb::DB::open(&options, &accounts_dir)
                .map_err(|e| SyncError::AccountTempDBDirNotFound(e.to_string()))?;
            let storages = rocksdb::DB::open(&options, &storages_dir)
                .map_err(|e| SyncError::RocksDBError(e.into_string()))?;
            Ok(Self {
                accounts,
                storages,
                accounts_dir,
                storages_dir,
            })
        }

        /// Absorb every finished account chunk in `dir`.
        pub async fn absorb_account_chunks(&self, dir: &Path) -> Result<(), SyncError> {
            let paths = async_fs::read_dir_paths(dir).await?;
            ingest(&self.accounts, paths)
        }

        /// Absorb every finished storage chunk in `dir`.
        pub async fn absorb_storage_chunks(&self, dir: &Path) -> Result<(), SyncError> {
            let paths = async_fs::read_dir_paths(dir).await?;
            ingest(&self.storages, paths)
        }

        pub fn get_account(&self, account_hash: H256) -> Result<Option<AccountState>, SyncError> {
            match self
                .accounts
                .get(account_hash.0)
                .map_err(|e| SyncError::RocksDBError(e.into_string()))?
            {
                Some(encoded) => Ok(Some(AccountState::decode(&encoded)?)),
                None => Ok(None),
            }
        }

        pub fn put_account(
            &self,
            account_hash: H256,
            account: &AccountState,
        ) -> Result<(), SyncError> {
            self.accounts
                .put(account_hash.0, account.encode_to_vec())
                .map_err(|e| SyncError::RocksDBError(e.into_string()))
        }

        pub fn delete_account(&self, account_hash: H256) -> Result<(), SyncError> {
            self.accounts
                .delete(account_hash.0)
                .map_err(|e| SyncError::RocksDBError(e.into_string()))
        }

        pub fn get_slot(
            &self,
            account_hash: H256,
            slot_hash: H256,
        ) -> Result<Option<U256>, SyncError> {
            match self
                .storages
                .get(slot_key(account_hash, slot_hash))
                .map_err(|e| SyncError::RocksDBError(e.into_string()))?
            {
                Some(encoded) => Ok(Some(U256::decode(&encoded)?)),
                None => Ok(None),
            }
        }

        pub fn put_slot(
            &self,
            account_hash: H256,
            slot_hash: H256,
            value: U256,
        ) -> Result<(), SyncError> {
            self.storages
                .put(slot_key(account_hash, slot_hash), value.encode_to_vec())
                .map_err(|e| SyncError::RocksDBError(e.into_string()))
        }

        pub fn delete_slot(&self, account_hash: H256, slot_hash: H256) -> Result<(), SyncError> {
            self.storages
                .delete(slot_key(account_hash, slot_hash))
                .map_err(|e| SyncError::RocksDBError(e.into_string()))
        }

        /// Every account in hash order, as the RLP the trie build consumes.
        pub fn iter_accounts(
            &self,
        ) -> impl Iterator<Item = Result<(H256, Vec<u8>), SyncError>> + '_ {
            self.accounts
                .full_iterator(rocksdb::IteratorMode::Start)
                .map(|entry| {
                    let (key, value) =
                        entry.map_err(|e| SyncError::RocksDBError(e.into_string()))?;
                    Ok((H256::from_slice(&key), value.to_vec()))
                })
        }

        /// One account's slots in hash order, as the RLP the trie build
        /// consumes.
        pub fn iter_slots(&self, account_hash: H256) -> impl Iterator<Item = (H256, Vec<u8>)> + '_ {
            let mut iter = self.storages.raw_iterator();
            iter.seek(slot_key(account_hash, H256::zero()));
            SlotIter { iter, account_hash }
        }

        /// Drop the flat state and the space it occupies. Called once the tries
        /// are built and verified.
        pub async fn destroy(self) -> Result<(), SyncError> {
            let Self {
                accounts,
                storages,
                accounts_dir,
                storages_dir,
            } = self;
            drop(accounts);
            drop(storages);
            async_fs::remove_dir_all(&accounts_dir).await?;
            async_fs::remove_dir_all(&storages_dir).await?;
            Ok(())
        }
    }

    /// Move the chunk files into `db`. Ingesting rather than copying keeps a
    /// single on-disk copy of the leaf data; the chunk dir and the store live
    /// under the same datadir, so the rename succeeds.
    fn ingest(db: &rocksdb::DB, paths: Vec<PathBuf>) -> Result<(), SyncError> {
        if paths.is_empty() {
            return Ok(());
        }
        let mut options = rocksdb::IngestExternalFileOptions::default();
        options.set_move_files(true);
        db.ingest_external_file_opts(&options, paths)
            .map_err(|e| SyncError::RocksDBError(e.into_string()))
    }

    struct SlotIter<'a> {
        iter: rocksdb::DBRawIterator<'a>,
        account_hash: H256,
    }

    impl Iterator for SlotIter<'_> {
        type Item = (H256, Vec<u8>);

        fn next(&mut self) -> Option<Self::Item> {
            let (key, value) = match (self.iter.key(), self.iter.value()) {
                (Some(key), Some(value)) if key.len() == 64 => (key, value),
                _ => return None,
            };
            if key[..32] != self.account_hash.0 {
                return None;
            }
            let slot = (H256::from_slice(&key[32..]), value.to_vec());
            self.iter.next();
            Some(slot)
        }
    }
}

#[cfg(not(feature = "rocksdb"))]
mod memory_impl {
    use super::*;
    use crate::utils::AccountsWithStorage;
    use std::collections::BTreeMap;
    use std::sync::{RwLock, RwLockReadGuard, RwLockWriteGuard};

    /// A storage chunk file as written by the download: groups of accounts
    /// sharing a storage root, each with that root's slots.
    type StorageChunk = Vec<(Vec<H256>, Vec<(H256, U256)>)>;

    /// Flat state held in memory, for builds without the on-disk backend.
    ///
    /// The stores are behind locks so that the API matches the on-disk one,
    /// which takes `&self` throughout because the backend does its own
    /// synchronization. The download shares one flat state across its workers
    /// and cannot hand out `&mut`.
    #[derive(Default)]
    pub struct FlatState {
        accounts: RwLock<BTreeMap<H256, Vec<u8>>>,
        storages: RwLock<BTreeMap<[u8; 64], Vec<u8>>>,
    }

    impl FlatState {
        pub fn open(_datadir: &Path) -> Result<Self, SyncError> {
            Ok(Self::default())
        }

        pub async fn absorb_account_chunks(&self, dir: &Path) -> Result<(), SyncError> {
            for path in async_fs::read_dir_paths(dir).await? {
                let contents = async_fs::read_file(&path).await?;
                let chunk: Vec<(H256, AccountState)> = RLPDecode::decode(&contents)
                    .map_err(|_| SyncError::SnapshotDecodeError(path.clone()))?;
                let mut accounts = write(&self.accounts)?;
                for (account_hash, account) in chunk {
                    accounts.insert(account_hash, account.encode_to_vec());
                }
                drop(accounts);
                async_fs::remove_file(&path).await?;
            }
            Ok(())
        }

        pub async fn absorb_storage_chunks(&self, dir: &Path) -> Result<(), SyncError> {
            for path in async_fs::read_dir_paths(dir).await? {
                let contents = async_fs::read_file(&path).await?;
                let chunk: Vec<AccountsWithStorage> = RLPDecode::decode(&contents)
                    .map(|all: StorageChunk| {
                        all.into_iter()
                            .map(|(accounts, storages)| AccountsWithStorage { accounts, storages })
                            .collect()
                    })
                    .map_err(|_| SyncError::SnapshotDecodeError(path.clone()))?;
                let mut storages = write(&self.storages)?;
                for entry in chunk {
                    for account_hash in entry.accounts {
                        for (slot_hash, value) in &entry.storages {
                            storages
                                .insert(slot_key(account_hash, *slot_hash), value.encode_to_vec());
                        }
                    }
                }
                drop(storages);
                async_fs::remove_file(&path).await?;
            }
            Ok(())
        }

        pub fn get_account(&self, account_hash: H256) -> Result<Option<AccountState>, SyncError> {
            match read(&self.accounts)?.get(&account_hash) {
                Some(encoded) => Ok(Some(AccountState::decode(encoded)?)),
                None => Ok(None),
            }
        }

        pub fn put_account(
            &self,
            account_hash: H256,
            account: &AccountState,
        ) -> Result<(), SyncError> {
            write(&self.accounts)?.insert(account_hash, account.encode_to_vec());
            Ok(())
        }

        pub fn delete_account(&self, account_hash: H256) -> Result<(), SyncError> {
            write(&self.accounts)?.remove(&account_hash);
            Ok(())
        }

        pub fn get_slot(
            &self,
            account_hash: H256,
            slot_hash: H256,
        ) -> Result<Option<U256>, SyncError> {
            match read(&self.storages)?.get(&slot_key(account_hash, slot_hash)) {
                Some(encoded) => Ok(Some(U256::decode(encoded)?)),
                None => Ok(None),
            }
        }

        pub fn put_slot(
            &self,
            account_hash: H256,
            slot_hash: H256,
            value: U256,
        ) -> Result<(), SyncError> {
            write(&self.storages)?.insert(slot_key(account_hash, slot_hash), value.encode_to_vec());
            Ok(())
        }

        pub fn delete_slot(&self, account_hash: H256, slot_hash: H256) -> Result<(), SyncError> {
            write(&self.storages)?.remove(&slot_key(account_hash, slot_hash));
            Ok(())
        }

        /// Every account in hash order, as the RLP the trie build consumes.
        ///
        /// Snapshots the store rather than holding the lock across the trie
        /// build, which writes as it goes.
        pub fn iter_accounts(&self) -> impl Iterator<Item = Result<(H256, Vec<u8>), SyncError>> {
            match read(&self.accounts) {
                Ok(accounts) => accounts
                    .iter()
                    .map(|(hash, encoded)| Ok((*hash, encoded.clone())))
                    .collect::<Vec<_>>(),
                Err(err) => vec![Err(err)],
            }
            .into_iter()
        }

        /// One account's slots in hash order, as the RLP the trie build
        /// consumes.
        pub fn iter_slots(&self, account_hash: H256) -> impl Iterator<Item = (H256, Vec<u8>)> {
            let Ok(storages) = read(&self.storages) else {
                return Vec::new().into_iter();
            };
            storages
                .range(slot_key(account_hash, H256::zero())..)
                .take_while(|(key, _)| key[..32] == account_hash.0)
                .map(|(key, value)| (H256::from_slice(&key[32..]), value.clone()))
                .collect::<Vec<_>>()
                .into_iter()
        }

        pub async fn destroy(self) -> Result<(), SyncError> {
            Ok(())
        }
    }

    fn read<T>(lock: &RwLock<T>) -> Result<RwLockReadGuard<'_, T>, SyncError> {
        lock.read().map_err(|_| SyncError::FlatStatePoisoned)
    }

    fn write<T>(lock: &RwLock<T>) -> Result<RwLockWriteGuard<'_, T>, SyncError> {
        lock.write().map_err(|_| SyncError::FlatStatePoisoned)
    }
}
