use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

use ethrex_common::{types::AccountState, H256, U256};

use super::{DiskLayer, SnapshotLayer};

#[derive(Clone)]
pub struct DiffLayer {
    origin: Arc<DiskLayer>,
    parent: Arc<dyn SnapshotLayer>,
    root: H256,
    stale: Arc<AtomicBool>,
    accounts: Arc<HashMap<H256, AccountState>>,
    storage: Arc<HashMap<H256, HashMap<H256, U256>>>,
}

impl DiffLayer {
    pub fn new(
        parent: Arc<dyn SnapshotLayer>,
        root: H256,
        accounts: HashMap<H256, AccountState>,
        storage: HashMap<H256, HashMap<H256, U256>>,
    ) -> Self {
        let layer = DiffLayer {
            origin: parent.origin(),
            parent,
            root,
            stale: AtomicBool::new(false).into(),
            accounts: Arc::new(accounts),
            storage: Arc::new(storage),
        };

        layer
    }
}

impl SnapshotLayer for DiffLayer {
    fn root(&self) -> H256 {
        todo!()
    }

    fn get_account(&self, hash: H256) -> Option<ethrex_common::types::AccountState> {
        todo!()
    }

    fn get_storage(
        &self,
        account_hash: H256,
        storage_hash: ethrex_common::H256,
    ) -> Option<ethrex_common::U256> {
        todo!()
    }

    fn stale(&self) -> bool {
        self.stale.load(Ordering::Acquire)
    }

    fn parent(&self) -> Option<Arc<dyn SnapshotLayer>> {
        Some(self.parent.clone())
    }

    fn update(
        &self,
        block: ethrex_common::H256,
        accounts: HashMap<H256, AccountState>,
        storage: HashMap<H256, HashMap<H256, U256>>,
    ) -> Arc<dyn SnapshotLayer> {
        todo!()
    }

    fn origin(&self) -> Arc<DiskLayer> {
        self.origin.clone()
    }
}
