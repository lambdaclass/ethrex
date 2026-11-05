//! Pre-sealing a payload build: the state tree of the held build is kept up to
//! date while the build loop runs, so the seal only re-hashes what changed
//! since the last update instead of computing a full state root.
//!
//! The tree moves from one best build to the next by diff (`rebase_writes`):
//! builds of one slot share most of their state changes, so the difference
//! between two of them is much smaller than either. Every key the older build
//! touched and the newer one does not goes back to its parent value, and the
//! tree keeps those paths loaded for the rest of the slot.

use std::sync::{
    Arc, Condvar, Mutex, MutexGuard, PoisonError,
    atomic::{AtomicUsize, Ordering},
    mpsc,
};

use ethrex_common::{
    H256, U256,
    types::{AccountDiff, AccountUpdate, StateDiff},
    utils::keccak,
};
use ethrex_storage::Store;
use rustc_hash::FxHashMap;
use tracing::debug;

use crate::{
    error::ChainError,
    sharded_trie::{AccountWrite, ShardedStateTrie},
};

/// What moving a tree from the build it reflects to a target build takes.
#[derive(Debug, Clone, PartialEq)]
pub enum Rebase {
    /// At most one write per account; apply them in any order.
    Writes(Vec<AccountWrite>),
    /// The tree cannot be moved by writes: reopen it at the parent and apply
    /// the target in full. A diff destroyed an account that had storage
    /// (`removed_storage`), whose parent storage cannot be restored slot by
    /// slot, or `held` lacks a parent value that a revert needs.
    Reset,
}

/// The held diff lacks the parent value of a slot it wrote.
struct MissingParentValue;

/// Writes that turn a tree reflecting `held` into one reflecting `target`.
/// A field or slot missing from the target is at its parent value, which
/// `held` carries. `None` means the tree is at the parent state, so the
/// writes are the target's updates as they are, `removed_storage` included.
///
/// `target` holds at most one update per address, as `get_state_transitions`
/// emits them. Only accounts and slots whose value changes get a write: the
/// tree rewrites every account a write names, so a write that changes nothing
/// still costs a leaf.
pub fn rebase_writes<'a>(
    held: Option<&StateDiff>,
    target: impl IntoIterator<Item = &'a AccountUpdate>,
) -> Rebase {
    let Some(held) = held else {
        return Rebase::Writes(target.into_iter().map(AccountWrite::from_update).collect());
    };
    // The VM sets `removed_storage` only for an account that had storage, and
    // then reports zero for the parent value of every slot it did not read,
    // so neither side's slot values say what the tree must go back to.
    if held.values().any(|diff| diff.update.removed_storage) {
        return Rebase::Reset;
    }
    let mut targets: FxHashMap<_, &AccountUpdate> = FxHashMap::default();
    for update in target {
        if update.removed_storage {
            return Rebase::Reset;
        }
        let previous = targets.insert(update.address, update);
        debug_assert!(
            previous.is_none(),
            "two target updates for {}",
            update.address
        );
    }

    let mut writes = Vec::new();
    for (address, update) in &targets {
        let write = match held.get(address) {
            Some(diff) => rebase_account(diff, Some(update)),
            // The tree holds the parent state for an account `held` does not
            // touch, which is what the update is relative to.
            None => Ok(Some(AccountWrite::from_update(update))),
        };
        match write {
            Ok(write) => writes.extend(write),
            Err(MissingParentValue) => return Rebase::Reset,
        }
    }
    for (address, diff) in held {
        if targets.contains_key(address) {
            continue;
        }
        match rebase_account(diff, None) {
            Ok(write) => writes.extend(write),
            Err(MissingParentValue) => return Rebase::Reset,
        }
    }
    Rebase::Writes(writes)
}

/// The write that moves one account from `held` to `target`, or `None` if the
/// tree already holds what the target needs. A `target` of `None` means the
/// target leaves the account at its parent state.
fn rebase_account(
    held: &AccountDiff,
    target: Option<&AccountUpdate>,
) -> Result<Option<AccountWrite>, MissingParentValue> {
    let held_update = &held.update;
    let removed = target.is_some_and(|update| update.removed);
    let held_info = held_update.info.as_ref().unwrap_or(&held.parent_info);
    let target_info = target
        .and_then(|update| update.info.as_ref())
        .unwrap_or(&held.parent_info);
    // A removed account keeps its last written fields in the tree, hidden, and
    // gets them back when a later write makes it exist again. Writing the
    // fields whenever it comes back means the rebase never depends on them.
    let write_info = target_info != held_info || (held_update.removed && !removed);

    let mut storage: Vec<(H256, U256)> = Vec::new();
    if let Some(update) = target {
        for (slot, value) in &update.added_storage {
            // A slot `held` does not write is at its parent value in the
            // tree, which the VM only reports a slot for when it changed.
            if held_update.added_storage.get(slot) != Some(value) {
                storage.push((keccak(slot), *value));
            }
        }
    }
    for (slot, held_value) in &held_update.added_storage {
        if target.is_some_and(|update| update.added_storage.contains_key(slot)) {
            continue;
        }
        let parent_value = held.parent_storage.get(slot).ok_or(MissingParentValue)?;
        if parent_value != held_value {
            storage.push((keccak(slot), *parent_value));
        }
    }

    if !write_info && storage.is_empty() && removed == held_update.removed {
        return Ok(None);
    }
    let mut write = AccountWrite {
        hashed_address: keccak(held_update.address),
        // Every write sets whether the account exists, so each one carries
        // the target's flag, even when only slots change.
        removed,
        storage,
        ..AccountWrite::default()
    };
    if write_info {
        // In full, so nothing the tree kept from an earlier write survives.
        write.nonce = Some(target_info.nonce);
        write.balance = Some(target_info.balance);
        write.code_hash = Some(target_info.code_hash);
        // As `AccountWrite::from_update`: the target's new code goes with its
        // info. A revert needs none, since the parent's code is stored.
        write.code = target.and_then(|update| {
            let info = update.info.as_ref()?;
            update.code.clone().map(|code| (info.code_hash, code))
        });
    }
    Ok(Some(write))
}

/// Keeps a held build's state tree up to date on its own thread, so the seal
/// only pays for what changed since the last update.
///
/// The build loop keeps it on its best build and moves it to each build that
/// beats the best, submitting that build's diff. Dropping it (the build timed
/// out or was evicted) stops the worker, which frees the tree.
///
/// The worker is a plain thread rather than a tokio blocking task or the
/// merkle pool: it lives for the whole slot, and the pool's workers must all
/// run together (see `crate::sharded_trie`).
pub(crate) struct PreSealer {
    shared: Arc<Shared>,
    /// The worker holds the only sender for as long as it runs, so a worker
    /// that panicked or never started shows up here as a closed channel
    /// instead of a reply that never comes.
    replies: mpsc::Receiver<Result<H256, ChainError>>,
}

struct Shared {
    slot: Mutex<Slot>,
    wake: Condvar,
    /// Diffs the worker has applied, so tests can tell a loop that rebased
    /// from one whose PreSealer sat idle.
    diffs_applied: AtomicUsize,
}

#[derive(Default)]
struct Slot {
    /// The newest job not started yet. A newer one replaces it: only the
    /// latest best build is worth merkleizing.
    next: Option<Job>,
    /// The handle is gone. The worker exits without running `next`.
    closed: bool,
}

enum Job {
    Rebase(Arc<StateDiff>),
    /// The seal's final updates. In an `Arc` so the seal keeps them for its
    /// result, and for the one-shot fallback, without a copy.
    Finish(Arc<Vec<AccountUpdate>>),
}

impl Shared {
    /// Every critical section is a single assignment or `take`, so a poisoned
    /// lock still guards a consistent slot; and `Drop` must not panic.
    fn lock(&self) -> MutexGuard<'_, Slot> {
        self.slot.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Queues `job` in place of any job not started yet.
    fn put(&self, job: Job) {
        self.lock().next = Some(job);
        self.wake.notify_one();
    }
}

impl PreSealer {
    /// Starts the worker thread ("payload-preseal"). The tree opens at
    /// `parent_state_root` on the first job.
    pub(crate) fn start(store: Store, parent_state_root: H256) -> std::io::Result<Self> {
        let (preseal, worker) = Self::new(store, parent_state_root);
        worker.spawn()?;
        Ok(preseal)
    }

    /// The handle and its worker, not running yet.
    fn new(store: Store, parent_state_root: H256) -> (Self, Worker) {
        let shared = Arc::new(Shared {
            slot: Mutex::new(Slot::default()),
            wake: Condvar::new(),
            diffs_applied: AtomicUsize::new(0),
        });
        let (replies_tx, replies) = mpsc::channel();
        let worker = Worker {
            shared: shared.clone(),
            replies: replies_tx,
            store,
            parent_state_root,
            tree: None,
            held: None,
            failed: None,
        };
        (Self { shared, replies }, worker)
    }

    /// Makes the tree reflect `diff`. Replaces a pending diff that has not
    /// started. Never waits for the worker, so the async loop can call it.
    pub(crate) fn submit(&self, diff: Arc<StateDiff>) {
        self.shared.put(Job::Rebase(diff));
    }

    /// Rebases the tree onto `updates` and returns its root. Replaces a
    /// pending diff. Blocks until the worker replies: call it from a blocking
    /// thread.
    ///
    /// Fails with the first error the worker hit, or if the worker is gone
    /// (it panicked or never started); the tree is not used then.
    pub(crate) fn finish(self, updates: Arc<Vec<AccountUpdate>>) -> Result<H256, ChainError> {
        self.shared.put(Job::Finish(updates));
        // `self` is dropped only after the reply, so `Drop` cannot close the
        // slot before the worker has taken the job.
        self.replies.recv().map_err(|_| {
            ChainError::Custom("the payload pre-seal worker stopped without a root".to_string())
        })?
    }

    /// Diffs the worker has applied so far.
    pub(crate) fn diffs_applied(&self) -> usize {
        self.shared.diffs_applied.load(Ordering::Relaxed)
    }
}

impl Drop for PreSealer {
    fn drop(&mut self) {
        // No join: this runs on the async loop, and the worker may be in the
        // middle of a rebase. It exits once that one is done.
        self.shared.lock().closed = true;
        self.shared.wake.notify_one();
    }
}

/// The worker side: the tree and the diff it reflects.
struct Worker {
    shared: Arc<Shared>,
    replies: mpsc::Sender<Result<H256, ChainError>>,
    store: Store,
    parent_state_root: H256,
    /// The parent state plus `held`. `None` before the first job and after an
    /// error.
    tree: Option<ShardedStateTrie>,
    /// The diff the tree reflects. `rebase_writes` relies on the tree holding
    /// exactly the parent plus this diff, so it is set only after a diff was
    /// applied and hashed without error.
    held: Option<Arc<StateDiff>>,
    /// The first error. Kept for `finish` rather than retried: a store that
    /// failed once is likely to fail again, and the seal has a fallback.
    failed: Option<ChainError>,
}

impl Worker {
    fn spawn(self) -> std::io::Result<()> {
        std::thread::Builder::new()
            .name("payload-preseal".to_string())
            .spawn(move || self.run())
            .map(drop)
    }

    fn run(mut self) {
        while let Some(job) = self.next_job() {
            match job {
                Job::Rebase(diff) => self.rebase(diff),
                Job::Finish(updates) => {
                    let root = self.finish(&updates);
                    // Before replying, so the seal gets its updates back
                    // without a copy.
                    drop(updates);
                    // The handle waits for this reply unless it panicked.
                    let _ = self.replies.send(root);
                    return;
                }
            }
        }
    }

    /// Waits for the next job, or `None` once the handle is gone.
    fn next_job(&self) -> Option<Job> {
        let mut slot = self.shared.lock();
        loop {
            if slot.closed {
                return None;
            }
            if let Some(job) = slot.next.take() {
                return Some(job);
            }
            slot = self
                .shared
                .wake
                .wait(slot)
                .unwrap_or_else(PoisonError::into_inner);
        }
    }

    fn rebase(&mut self, diff: Arc<StateDiff>) {
        if self.failed.is_some() {
            return;
        }
        match self.move_to(diff.values().map(|diff| &diff.update)) {
            Ok(_) => {
                self.held = Some(diff);
                self.shared.diffs_applied.fetch_add(1, Ordering::Relaxed);
            }
            Err(err) => {
                debug!(%err, "Payload pre-seal failed; the seal will compute the state root in one shot");
                // `move_to` already dropped the tree.
                self.held = None;
                self.failed = Some(err);
            }
        }
    }

    fn finish(&mut self, updates: &[AccountUpdate]) -> Result<H256, ChainError> {
        if let Some(err) = self.failed.take() {
            return Err(err);
        }
        self.move_to(updates)
    }

    /// Makes the tree reflect `target`, a build's full change over the
    /// parent, and returns its root. On error the tree is gone, since it is
    /// then in an unknown state.
    fn move_to<'a>(
        &mut self,
        target: impl IntoIterator<Item = &'a AccountUpdate> + Clone,
    ) -> Result<H256, ChainError> {
        let rebased = match (self.tree.take(), self.held.as_deref()) {
            (Some(tree), Some(held)) => match rebase_writes(Some(held), target.clone()) {
                Rebase::Writes(writes) => Some((tree, writes)),
                // The old tree drops here, before a new one opens: both can
                // be large.
                Rebase::Reset => None,
            },
            _ => None,
        };
        let (mut tree, writes) = match rebased {
            Some(rebased) => rebased,
            None => {
                self.held = None;
                let tree = ShardedStateTrie::open(self.store.clone(), self.parent_state_root)?;
                // A fresh tree is at the parent, which the updates are
                // relative to.
                let writes = target.into_iter().map(AccountWrite::from_update).collect();
                (tree, writes)
            }
        };
        // The tree queues every write's code for `finish()`, which this tree
        // never calls, so code would only pile up with each rebase. The root
        // does not depend on it.
        tree.apply(writes.into_iter().map(|write: AccountWrite| AccountWrite {
            code: None,
            ..write
        }));
        let root = tree.root()?;
        self.tree = Some(tree);
        Ok(root)
    }
}

#[cfg(test)]
mod tests {
    use std::{
        path::Path,
        sync::{Arc, Weak, atomic::Ordering},
        time::{Duration, Instant},
    };

    use ethrex_common::{
        Address, H256, U256,
        seeded_hash::SeededHashMap,
        types::{AccountDiff, AccountInfo, AccountUpdate, BlockHeader, Genesis, StateDiff},
    };
    use ethrex_storage::{EngineType, Store};

    use super::{PreSealer, Shared};
    use crate::error::ChainError;

    /// An in-memory store at the execution-api genesis, and the genesis header.
    async fn genesis_store() -> (Store, BlockHeader) {
        let path = Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../fixtures/genesis/execution-api.json"
        ));
        let genesis = Genesis::try_from(path).expect("load the execution-api genesis");
        let mut store = Store::new("store.db", EngineType::InMemory).expect("in-memory store");
        store
            .add_initial_state(genesis)
            .await
            .expect("add the genesis state");
        let header = store
            .get_block_header(0)
            .expect("read the genesis header")
            .expect("genesis header");
        (store, header)
    }

    /// A build that funds accounts the parent lacks, `(index, balance)`, and
    /// writes the balance into slot 1 of each. Every pair of these builds
    /// differs in accounts the other does not touch, so a rebase between them
    /// must both write and revert.
    fn build(balances: &[(u64, u64)]) -> Arc<StateDiff> {
        let slot = H256::from_low_u64_be(1);
        let diff = balances
            .iter()
            .map(|&(index, balance)| {
                let address = Address::from_low_u64_be(0x1000 + index);
                let mut update = AccountUpdate::new(address);
                update.info = Some(AccountInfo {
                    nonce: 1,
                    balance: balance.into(),
                    ..Default::default()
                });
                update.added_storage.insert(slot, balance.into());
                let mut parent_storage = SeededHashMap::default();
                parent_storage.insert(slot, U256::zero());
                let diff = AccountDiff {
                    update,
                    parent_info: AccountInfo::default(),
                    parent_storage,
                };
                (address, diff)
            })
            .collect();
        Arc::new(diff)
    }

    fn updates(diff: &StateDiff) -> Arc<Vec<AccountUpdate>> {
        Arc::new(diff.values().map(|diff| diff.update.clone()).collect())
    }

    /// The root the serial store merkleizer gives `updates` over the parent.
    fn serial_root(store: &Store, parent: &BlockHeader, updates: &[AccountUpdate]) -> H256 {
        store
            .apply_account_updates_batch(parent.hash(), updates)
            .expect("serial merkleization")
            .expect("parent state")
            .state_trie_hash
    }

    /// Waits until the worker has applied `count` diffs. The worker runs on
    /// its own thread, which a test can only observe through the counter.
    fn wait_for_applied(preseal: &PreSealer, count: usize) {
        let deadline = Instant::now() + Duration::from_secs(30);
        while preseal.diffs_applied() < count {
            assert!(
                Instant::now() < deadline,
                "the worker applied {} diffs, expected {count}",
                preseal.diffs_applied()
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    /// Waits until nothing but `shared` (a weak reference) points at the
    /// shared state, which means the worker returned and dropped its tree.
    fn wait_for_exit(shared: &Weak<Shared>) {
        let deadline = Instant::now() + Duration::from_secs(30);
        while shared.upgrade().is_some() {
            assert!(Instant::now() < deadline, "the worker did not exit");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    /// `finish` found the worker gone (its reply channel closed).
    fn is_worker_gone(result: &Result<H256, ChainError>) -> bool {
        matches!(result, Err(ChainError::Custom(msg)) if msg.contains("stopped without a root"))
    }

    /// Diffs submitted while the worker is busy replace each other: the worker
    /// applies only the newest, and the seal still gets the right root.
    #[tokio::test]
    async fn submit_keeps_only_the_newest_diff() {
        let (store, parent) = genesis_store().await;
        let first = build(&[(1, 10), (2, 20)]);
        let second = build(&[(2, 21), (3, 30)]);
        let newest = build(&[(3, 31), (4, 40)]);

        // The worker has not started, as if it were busy with an older diff.
        let (preseal, worker) = PreSealer::new(store.clone(), parent.state_root);
        preseal.submit(first);
        preseal.submit(second);
        preseal.submit(newest.clone());
        match &preseal.shared.lock().next {
            Some(super::Job::Rebase(pending)) => assert!(
                Arc::ptr_eq(pending, &newest),
                "a submit did not replace the pending diff"
            ),
            _ => panic!("no pending diff"),
        }

        worker.spawn().expect("spawn the worker");
        // Before `finish`, which would replace the diff if it had not started.
        wait_for_applied(&preseal, 1);
        let shared = preseal.shared.clone();
        let target = updates(&newest);
        let root = preseal.finish(target.clone()).expect("pre-sealed root");
        assert_eq!(root, serial_root(&store, &parent, &target));
        assert_eq!(
            shared.diffs_applied.load(Ordering::Relaxed),
            1,
            "the replaced diffs ran"
        );
    }

    /// Each submitted diff moves the tree, and the seal moves it once more
    /// onto updates it has not seen.
    #[tokio::test]
    async fn rebases_follow_each_submit() {
        let (store, parent) = genesis_store().await;
        let preseal = PreSealer::start(store.clone(), parent.state_root).expect("start");
        preseal.submit(build(&[(1, 10), (2, 20)]));
        wait_for_applied(&preseal, 1);
        preseal.submit(build(&[(2, 21), (3, 30)]));
        wait_for_applied(&preseal, 2);

        let target = updates(&build(&[(3, 30), (5, 50)]));
        let root = preseal.finish(target.clone()).expect("pre-sealed root");
        assert_eq!(root, serial_root(&store, &parent, &target));
    }

    /// A seal with no diff submitted before it opens the tree and applies the
    /// updates in full.
    #[tokio::test]
    async fn finish_without_submit() {
        let (store, parent) = genesis_store().await;
        let preseal = PreSealer::start(store.clone(), parent.state_root).expect("start");
        let target = updates(&build(&[(1, 10), (2, 20)]));
        let root = preseal.finish(target.clone()).expect("pre-sealed root");
        assert_eq!(root, serial_root(&store, &parent, &target));
    }

    /// Dropping the handle stops the worker, which frees the tree, even with
    /// a diff still pending.
    #[tokio::test]
    async fn drop_stops_the_worker() {
        let (store, parent) = genesis_store().await;
        let preseal = PreSealer::start(store, parent.state_root).expect("start");
        preseal.submit(build(&[(1, 10)]));
        wait_for_applied(&preseal, 1);
        preseal.submit(build(&[(2, 20)]));
        let shared = Arc::downgrade(&preseal.shared);
        drop(preseal);
        wait_for_exit(&shared);
    }

    /// A worker that never started (the thread could not be spawned) is a
    /// failed pre-seal, not a seal that waits forever.
    #[tokio::test]
    async fn finish_without_a_worker_fails() {
        let (store, parent) = genesis_store().await;
        let (preseal, worker) = PreSealer::new(store, parent.state_root);
        drop(worker);
        let result = preseal.finish(updates(&build(&[(1, 10)])));
        assert!(is_worker_gone(&result), "{result:?}");
    }

    /// A tree that cannot be read fails the rebase; the worker keeps the
    /// error, applies nothing more and returns it from `finish`, so the seal
    /// falls back instead of using a root from a broken tree.
    #[tokio::test]
    async fn failed_rebase_is_returned_by_finish() {
        let (store, _) = genesis_store().await;
        // No state trie has this root, so the first `root()` fails to read it.
        let preseal = PreSealer::start(store, H256::repeat_byte(0xab)).expect("start");
        preseal.submit(build(&[(1, 10)]));
        preseal.submit(build(&[(2, 20)]));
        let shared = preseal.shared.clone();
        let result = preseal.finish(updates(&build(&[(2, 20)])));
        // The tree's own error, not a dead worker.
        assert!(result.is_err() && !is_worker_gone(&result), "{result:?}");
        assert_eq!(shared.diffs_applied.load(Ordering::Relaxed), 0);
    }

    /// A worker that panics closes the reply channel, so `finish` fails
    /// instead of waiting forever. The panic comes from `rebase_writes`'s
    /// debug assertion against two target updates for one address, the only
    /// way to make the real worker panic without test hooks; release builds
    /// compile the assertion out, so the test only exists with debug
    /// assertions.
    #[cfg(debug_assertions)]
    #[tokio::test]
    async fn panicking_worker_fails_finish() {
        let (store, parent) = genesis_store().await;
        let preseal = PreSealer::start(store, parent.state_root).expect("start");
        let held = build(&[(1, 10)]);
        preseal.submit(held.clone());
        // `rebase_writes` checks for duplicates only once the tree holds a diff.
        wait_for_applied(&preseal, 1);
        let update = held.values().next().expect("one account").update.clone();
        let duplicated = Arc::new(vec![update.clone(), update]);
        let result = preseal.finish(duplicated);
        assert!(is_worker_gone(&result), "{result:?}");
    }
}
