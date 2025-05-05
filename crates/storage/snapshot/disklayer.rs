use ethrex_common::H256;
use ethrex_trie::{Trie, TrieDB};

use crate::{cache::Cache, Store};

use super::layer::SnapshotLayer;


pub struct DiskLayer {
    trie_db: Trie,
    cache: Cache,
    root: H256,
    stale: bool,
}

impl SnapshotLayer for DiskLayer {
    fn root(&self) -> H256 {
        self.root
    }

    fn get_account(&self, hash: H256) -> Option<ethrex_common::types::AccountState> {
        self.store.get_account_state_from_trie(state_trie, address)
    }

    fn get_account_rlp(&self, hash: H256) -> Option<crate::rlp::AccountStateRLP> {
        todo!()
    }

    fn get_storage(&self, account_hash: H256, storage_hash: H256) -> Option<Vec<u8>> {
        todo!()
    }

    fn parent(&self) -> Option<Box<dyn SnapshotLayer>> {
        todo!()
    }

    fn update(
        &self,
        block: H256,
        accounts: std::collections::HashMap<H256, ethrex_common::types::AccountState>,
        storage: std::collections::HashMap<H256, std::collections::HashMap<H256, Vec<u8>>>,
    ) -> Box<dyn SnapshotLayer> {
        todo!()
    }
}
