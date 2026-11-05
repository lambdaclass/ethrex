//! A state trie kept open across updates and split into 16 shards by the first
//! nibble of the hashed address, so its root is recomputed after each change
//! by re-hashing only the changed paths, on 16 threads.
//!
//! Import's BAL merkleizer uses it one-shot (`apply` once, then `finish`).
//! Payload building keeps one per held build and calls `apply` and `root`
//! repeatedly (see `crate::preseal`).
//!
//! It never touches `Blockchain`'s merkle pool: that pool's 16 workers must
//! all run together, so a second concurrent user deadlocks it. Each call here
//! spawns scoped threads instead.

use std::collections::hash_map::Entry;

use ethrex_common::{
    H256, U256,
    constants::EMPTY_TRIE_HASH,
    types::{AccountState, AccountUpdate, Code},
};
use ethrex_crypto::NativeCrypto;
use ethrex_rlp::{constants::RLP_NULL, decode::RLPDecode, encode::RLPEncode};
use ethrex_storage::{AccountUpdatesList, Store, error::StoreError};
use ethrex_trie::{
    Nibbles, Node, NodeRef, OnceLock, Trie, TrieError, TrieNode,
    node::{BranchNode, ExtensionNode, LeafNode},
};
use rustc_hash::FxHashMap;

use crate::{
    DROP_SENDER, STATE_SHARD_PREFETCH_THRESHOLD, STORAGE_PREFETCH_THRESHOLD, collapse_root_node,
    collect_trie, compute_sharded_storage_root, serial_storage_root,
};

/// Shards of the state trie, one per first nibble of the hashed address.
pub const SHARDS: usize = 16;
/// An account whose storage trie is first opened with at least this many slot
/// writes gets it split in 16 by slot nibble (same threshold as import's
/// `STORAGE_SHARD_THRESHOLD`).
pub const HOT_STORAGE_SLOTS: usize = 2048;
/// Below this many dirty accounts plus slots, `root()` runs inline: spawning
/// 16 threads costs more than hashing a few paths.
const INLINE_THRESHOLD: usize = 64;
/// Each part of a split storage trie gets about a 16th of the account's
/// slots, so its prefetch gate is a 16th of the whole-trie one (as in
/// `compute_sharded_storage_root`).
const STORAGE_PART_PREFETCH_THRESHOLD: usize = STORAGE_PREFETCH_THRESHOLD / SHARDS;

/// One account write with absolute values: the last write to a field wins.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AccountWrite {
    pub hashed_address: H256,
    /// The account stops existing. Its fields and storage are kept for a
    /// later write that makes it exist again, as `AccountUpdate::merge` keeps
    /// them.
    pub removed: bool,
    /// Clear the storage before applying `storage`.
    pub removed_storage: bool,
    pub nonce: Option<u64>,
    pub balance: Option<U256>,
    pub code_hash: Option<H256>,
    /// New code to persist, keyed by its hash (`finish()` returns it in
    /// `code_updates`; `root()` ignores it).
    pub code: Option<(H256, Code)>,
    /// Keyed by hashed slot. Zero deletes the slot.
    pub storage: Vec<(H256, U256)>,
}

impl AccountWrite {
    /// The write equivalent to `update` as `Store::apply_account_updates_batch`
    /// applies it.
    pub fn from_update(update: &AccountUpdate) -> Self {
        Self {
            hashed_address: ethrex_common::utils::keccak(update.address),
            removed: update.removed,
            removed_storage: update.removed_storage,
            nonce: update.info.as_ref().map(|i| i.nonce),
            balance: update.info.as_ref().map(|i| i.balance),
            code_hash: update.info.as_ref().map(|i| i.code_hash),
            // Same rule as `apply_account_updates_from_trie_batch`: code is
            // stored only alongside an info change.
            code: update
                .info
                .as_ref()
                .and_then(|i| update.code.clone().map(|c| (i.code_hash, c))),
            storage: update
                .added_storage
                .iter()
                .map(|(k, v)| (ethrex_common::utils::keccak(k), *v))
                .collect(),
        }
    }
}

/// The state of one block over its parent, kept open so that `apply` and
/// `root` can alternate and each `root()` re-hashes only what changed since
/// the previous one.
///
/// Every account a pending write names gets its leaf written by the next
/// `root()`, even when the value did not change. Import's node set, which
/// `finish()` must reproduce, includes such paths, and the payload rebase
/// never sends a write that changes nothing, so `root()` loses nothing by it.
///
/// `root()` memoizes node hashes, and `NodeRef::commit` stops at a memoized
/// node, so a node collection after a `root()` would miss every node that
/// `root()` hashed. `hashed` records that a `root()` ran so `finish()` can
/// refuse to follow one.
///
/// After an error the tree is in an unknown state; drop it.
pub struct ShardedStateTrie {
    store: Store,
    parent_state_root: H256,
    /// Shard `i` is the parent state trie, only ever written under nibble `i`.
    state: Vec<Trie>,
    /// Per shard, the accounts this tree has touched.
    accounts: Vec<FxHashMap<H256, TouchedAccount>>,
    /// Per shard, writes applied since the last `root()`, folded into
    /// `accounts` and `state` by `root()`.
    pending: Vec<FxHashMap<H256, PendingWrite>>,
    /// A `root()` has run (see the type docs).
    hashed: bool,
    /// Writes applied so far, numbering each account's first one.
    writes_seen: usize,
    /// Every `AccountWrite::code`, in write order, for `finish()`.
    code_updates: Vec<(H256, Code)>,
}

impl ShardedStateTrie {
    /// Opens the tree at `parent_state_root`. Nothing is read until `root()`.
    pub fn open(store: Store, parent_state_root: H256) -> Result<Self, StoreError> {
        let state = (0..SHARDS)
            .map(|_| store.open_state_trie(parent_state_root))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            store,
            parent_state_root,
            state,
            accounts: (0..SHARDS).map(|_| FxHashMap::default()).collect(),
            pending: (0..SHARDS).map(|_| FxHashMap::default()).collect(),
            hashed: false,
            writes_seen: 0,
            code_updates: Vec::new(),
        })
    }

    /// Queues `writes` for the next `root()` or `finish()`, in order: the last
    /// write to a field or slot wins.
    pub fn apply(&mut self, writes: impl IntoIterator<Item = AccountWrite>) {
        for mut write in writes {
            if let Some(code) = write.code.take() {
                self.code_updates.push(code);
            }
            let seq = self.writes_seen;
            self.writes_seen += 1;
            let shard = usize::from(write.hashed_address.as_bytes()[0] >> 4);
            self.pending[shard]
                .entry(write.hashed_address)
                .or_insert_with(|| PendingWrite {
                    seq,
                    ..Default::default()
                })
                .merge(write);
        }
    }

    /// The state root with every write applied so far.
    pub fn root(&mut self) -> Result<H256, StoreError> {
        self.hashed = true;
        let inline = pending_work(&self.pending) < INLINE_THRESHOLD;
        // Storage first: the state step writes each account's storage root.
        self.update_storage(inline)?;
        let children = self.update_state(inline)?;
        root_hash(&self.store, self.parent_state_root, None, children)
    }

    /// The state root, with every trie node and code import must persist for
    /// the writes applied. Consumes the tree.
    ///
    /// It reproduces import's BAL merkleizer node for node: a storage trie
    /// with `HOT_STORAGE_SLOTS` or more slot writes goes through
    /// `compute_sharded_storage_root` (Stage B.5) and any other through
    /// `serial_storage_root` (Stage B); each state shard takes accounts in the
    /// order they were first written and re-inserts an account's parent leaf
    /// before writing it (Stage C); the root is collapsed and committed
    /// (Stage D).
    ///
    /// Fails after a `root()` (see the type docs).
    pub fn finish(self) -> Result<AccountUpdatesList, StoreError> {
        if self.hashed {
            return Err(StoreError::Custom(
                "ShardedStateTrie::finish after root() would miss the nodes root() hashed"
                    .to_string(),
            ));
        }
        let Self {
            store,
            parent_state_root,
            state,
            mut accounts,
            mut pending,
            code_updates,
            ..
        } = self;
        let inline = pending_work(&pending) < INLINE_THRESHOLD;

        // Stages B and B.5.
        let jobs = take_storage_jobs(&mut pending, &mut accounts);
        let (hot, normal): (Vec<_>, Vec<_>) = jobs.into_iter().partition(StorageJob::sharded);
        let mut collected = run_storage_jobs(normal, inline, |job, reader| {
            job.collect(&store, parent_state_root, reader)
        })?;
        // One at a time: `compute_sharded_storage_root` keeps 16 threads busy.
        let mut reader = None;
        for job in hot {
            collected.push(job.collect(&store, parent_state_root, &mut reader)?);
        }
        let mut storage_updates = Vec::with_capacity(collected.len());
        for (shard, hashed_address, account, nodes) in collected {
            accounts[shard].insert(hashed_address, account);
            storage_updates.push((hashed_address, nodes));
        }

        // Stage C. A shard without writes still contributes its child.
        let shards = per_part(
            "sharded_trie_collect",
            state.into_iter().zip(accounts).zip(pending),
            |(_, pending)| !inline && !pending.is_empty(),
            |index, ((trie, accounts), pending)| collect_shard(index, trie, accounts, pending),
        )?;
        let mut root = BranchNode::default();
        let mut state_updates = Vec::new();
        for (index, (child, nodes)) in shards.into_iter().enumerate() {
            state_updates.extend(nodes);
            root.choices[index] = child;
        }

        // Stage D.
        let state_trie_hash = match collapse_root_node(&store, parent_state_root, None, root)? {
            Some(root) => {
                let mut root = NodeRef::from(root);
                let hash = root.commit(Nibbles::default(), &mut state_updates, &NativeCrypto);
                // Freeing a large tree is slow; the drop thread does it off
                // the import path.
                let _ = DROP_SENDER.send(Box::new(root));
                hash.finalize(&NativeCrypto)
            }
            None => {
                state_updates.push((Nibbles::default(), vec![RLP_NULL]));
                *EMPTY_TRIE_HASH
            }
        };

        Ok(AccountUpdatesList {
            state_trie_hash,
            state_updates,
            storage_updates,
            code_updates,
        })
    }

    /// Whether `hashed_address`'s storage trie is split in 16, as of the last
    /// `root()`. Only for tests.
    #[doc(hidden)]
    pub fn is_sharded_storage(&self, hashed_address: H256) -> bool {
        let shard = usize::from(hashed_address.as_bytes()[0] >> 4);
        self.accounts[shard]
            .get(&hashed_address)
            .and_then(|account| account.storage.as_ref())
            .is_some_and(StorageTrie::is_sharded)
    }

    /// Applies the queued slot writes and stores each written account's new
    /// storage root in its `TouchedAccount`.
    fn update_storage(&mut self, inline: bool) -> Result<(), StoreError> {
        let jobs = take_storage_jobs(&mut self.pending, &mut self.accounts);
        let (store, parent_state_root) = (&self.store, self.parent_state_root);
        // A split trie with enough slots runs its 16 parts on threads by itself.
        let (hot, normal): (Vec<_>, Vec<_>) = jobs
            .into_iter()
            .partition(|job| !inline && job.sharded() && job.slots.len() >= INLINE_THRESHOLD);
        let mut done = run_storage_jobs(normal, inline, |job, reader| {
            job.run(store, parent_state_root, reader, false)
        })?;
        // One at a time, after the rest, as import's Stage B.5 does: each
        // already keeps 16 threads busy.
        let mut reader = None;
        for job in hot {
            done.push(job.run(store, parent_state_root, &mut reader, true)?);
        }
        for (shard, hashed_address, account) in done {
            self.accounts[shard].insert(hashed_address, account);
        }
        Ok(())
    }

    /// Writes every pending account's leaf into its shard and returns each
    /// shard's child of the root branch.
    fn update_state(&mut self, inline: bool) -> Result<[NodeRef; SHARDS], StoreError> {
        children_of(
            "sharded_trie_state",
            self.state
                .iter_mut()
                .zip(self.accounts.iter_mut())
                .zip(self.pending.iter_mut().map(std::mem::take)),
            |(_, pending)| !inline && !pending.is_empty(),
            |index, ((trie, accounts), pending)| update_shard(index, trie, accounts, pending),
        )
    }
}

/// The 16 children of a root branch, child `i` computed by `child(i, part)`
/// from the `i`th of 16 `parts` (see `per_part`).
fn children_of<P: Send>(
    thread_name: &str,
    parts: impl Iterator<Item = P>,
    threaded: impl Fn(&P) -> bool,
    child: impl Fn(usize, P) -> Result<NodeRef, StoreError> + Sync,
) -> Result<[NodeRef; SHARDS], StoreError> {
    let mut children = BranchNode::EMPTY_CHOICES;
    for (slot, child) in children
        .iter_mut()
        .zip(per_part(thread_name, parts, threaded, child)?)
    {
        *slot = child;
    }
    Ok(children)
}

/// `work(i, part)` for each of `parts`, in part order. Parts for which
/// `threaded` holds run on scoped threads; the rest run inline, which suits
/// parts with nothing to write, whose result is a memoized read.
fn per_part<P: Send, T: Send>(
    thread_name: &str,
    parts: impl Iterator<Item = P>,
    threaded: impl Fn(&P) -> bool,
    work: impl Fn(usize, P) -> Result<T, StoreError> + Sync,
) -> Result<Vec<T>, StoreError> {
    let mut results: Vec<Option<T>> = Vec::new();
    std::thread::scope(|s| -> Result<(), StoreError> {
        let work = &work;
        let mut handles = Vec::new();
        for (index, part) in parts.enumerate() {
            if !threaded(&part) {
                results.push(Some(work(index, part)?));
                continue;
            }
            let handle = std::thread::Builder::new()
                .name(format!("{thread_name}_{index}"))
                .spawn_scoped(s, move || work(index, part))
                .map_err(|e| StoreError::Custom(format!("spawn failed: {e}")))?;
            handles.push((index, handle));
            results.push(None);
        }
        for (index, handle) in handles {
            results[index] = Some(
                handle
                    .join()
                    .map_err(|_| StoreError::Custom(format!("{thread_name} worker panicked")))??,
            );
        }
        Ok(())
    })?;
    Ok(results.into_iter().flatten().collect())
}

/// Dirty accounts plus queued slots, to choose between inline and threads.
fn pending_work(pending: &[FxHashMap<H256, PendingWrite>]) -> usize {
    pending
        .iter()
        .flat_map(|shard| shard.values())
        .map(|write| 1 + write.slots.len())
        .sum()
}

/// Moves the storage work queued in `pending` into one job per account. Each
/// job takes its account out of `accounts`, so jobs can run on separate
/// threads without sharing the maps; the caller puts it back.
fn take_storage_jobs(
    pending: &mut [FxHashMap<H256, PendingWrite>],
    accounts: &mut [FxHashMap<H256, TouchedAccount>],
) -> Vec<StorageJob> {
    let mut jobs = Vec::new();
    for (shard, (pending, accounts)) in pending.iter_mut().zip(accounts).enumerate() {
        for (hashed_address, write) in pending.iter_mut() {
            if !write.has_storage_work() {
                continue;
            }
            jobs.push(StorageJob {
                shard,
                hashed_address: *hashed_address,
                account: accounts.remove(hashed_address),
                reset: write.removed_storage,
                slots: last_write_per_slot(std::mem::take(&mut write.slots)),
            });
        }
    }
    jobs
}

/// The root hash of a trie whose root branch has `children`, collapsed as
/// `collapse_root_node` does when fewer than two remain. `prefix` names the
/// account for a storage trie (`None` for the state trie).
fn root_hash(
    store: &Store,
    parent_state_root: H256,
    prefix: Option<H256>,
    children: [NodeRef; SHARDS],
) -> Result<H256, StoreError> {
    let root = BranchNode::new(children);
    Ok(
        match collapse_root_node(store, parent_state_root, prefix, root)? {
            Some(node) => node.compute_hash(&NativeCrypto).finalize(&NativeCrypto),
            None => *EMPTY_TRIE_HASH,
        },
    )
}

/// An account the tree has touched.
struct TouchedAccount {
    /// Fields as last written over the parent's (the empty account's when the
    /// parent has none). `storage_root` is the parent's until `storage` opens,
    /// then the root `root()` last computed for it. Kept while the account is
    /// removed, so a later write that makes it exist again starts from them.
    state: AccountState,
    /// The last write did not remove the account.
    exists: bool,
    /// Where `storage` opens, unless a reset opens it empty.
    parent_storage_root: H256,
    storage: Option<StorageTrie>,
}

impl TouchedAccount {
    fn new(parent: Option<AccountState>) -> Self {
        Self {
            exists: parent.is_some(),
            parent_storage_root: parent.map_or(*EMPTY_TRIE_HASH, |p| p.storage_root),
            state: parent.unwrap_or_default(),
            storage: None,
        }
    }

    fn apply(&mut self, write: &PendingWrite) {
        self.exists = !write.removed;
        if let Some(nonce) = write.nonce {
            self.state.nonce = nonce;
        }
        if let Some(balance) = write.balance {
            self.state.balance = balance;
        }
        if let Some(code_hash) = write.code_hash {
            self.state.code_hash = code_hash;
        }
    }

    /// The leaf the account has in the state trie, if any. An account left
    /// empty is removed (EIP-161), as import's Stage C does.
    fn leaf(&self) -> Option<&AccountState> {
        (self.exists && self.state != AccountState::default()).then_some(&self.state)
    }
}

/// Every write to one account since the last `root()`, merged.
#[derive(Default)]
struct PendingWrite {
    /// Number of the first of these writes among all the tree's writes.
    /// `finish()` replays accounts in this order, as Stage C did.
    seq: usize,
    /// The last write's `removed`.
    removed: bool,
    /// Some write reset the storage; `slots` holds only later writes.
    removed_storage: bool,
    nonce: Option<u64>,
    balance: Option<U256>,
    code_hash: Option<H256>,
    /// In write order, so the last write to a slot wins.
    slots: Vec<(H256, U256)>,
}

impl PendingWrite {
    fn merge(&mut self, write: AccountWrite) {
        self.removed = write.removed;
        if write.removed_storage {
            self.removed_storage = true;
            self.slots.clear();
        }
        self.nonce = write.nonce.or(self.nonce);
        self.balance = write.balance.or(self.balance);
        self.code_hash = write.code_hash.or(self.code_hash);
        if self.slots.is_empty() {
            // Most accounts get one write: keep its vector instead of copying.
            self.slots = write.storage;
        } else {
            self.slots.extend(write.storage);
        }
    }

    fn has_storage_work(&self) -> bool {
        self.removed_storage || !self.slots.is_empty()
    }
}

/// An account's storage trie, opened on its first slot write.
enum StorageTrie {
    Single(Trie),
    /// Trie `i` is the account's storage trie, only ever written under first
    /// slot nibble `i`, so the 16 can be written and hashed at once.
    Sharded(Vec<Trie>),
}

impl StorageTrie {
    fn open(
        store: &Store,
        parent_state_root: H256,
        hashed_address: H256,
        storage_root: H256,
        sharded: bool,
    ) -> Result<Self, StoreError> {
        let open = || store.open_storage_trie(hashed_address, parent_state_root, storage_root);
        Ok(if sharded {
            Self::Sharded((0..SHARDS).map(|_| open()).collect::<Result<_, _>>()?)
        } else {
            Self::Single(open()?)
        })
    }

    fn is_sharded(&self) -> bool {
        matches!(self, Self::Sharded(_))
    }

    /// Applies `slots` (sorted by key, one write per key) and returns the new
    /// storage root. A split trie runs its parts on threads when `parallel`.
    fn apply(
        &mut self,
        store: &Store,
        parent_state_root: H256,
        hashed_address: H256,
        slots: &[(H256, U256)],
        parallel: bool,
    ) -> Result<H256, StoreError> {
        match self {
            Self::Single(trie) => {
                if slots.len() >= STORAGE_PREFETCH_THRESHOLD {
                    trie.prefetch_sorted(&paths(slots))?;
                }
                write_slots(trie, slots)?;
                Ok(trie.hash_no_commit(&NativeCrypto))
            }
            Self::Sharded(tries) => {
                let mut buckets: [Vec<(H256, U256)>; SHARDS] = Default::default();
                for &(key, value) in slots {
                    buckets[usize::from(key.as_bytes()[0] >> 4)].push((key, value));
                }
                let children = children_of(
                    "sharded_trie_slots",
                    tries.iter_mut().zip(buckets),
                    |(_, bucket)| parallel && !bucket.is_empty(),
                    |nibble, (trie, bucket)| {
                        if bucket.len() >= STORAGE_PART_PREFETCH_THRESHOLD {
                            trie.prefetch_sorted(&paths(&bucket))?;
                        }
                        write_slots(trie, &bucket)?;
                        Ok(shard_child(trie, nibble)?)
                    },
                )?;
                root_hash(store, parent_state_root, Some(hashed_address), children)
            }
        }
    }
}

/// Storage work for one account in one `root()`.
struct StorageJob {
    shard: usize,
    hashed_address: H256,
    /// `None` on the account's first touch: the job reads its parent leaf.
    account: Option<TouchedAccount>,
    reset: bool,
    /// Sorted by key, one write per key.
    slots: Vec<(H256, U256)>,
}

impl StorageJob {
    /// Bin-packing weight. At least 1, so a reset without slots still counts.
    fn weight(&self) -> usize {
        self.slots.len().max(1)
    }

    /// The account's storage trie is split, or opens split in this job. The
    /// kind is chosen when the trie first opens and kept, through resets too.
    fn sharded(&self) -> bool {
        match self.account.as_ref().and_then(|a| a.storage.as_ref()) {
            Some(storage) => storage.is_sharded(),
            None => self.slots.len() >= HOT_STORAGE_SLOTS,
        }
    }

    /// Applies the job and returns the account to put back in its shard.
    fn run(
        self,
        store: &Store,
        parent_state_root: H256,
        reader: &mut Option<Trie>,
        parallel: bool,
    ) -> Result<(usize, H256, TouchedAccount), StoreError> {
        let sharded = self.sharded();
        let mut account = match self.account {
            Some(account) => account,
            None => TouchedAccount::new(read_parent(
                store,
                parent_state_root,
                reader,
                self.hashed_address,
            )?),
        };
        let mut storage = match account.storage.take() {
            Some(storage) if !self.reset => storage,
            // A reset replaces the trie: nothing in the old one survives it.
            _ => {
                let root = if self.reset {
                    *EMPTY_TRIE_HASH
                } else {
                    account.parent_storage_root
                };
                StorageTrie::open(store, parent_state_root, self.hashed_address, root, sharded)?
            }
        };
        account.state.storage_root = storage.apply(
            store,
            parent_state_root,
            self.hashed_address,
            &self.slots,
            parallel,
        )?;
        account.storage = Some(storage);
        Ok((self.shard, self.hashed_address, account))
    }

    /// The one-shot counterpart of `run`, for `finish()`: also returns the
    /// trie nodes the slot writes produce, exactly as import's Stage B
    /// (`serial_storage_root`) or Stage B.5 (`compute_sharded_storage_root`)
    /// produced them, and keeps no trie.
    fn collect(
        self,
        store: &Store,
        parent_state_root: H256,
        reader: &mut Option<Trie>,
    ) -> Result<(usize, H256, TouchedAccount, Vec<TrieNode>), StoreError> {
        let sharded = self.sharded();
        let mut account = match self.account {
            Some(account) => account,
            None => TouchedAccount::new(read_parent(
                store,
                parent_state_root,
                reader,
                self.hashed_address,
            )?),
        };
        let storage_root = if self.reset {
            *EMPTY_TRIE_HASH
        } else {
            account.parent_storage_root
        };
        let collect = if sharded {
            compute_sharded_storage_root
        } else {
            serial_storage_root
        };
        let (root, nodes) = collect(
            store,
            parent_state_root,
            self.hashed_address,
            storage_root,
            &self.slots,
        )?;
        account.state.storage_root = root;
        Ok((self.shard, self.hashed_address, account, nodes))
    }
}

/// `run(job, reader)` for each job: inline when `inline` or there is at most
/// one job, else on up to `SHARDS` scoped threads, bin-packed greedily by slot
/// count as import's Stage B does. `reader` is the worker's own read-only
/// parent state trie, opened on first use.
fn run_storage_jobs<T: Send>(
    mut jobs: Vec<StorageJob>,
    inline: bool,
    run: impl Fn(StorageJob, &mut Option<Trie>) -> Result<T, StoreError> + Sync,
) -> Result<Vec<T>, StoreError> {
    if inline || jobs.len() <= 1 {
        let mut reader = None;
        return jobs.into_iter().map(|job| run(job, &mut reader)).collect();
    }
    jobs.sort_unstable_by_key(|job| std::cmp::Reverse(job.weight()));
    let mut bins: Vec<Vec<StorageJob>> = (0..SHARDS).map(|_| Vec::new()).collect();
    let mut weights = [0usize; SHARDS];
    for job in jobs {
        let lightest = weights
            .iter()
            .enumerate()
            .min_by_key(|(_, weight)| **weight)
            .map_or(0, |(bin, _)| bin);
        weights[lightest] += job.weight();
        bins[lightest].push(job);
    }
    let run = &run;
    std::thread::scope(|s| {
        let handles = bins
            .into_iter()
            .enumerate()
            .filter(|(_, bin)| !bin.is_empty())
            .map(|(worker, bin)| {
                std::thread::Builder::new()
                    .name(format!("sharded_trie_storage_{worker}"))
                    .spawn_scoped(s, move || {
                        let mut reader = None;
                        bin.into_iter()
                            .map(|job| run(job, &mut reader))
                            .collect::<Result<Vec<_>, _>>()
                    })
                    .map_err(|e| StoreError::Custom(format!("spawn failed: {e}")))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let mut done = Vec::new();
        for handle in handles {
            done.extend(handle.join().map_err(|_| {
                StoreError::Custom("sharded trie storage worker panicked".to_string())
            })??);
        }
        Ok(done)
    })
}

/// Writes `pending` into shard `index` and returns the shard's child of the
/// root branch.
fn update_shard(
    index: usize,
    trie: &mut Trie,
    accounts: &mut FxHashMap<H256, TouchedAccount>,
    pending: FxHashMap<H256, PendingWrite>,
) -> Result<NodeRef, StoreError> {
    let mut writes: Vec<(H256, PendingWrite)> = pending.into_iter().collect();
    // Path order, so neighbouring writes reuse the nodes just loaded.
    writes.sort_unstable_by_key(|(hashed_address, _)| *hashed_address);
    // Loads the paths level by level instead of one read per node. Only the
    // first `root()` gains: once the shard root is resolved it is a no-op.
    if writes.len() >= STATE_SHARD_PREFETCH_THRESHOLD {
        let paths: Vec<Nibbles> = writes
            .iter()
            .map(|(hashed_address, _)| Nibbles::from_bytes(hashed_address.as_bytes()))
            .collect();
        trie.prefetch_sorted(&paths)?;
    }
    for (hashed_address, write) in writes {
        let path = hashed_address.as_bytes();
        let account = match accounts.entry(hashed_address) {
            Entry::Occupied(entry) => entry.into_mut(),
            // First touch: this shard never wrote the leaf, so it still holds
            // the parent's.
            Entry::Vacant(entry) => {
                entry.insert(TouchedAccount::new(decode_account(trie.get(path)?)?))
            }
        };
        account.apply(&write);
        match account.leaf() {
            Some(leaf) => trie.insert(path.to_vec(), leaf.encode_to_vec())?,
            None => {
                trie.remove(path)?;
            }
        }
    }
    Ok(shard_child(trie, index)?)
}

/// Stage C for shard `index`, for `finish()`: writes `pending` the way import
/// did and returns the shard's child of the root branch with the nodes the
/// shard produced.
fn collect_shard(
    index: usize,
    mut trie: Trie,
    mut accounts: FxHashMap<H256, TouchedAccount>,
    pending: FxHashMap<H256, PendingWrite>,
) -> Result<(NodeRef, Vec<TrieNode>), StoreError> {
    let mut writes: Vec<(H256, PendingWrite)> = pending.into_iter().collect();
    if writes.len() >= STATE_SHARD_PREFETCH_THRESHOLD {
        let mut addresses: Vec<H256> = writes.iter().map(|(address, _)| *address).collect();
        addresses.sort_unstable();
        let paths: Vec<Nibbles> = addresses
            .iter()
            .map(|address| Nibbles::from_bytes(address.as_bytes()))
            .collect();
        trie.prefetch_sorted(&paths)?;
    }
    // First-write order, the order Stage C took them in: which unchanged
    // nodes a removal re-creates depends on what was inserted before it, so
    // another order can emit a different node set.
    writes.sort_unstable_by_key(|(_, write)| write.seq);
    for (hashed_address, write) in writes {
        let path = hashed_address.as_bytes();
        // Stage C re-inserts the parent leaf before writing the account, so
        // its path is collected even when the account ends up unchanged.
        let parent = match trie.get(path)? {
            Some(rlp) => {
                let parent = AccountState::decode(&rlp)?;
                trie.insert(path.to_vec(), rlp)?;
                Some(parent)
            }
            None => None,
        };
        let mut account = accounts
            .remove(&hashed_address)
            .unwrap_or_else(|| TouchedAccount::new(parent));
        account.apply(&write);
        match account.leaf() {
            Some(leaf) => trie.insert(path.to_vec(), leaf.encode_to_vec())?,
            None => {
                trie.remove(path)?;
            }
        }
    }
    let (mut subroot, nodes) = collect_trie(index as u8, trie)?;
    Ok((std::mem::take(&mut subroot.choices[index]), nodes))
}

/// Child `index` of the root branch as `trie` holds it, with its hash
/// memoized. `trie` only ever has writes under nibble `index`, so that child
/// is current and the others are stale.
///
/// The root is read, not rewritten in branch form as `collect_trie` does: on a
/// trie whose root is a leaf or extension that would leave a branch with one
/// child, which `Trie::remove` does not collapse when that child goes.
fn shard_child(trie: &mut Trie, index: usize) -> Result<NodeRef, TrieError> {
    // Resolve a root still held as a hash, keeping the hash memoized, so later
    // calls do not read it from the database again.
    if let NodeRef::Hash(hash) = trie.root
        && hash.is_valid()
    {
        let node = trie.get_root_node(Nibbles::default())?;
        trie.root = NodeRef::Node(node, OnceLock::from(hash));
    }
    let NodeRef::Node(root, _) = &trie.root else {
        // An empty trie.
        return Ok(NodeRef::default());
    };
    // Hash an existing child in place before cloning it, so the trie keeps
    // the hash for the next call.
    let existing = |child: &NodeRef| {
        child.compute_hash(&NativeCrypto);
        child.clone()
    };
    let child = match root.as_ref() {
        Node::Branch(branch) => existing(&branch.choices[index]),
        Node::Extension(extension) if extension.prefix.at(0) == index => {
            if extension.prefix.len() == 1 {
                existing(&extension.child)
            } else {
                Node::from(ExtensionNode::new(
                    extension.prefix.offset(1),
                    extension.child.clone(),
                ))
                .into()
            }
        }
        Node::Leaf(leaf) if leaf.partial.at(0) == index => {
            Node::from(LeafNode::new(leaf.partial.offset(1), leaf.value.clone())).into()
        }
        Node::Extension(_) | Node::Leaf(_) => NodeRef::default(),
    };
    child.compute_hash(&NativeCrypto);
    Ok(child)
}

/// Reads `hashed_address`'s leaf in the parent state, opening `reader` on
/// first use.
fn read_parent(
    store: &Store,
    parent_state_root: H256,
    reader: &mut Option<Trie>,
    hashed_address: H256,
) -> Result<Option<AccountState>, StoreError> {
    let trie = match reader.take() {
        Some(trie) => trie,
        None => store.open_state_trie(parent_state_root)?,
    };
    let leaf = trie.get(hashed_address.as_bytes());
    *reader = Some(trie);
    decode_account(leaf?)
}

fn decode_account(rlp: Option<Vec<u8>>) -> Result<Option<AccountState>, StoreError> {
    Ok(rlp.map(|rlp| AccountState::decode(&rlp)).transpose()?)
}

/// Sorts `slots` by key and keeps the last write to each key.
fn last_write_per_slot(mut slots: Vec<(H256, U256)>) -> Vec<(H256, U256)> {
    // Stable, so writes to one key keep their order.
    slots.sort_by_key(|(key, _)| *key);
    // `dedup_by` keeps the first write of a run and drops each later one;
    // carry the later value into the kept one first.
    slots.dedup_by(|later, kept| {
        let same = later.0 == kept.0;
        if same {
            kept.1 = later.1;
        }
        same
    });
    slots
}

fn paths(slots: &[(H256, U256)]) -> Vec<Nibbles> {
    slots
        .iter()
        .map(|(key, _)| Nibbles::from_bytes(key.as_bytes()))
        .collect()
}

fn write_slots(trie: &mut Trie, slots: &[(H256, U256)]) -> Result<(), StoreError> {
    for (key, value) in slots {
        if value.is_zero() {
            trie.remove(key.as_bytes())?;
        } else {
            trie.insert(key.as_bytes().to_vec(), value.encode_to_vec())?;
        }
    }
    Ok(())
}
