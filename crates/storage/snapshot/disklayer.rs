use std::sync::Arc;

use ethrex_common::{types::AccountState, H256, U256};
use ethrex_rlp::decode::RLPDecode;
use ethrex_trie::{Trie, TrieDB};
use libmdbx::orm::Encodable;

use crate::{cache::Cache, hash_key, rlp::AccountStateRLP, Store};

use super::layer::SnapshotLayer;

pub struct DiskLayer {
    state_trie: Trie,
    store: Store,
    cache: Cache,
    root: H256,
    stale: bool,
}

impl SnapshotLayer for DiskLayer {
    fn root(&self) -> H256 {
        self.root
    }

    fn get_account(&self, hash: H256) -> Option<AccountState> {
        let value = self.get_account_rlp(hash)?;

        AccountState::decode(value.bytes()).ok()
    }

    fn get_account_rlp(&self, hash: H256) -> Option<AccountStateRLP> {
        if let Some(value) = self.cache.accounts_rlp.get(&hash) {
            return Some((*value).clone());
        }

        let value = self
            .state_trie
            .get(&hash)
            .ok()
            .flatten()
            .map(|x| AccountStateRLP::from_bytes(x));

        if let Some(value) = &value {
            self.cache
                .accounts_rlp
                .insert(hash, Arc::new(value.clone()));
        }

        value
    }

    fn get_storage(&self, account_hash: H256, storage_hash: H256) -> Option<U256> {
        if let Some(value) = self.cache.storages.get(&(account_hash, storage_hash)) {
            return Some(value);
        }

        let account = self.get_account(account_hash)?;

        let storage_trie = self
            .store
            .open_storage_trie(account_hash, account.storage_root);
        let value: U256 = U256::decode(&storage_trie.get(storage_hash).ok().flatten()?).ok()?;

        self.cache
            .storages
            .insert((account_hash, storage_hash), value);

        Some(value)
    }

    fn parent(&self) -> Option<Box<dyn SnapshotLayer>> {
        None
    }

    fn update(
        &self,
        block: H256,
        accounts: std::collections::HashMap<H256, ethrex_common::types::AccountState>,
        storage: std::collections::HashMap<H256, std::collections::HashMap<H256, Vec<u8>>>,
    ) -> Box<dyn SnapshotLayer> {
        todo!()
    }

    fn stale(&self) -> bool {
        self.stale
    }
}
