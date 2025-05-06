use std::{
    cell::OnceCell,
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, LazyLock, OnceLock,
    },
};

use ethrex_common::{types::AccountState, Bloom, BloomInput, H256, U256};

use super::{DiskLayer, SnapshotLayer};

#[derive(Clone)]
pub struct DiffLayer {
    origin: Arc<DiskLayer>,
    parent: Arc<dyn SnapshotLayer>,
    root: H256,
    stale: Arc<AtomicBool>,
    accounts: Arc<HashMap<H256, AccountState>>,
    storage: Arc<HashMap<H256, HashMap<H256, U256>>>,
    /// tracks all diffed items up to disk layer
    diffed: Bloom,
}

// TODO: make this random
// range 0:24
static BLOOM_ACCOUNT_HASHER_OFFSET: usize = 0;
static BLOOM_STORAGE_HASHER_OFFSET: usize = 10;

fn account_bloom(hash: H256) -> u64 {
    let value: [u8; 8] = hash.0[BLOOM_ACCOUNT_HASHER_OFFSET..(BLOOM_ACCOUNT_HASHER_OFFSET + 8)]
        .try_into()
        .unwrap();
    u64::from_le_bytes(value)
}

fn storage_bloom(hash: H256) -> u64 {
    let value: [u8; 8] = hash.0[BLOOM_STORAGE_HASHER_OFFSET..(BLOOM_STORAGE_HASHER_OFFSET + 8)]
        .try_into()
        .unwrap();
    u64::from_le_bytes(value)
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
            diffed: Bloom::zero(),
        };

        layer
    }
}

impl DiffLayer {
    pub fn rebloom(&mut self) {
        self.diffed = self.parent.diffed().unwrap_or_default();

        for hash in self.accounts.keys() {
            self.diffed.accrue(BloomInput::Hash(hash.as_fixed_bytes()));
        }

        for (hash, slots) in self.storage.iter() {
            for slot in slots.keys() {
                let value = hash ^ slot;
                self.diffed.accrue(BloomInput::Hash(value.as_fixed_bytes()));
            }
        }
    }

    fn get_account_depth(&self, hash: H256, depth: usize) -> Option<AccountState> {
        todo!()
    }
}

impl SnapshotLayer for DiffLayer {
    fn root(&self) -> H256 {
        self.root
    }

    fn diffed(&self) -> Option<Bloom> {
        Some(self.diffed)
    }

    fn get_account(&self, hash: H256) -> Option<AccountState> {
        // todo: check stale

        let hit = self
            .diffed
            .contains_input(BloomInput::Hash(hash.as_fixed_bytes()));

        // If bloom misses we can skip diff layers
        if !hit {
            return self.origin.get_account(hash);
        }

        // Start traversing layers.
        self.get_account_depth(hash, 0)
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
