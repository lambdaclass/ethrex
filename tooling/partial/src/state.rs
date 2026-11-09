//! The known-key store and the canonical state around it.
//!
//! [`Store`] is partial state at one verified block, its anchor. [`State`] is the value the
//! follower owns: the published store, the working set of keys it keeps, the finalized
//! header, and the transaction book. Both are plain values, and no method performs I/O.

use std::collections::{BTreeSet, HashMap, VecDeque};
use std::sync::Arc;

use ethrex_common::constants::EMPTY_KECCAK_HASH;
use ethrex_common::types::block_access_list::{AccountChanges, BlockAccessList};
use ethrex_common::types::{AccountState, Code};
use ethrex_common::{Address, H256, U256};
use ethrex_trie::EMPTY_TRIE_HASH;

use crate::txbook::TxBook;
use crate::verify::{Fact, Proven, VerifiedBlock, VerifiedHeader, keccak};
use crate::working::WorkingSet;

/// Largest head gap closed by applying BALs. Past it, re-proving pins is cheaper, and old
/// roots fall out of what snap peers serve.
pub const MAX_BAL_GAP: u64 = 32;

/// Ancestor headers held besides the anchor, so `BLOCKHASH` reaches its full 256 blocks.
pub const HEADER_WINDOW: usize = 256;

/// Stores kept besides the published one. Wallet block trackers rewrite `latest` to the
/// last number they polled, up to about 20 s old; 8 blocks of 12 s cover 96 s.
pub const RECENT_STATES: usize = 8;

/// An account without its storage root. Storage roots are not held because BALs would
/// leave them stale; only whether storage is non-empty is tracked, which is all the EVM
/// needs (EIP-7610 create collisions).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Account {
    /// Transaction count.
    pub nonce: u64,
    /// Balance in wei.
    pub balance: U256,
    /// Code hash, `EMPTY_KECCAK_HASH` without code.
    pub code_hash: H256,
    /// Whether the storage trie is known or assumed non-empty. Conservative: a BAL that
    /// zeroes every slot leaves it `true`, which only affects create-collision checks.
    pub has_storage: bool,
}

impl Default for Account {
    fn default() -> Self {
        Self {
            nonce: 0,
            balance: U256::zero(),
            code_hash: *EMPTY_KECCAK_HASH,
            has_storage: false,
        }
    }
}

impl Account {
    /// EIP-161 emptiness: no nonce, no balance, no code.
    pub fn is_empty(&self) -> bool {
        self.nonce == 0 && self.balance.is_zero() && self.code_hash == *EMPTY_KECCAK_HASH
    }

    /// The account as the EVM reads it. The storage root is a stand-in that only preserves
    /// emptiness.
    pub fn to_account_state(self) -> AccountState {
        AccountState {
            nonce: self.nonce,
            balance: self.balance,
            storage_root: if self.has_storage {
                NON_EMPTY_STORAGE_ROOT_STAND_IN
            } else {
                EMPTY_TRIE_HASH
            },
            code_hash: self.code_hash,
        }
    }

    fn apply(&mut self, changes: &AccountChanges) -> Option<(H256, Code)> {
        if let Some(change) = changes.balance_changes.last() {
            self.balance = change.post_balance;
        }
        if let Some(change) = changes.nonce_changes.last() {
            self.nonce = change.post_nonce;
        }
        let mut new_code = None;
        if let Some(change) = changes.code_changes.last() {
            if change.new_code.is_empty() {
                self.code_hash = *EMPTY_KECCAK_HASH;
            } else {
                let hash = keccak(&change.new_code);
                self.code_hash = hash;
                new_code = Some((
                    hash,
                    Code::from_bytecode_unchecked(change.new_code.clone(), hash),
                ));
            }
        }
        new_code
    }
}

/// A storage root that is not the empty trie's, used where only emptiness matters.
const NON_EMPTY_STORAGE_ROOT_STAND_IN: H256 = H256([0xff; 32]);

impl From<&AccountState> for Account {
    fn from(account: &AccountState) -> Self {
        Self {
            nonce: account.nonce,
            balance: account.balance,
            code_hash: account.code_hash,
            has_storage: account.storage_root != EMPTY_TRIE_HASH,
        }
    }
}

/// What the store knows about one address at the anchor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Entry {
    /// Proven absent. Every slot is a known zero.
    Absent,
    /// Proven present. `slots` holds known slots; any other slot is a miss.
    Present {
        /// The account fields.
        account: Account,
        /// Known slot values, zero included.
        slots: HashMap<U256, U256>,
    },
}

/// A state key the store can hold.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Key {
    /// An account.
    Account(Address),
    /// One storage slot of an account.
    Slot(Address, U256),
}

/// A value the store does not hold. Never read as zero.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, thiserror::Error)]
pub enum Miss {
    /// The account is unknown.
    #[error("account {0:#x} is not held")]
    Account(Address),
    /// The account is known present but the slot is not held.
    #[error("slot {1} of {0:#x} is not held")]
    Storage(Address, U256),
    /// Code by hash is not held.
    #[error("code {0:#x} is not held")]
    Code(H256),
    /// The block hash is outside the header window.
    #[error("block hash {0} is not held")]
    BlockHash(u64),
}

/// Partial state at exactly one verified block, the anchor.
///
/// Every entry holds its value at the anchor; entries carry no block of their own. Values
/// enter only through [`Store::insert`] (a [`Proven`] at the anchor), [`State::advance`]
/// (a BAL of the anchor's child), and [`State::reanchor`].
#[derive(Clone, Debug)]
pub struct Store {
    /// Verified ancestors of the anchor, oldest first, each the parent of the next, the
    /// newest being the anchor's parent. At most [`HEADER_WINDOW`] entries.
    ancestors: VecDeque<VerifiedHeader>,
    anchor: VerifiedHeader,
    accounts: HashMap<Address, Entry>,
    code: HashMap<H256, Code>,
}

impl Store {
    /// An empty store anchored at `anchor`.
    pub fn new(anchor: VerifiedHeader) -> Self {
        Self {
            ancestors: VecDeque::new(),
            anchor,
            accounts: HashMap::new(),
            code: HashMap::new(),
        }
    }

    /// The block every held value is valid at.
    pub const fn anchor(&self) -> &VerifiedHeader {
        &self.anchor
    }

    /// Held headers, oldest first, ending at the anchor.
    pub fn headers(&self) -> impl DoubleEndedIterator<Item = &VerifiedHeader> {
        self.ancestors.iter().chain(std::iter::once(&self.anchor))
    }

    /// A held header by number.
    pub fn header(&self, number: u64) -> Option<&VerifiedHeader> {
        if number == self.anchor.number() {
            return Some(&self.anchor);
        }
        let oldest = self.ancestors.front()?.number();
        let offset = usize::try_from(number.checked_sub(oldest)?).ok()?;
        self.ancestors.get(offset)
    }

    /// A held header by hash.
    pub fn header_by_hash(&self, hash: H256) -> Option<&VerifiedHeader> {
        self.headers().find(|header| header.hash() == hash)
    }

    /// The raw entry for `address`, if held.
    pub fn entry(&self, address: Address) -> Option<&Entry> {
        self.accounts.get(&address)
    }

    /// `Ok(None)` is a proven-absent account.
    pub fn account(&self, address: Address) -> Result<Option<&Account>, Miss> {
        match self.accounts.get(&address) {
            None => Err(Miss::Account(address)),
            Some(Entry::Absent) => Ok(None),
            Some(Entry::Present { account, .. }) => Ok(Some(account)),
        }
    }

    /// A slot of a proven-absent account is a known zero.
    pub fn slot(&self, address: Address, slot: U256) -> Result<U256, Miss> {
        match self.accounts.get(&address) {
            None => Err(Miss::Account(address)),
            Some(Entry::Absent) => Ok(U256::zero()),
            Some(Entry::Present { slots, .. }) => slots
                .get(&slot)
                .copied()
                .ok_or(Miss::Storage(address, slot)),
        }
    }

    /// Code by hash. Empty code is always known.
    pub fn code(&self, hash: H256) -> Result<Code, Miss> {
        if hash == *EMPTY_KECCAK_HASH {
            return Ok(Code::from_bytecode_unchecked(bytes::Bytes::new(), hash));
        }
        self.code.get(&hash).cloned().ok_or(Miss::Code(hash))
    }

    /// Hash of a held header by number.
    pub fn block_hash(&self, number: u64) -> Result<H256, Miss> {
        self.header(number)
            .map(VerifiedHeader::hash)
            .ok_or(Miss::BlockHash(number))
    }

    /// Whether `key` is held.
    pub fn holds(&self, key: Key) -> bool {
        match key {
            Key::Account(address) => self.accounts.contains_key(&address),
            Key::Slot(address, slot) => self.slot(address, slot).is_ok(),
        }
    }

    /// Number of held accounts (present or proven absent).
    pub fn account_count(&self) -> usize {
        self.accounts.len()
    }

    /// Number of held storage slots.
    pub fn slot_count(&self) -> usize {
        self.accounts
            .values()
            .map(|entry| match entry {
                Entry::Absent => 0,
                Entry::Present { slots, .. } => slots.len(),
            })
            .sum()
    }

    /// Adds a proven fact. Rejects a fact proven at any block but the anchor.
    pub fn insert(&mut self, proven: &Proven) -> Result<(), StateError> {
        self.check(proven)?;
        self.insert_unchecked(proven);
        Ok(())
    }

    fn check(&self, proven: &Proven) -> Result<(), StateError> {
        let anchor = self.anchor().hash();
        match proven.at() {
            Some(at) if at.hash != anchor => Err(StateError::Stale {
                proven_at: at.hash,
                anchor,
            }),
            Some(_) | None => Ok(()),
        }
    }

    fn insert_unchecked(&mut self, proven: &Proven) {
        match proven.fact() {
            Fact::Account {
                address,
                account: None,
            } => {
                self.accounts.insert(*address, Entry::Absent);
            }
            Fact::Account {
                address,
                account: Some(account),
            } => match self.accounts.get_mut(address) {
                Some(Entry::Present { account: held, .. }) => *held = *account,
                Some(Entry::Absent) | None => {
                    self.accounts.insert(
                        *address,
                        Entry::Present {
                            account: *account,
                            slots: HashMap::new(),
                        },
                    );
                }
            },
            Fact::Slot {
                address,
                account,
                slot,
                value,
            } => match self.accounts.get_mut(address) {
                Some(Entry::Present {
                    account: held,
                    slots,
                }) => {
                    *held = *account;
                    slots.insert(*slot, *value);
                }
                Some(Entry::Absent) | None => {
                    self.accounts.insert(
                        *address,
                        Entry::Present {
                            account: *account,
                            slots: HashMap::from([(*slot, *value)]),
                        },
                    );
                }
            },
            Fact::Code { hash, code } => {
                self.code.insert(*hash, code.clone());
            }
        }
    }

    /// Prepends verified ancestors, ascending, the newest being the parent of the oldest
    /// held header.
    ///
    /// All or nothing: refuses ancestors that do not link or that would not all fit in the
    /// window, so a caller fetching for a [`Miss::BlockHash`] learns it cannot be served.
    pub fn insert_ancestors(&mut self, ancestors: Vec<VerifiedHeader>) -> Result<(), StateError> {
        let oldest = self.ancestors.front().unwrap_or(&self.anchor);
        let linked = ancestors
            .last()
            .is_none_or(|newest| newest.is_parent_of(oldest))
            && ancestors
                .windows(2)
                .all(|pair| matches!(pair, [parent, child] if parent.is_parent_of(child)));
        if !linked {
            return Err(StateError::Unlinked);
        }
        let room = HEADER_WINDOW.saturating_sub(self.ancestors.len());
        if ancestors.len() > room {
            return Err(StateError::WindowFull {
                offered: ancestors.len(),
                room,
            });
        }
        for header in ancestors.into_iter().rev() {
            self.ancestors.push_front(header);
        }
        Ok(())
    }

    /// Drops a held key. Forgetting an account forgets its slots.
    fn forget(&mut self, key: Key) {
        match key {
            Key::Account(address) => {
                self.accounts.remove(&address);
            }
            Key::Slot(address, slot) => {
                if let Some(Entry::Present { slots, .. }) = self.accounts.get_mut(&address) {
                    slots.remove(&slot);
                }
            }
        }
    }

    /// Makes the anchor's child the new anchor, keeping the window bounded.
    fn push_anchor(&mut self, child: VerifiedHeader) {
        let parent = std::mem::replace(&mut self.anchor, child);
        self.ancestors.push_back(parent);
        while self.ancestors.len() > HEADER_WINDOW {
            self.ancestors.pop_front();
        }
    }

    /// BAL semantics: post-values, last change wins, only held keys change. An account
    /// that ends EIP-161 empty becomes proven absent, with every slot a known zero.
    fn apply_bal(&mut self, bal: &BlockAccessList) {
        for changes in bal.accounts() {
            let changes_state = !(changes.balance_changes.is_empty()
                && changes.nonce_changes.is_empty()
                && changes.code_changes.is_empty()
                && changes.storage_changes.is_empty());
            if !changes_state {
                continue;
            }
            let Some(entry) = self.accounts.get_mut(&changes.address) else {
                continue;
            };
            let (mut account, mut slots) = match std::mem::replace(entry, Entry::Absent) {
                Entry::Absent => (Account::default(), HashMap::new()),
                Entry::Present { account, slots } => (account, slots),
            };
            let new_code = account.apply(changes);
            if account.is_empty() {
                // `entry` already reads `Absent`.
                continue;
            }
            for slot_change in &changes.storage_changes {
                let Some(last) = slot_change.slot_changes.last() else {
                    continue;
                };
                if !last.post_value.is_zero() {
                    account.has_storage = true;
                }
                if let Some(held) = slots.get_mut(&slot_change.slot) {
                    *held = last.post_value;
                }
            }
            *entry = Entry::Present { account, slots };
            if let Some((hash, code)) = new_code {
                // A BAL matching a verified header commits to this code.
                self.code.insert(hash, code);
            }
        }
    }
}

/// How the store reaches a newer verified head.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Plan {
    /// The target is the anchor or a held ancestor.
    Current,
    /// Fetch and apply this many BALs, ending at the target.
    Extend {
        /// Number of blocks from the anchor's child through the target.
        blocks: u64,
    },
    /// Prove the pins at the target into a fresh store; learned keys are warmed afterwards.
    Reanchor,
}

/// Outcome of folding a request's facts into [`State`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Merge {
    /// The facts for working-set keys were inserted.
    Kept,
    /// The anchor moved while the request ran; only code was kept.
    Dropped,
}

/// The canonical value the follower owns.
///
/// The store holds a key only if the working set does. Working-set keys the store lacks are
/// unheld, and the follower proves them at the anchor.
#[derive(Clone, Debug)]
pub struct State {
    store: Arc<Store>,
    /// Stores published before `store`, newest first, each anchored at a held ancestor of
    /// its anchor. At most [`RECENT_STATES`]. Nothing is folded into them.
    recent: VecDeque<Arc<Store>>,
    working: WorkingSet,
    finalized: Option<VerifiedHeader>,
    txs: TxBook,
}

impl State {
    /// State at `anchor` with nothing held yet: every key of `working` starts unheld.
    pub fn new(anchor: VerifiedHeader, working: WorkingSet) -> Self {
        Self {
            store: Arc::new(Store::new(anchor)),
            recent: VecDeque::new(),
            working,
            finalized: None,
            txs: TxBook::default(),
        }
    }

    /// The published store.
    pub const fn store(&self) -> &Arc<Store> {
        &self.store
    }

    /// Pins the current store.
    pub fn snapshot(&self) -> Arc<Store> {
        Arc::clone(&self.store)
    }

    /// Pins the store anchored at `hash`: the published one or a retained recent one.
    pub fn snapshot_at(&self, hash: H256) -> Option<Arc<Store>> {
        std::iter::once(&self.store)
            .chain(&self.recent)
            .find(|store| store.anchor().hash() == hash)
            .map(Arc::clone)
    }

    /// Keeps `previous`, the store just replaced, and drops retained stores that are no
    /// longer held ancestors of the anchor, then all but the newest [`RECENT_STATES`].
    fn retain(&mut self, previous: Arc<Store>) {
        self.recent.push_front(previous);
        let store = &self.store;
        let anchor = store.anchor().number();
        self.recent.retain(|old| {
            let at = old.anchor();
            at.number() < anchor
                && store
                    .header(at.number())
                    .is_some_and(|held| held.hash() == at.hash())
        });
        self.recent.truncate(RECENT_STATES);
    }

    /// The anchor header.
    pub fn anchor(&self) -> &VerifiedHeader {
        self.store.anchor()
    }

    /// The keys the node keeps.
    pub const fn working(&self) -> &WorkingSet {
        &self.working
    }

    /// A request read `keys`: records the use. Evicted keys' values leave the store.
    pub fn learn(&mut self, keys: impl IntoIterator<Item = Key>) {
        let evicted = self.working.touch(keys);
        self.forget(evicted);
    }

    /// Replaces the keys pinned by seeding with `keys`.
    pub fn set_seeded(&mut self, keys: BTreeSet<Key>) {
        let evicted = self.working.set_seeded(keys);
        self.forget(evicted);
    }

    /// Working-set keys the store lacks: pins first, then learned keys most recently used
    /// first.
    pub fn unheld(&self) -> impl Iterator<Item = Key> + '_ {
        self.working
            .pins()
            .chain(self.working.learned())
            .filter(|key| !self.store.holds(*key))
    }

    fn forget(&mut self, evicted: Vec<Key>) {
        if evicted.iter().any(|key| self.store.holds(*key)) {
            let store = Arc::make_mut(&mut self.store);
            for key in evicted {
                store.forget(key);
            }
        }
    }

    /// The latest finalized header.
    pub const fn finalized(&self) -> Option<&VerifiedHeader> {
        self.finalized.as_ref()
    }

    /// Records the latest finalized header.
    pub fn set_finalized(&mut self, header: VerifiedHeader) {
        self.finalized = Some(header);
    }

    /// Submitted transactions.
    pub const fn txs(&self) -> &TxBook {
        &self.txs
    }

    /// Submitted transactions, mutably.
    pub const fn txs_mut(&mut self) -> &mut TxBook {
        &mut self.txs
    }

    /// Decides how to reach `target`.
    ///
    /// BAL hashes appear at a fork and stay, so when both the anchor and the target carry
    /// one, every header between them does too; a gap straddling the fork reanchors.
    pub fn plan(&self, target: &VerifiedHeader) -> Plan {
        let anchor = self.store.anchor();
        if self
            .store
            .header(target.number())
            .is_some_and(|held| held.hash() == target.hash())
        {
            return Plan::Current;
        }
        let Some(blocks) = target
            .number()
            .checked_sub(anchor.number())
            .filter(|gap| *gap > 0)
        else {
            return Plan::Reanchor;
        };
        let has_bals = anchor.header().block_access_list_hash.is_some()
            && target.header().block_access_list_hash.is_some();
        let links = blocks > 1 || anchor.is_parent_of(target);
        if blocks <= MAX_BAL_GAP && has_bals && links {
            Plan::Extend { blocks }
        } else {
            Plan::Reanchor
        }
    }

    /// Moves the anchor to its child and applies the child's BAL to held keys. The replaced
    /// store stays readable through [`State::snapshot_at`]. Located transactions whose
    /// block is no longer on the held chain return to pending.
    ///
    /// All or nothing: the parent check runs before any mutation.
    pub fn advance(&mut self, block: VerifiedBlock) -> Result<(), StateError> {
        let anchor = self.store.anchor();
        if !anchor.is_parent_of(block.header()) {
            return Err(StateError::NotChild {
                anchor: anchor.hash(),
                child: block.header().hash(),
            });
        }
        let previous = Arc::clone(&self.store);
        let store = Arc::make_mut(&mut self.store);
        let (header, bal) = block.into_parts();
        store.apply_bal(&bal);
        store.push_anchor(header);
        self.retain(previous);
        self.txs.revalidate(&self.store);
        Ok(())
    }

    /// Replaces the store with one anchored at `target` holding `facts`, the proven pins.
    /// Every learned key becomes unheld. Code carries over because it is content-addressed.
    /// The header window carries over when `target` is the anchor's child. The replaced
    /// store is retained like [`State::advance`] retains it, and retained stores whose
    /// blocks are not held ancestors of `target` are dropped. Located transactions whose
    /// block is no longer on the held chain return to pending.
    ///
    /// All or nothing: every fact's block is checked before the store is replaced.
    pub fn reanchor(&mut self, target: VerifiedHeader, facts: &[Proven]) -> Result<(), StateError> {
        let target_hash = target.hash();
        if let Some(at) = facts
            .iter()
            .filter_map(Proven::at)
            .find(|at| at.hash != target_hash)
        {
            return Err(StateError::Stale {
                proven_at: at.hash,
                anchor: target_hash,
            });
        }
        let old = &self.store;
        let mut store = Store {
            ancestors: VecDeque::new(),
            anchor: old.anchor.clone(),
            accounts: HashMap::new(),
            code: old.code.clone(),
        };
        if old.anchor.is_parent_of(&target) {
            store.ancestors.clone_from(&old.ancestors);
        }
        store.push_anchor(target);
        if store
            .ancestors
            .back()
            .is_some_and(|parent| !parent.is_parent_of(&store.anchor))
        {
            store.ancestors.clear();
        }
        for proven in facts {
            store.insert_unchecked(proven);
        }
        let previous = std::mem::replace(&mut self.store, Arc::new(store));
        self.retain(previous);
        self.txs.revalidate(&self.store);
        Ok(())
    }

    /// Folds facts a request proved at `anchor` into the canonical store. Facts for keys
    /// outside the working set are skipped. If `anchor` is not the current one, because the
    /// anchor moved or the request read a retained store, only code is kept: it is valid at
    /// every block.
    pub fn merge(&mut self, anchor: H256, facts: &[Proven]) -> Merge {
        let current = anchor == self.store.anchor().hash()
            && facts.iter().all(|proven| self.store.check(proven).is_ok());
        let kept: Vec<&Proven> = facts
            .iter()
            .filter(|proven| match proven.key() {
                None => true,
                Some(key) => current && self.working.contains(key),
            })
            .collect();
        if !kept.is_empty() {
            let store = Arc::make_mut(&mut self.store);
            for proven in kept {
                store.insert_unchecked(proven);
            }
        }
        if current { Merge::Kept } else { Merge::Dropped }
    }
}

/// A state transition was refused. The state is unchanged.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum StateError {
    /// A fact was proven at a block other than the anchor.
    #[error("fact proven at {proven_at:#x} but the store is anchored at {anchor:#x}")]
    Stale {
        /// Block the fact was proven at.
        proven_at: H256,
        /// Current anchor.
        anchor: H256,
    },
    /// A block is not the anchor's child.
    #[error("block {child:#x} does not extend anchor {anchor:#x}")]
    NotChild {
        /// Current anchor.
        anchor: H256,
        /// The offered block.
        child: H256,
    },
    /// Ancestor headers do not link to the oldest held header.
    #[error("ancestor headers do not link to the oldest held header")]
    Unlinked,
    /// Ancestors were offered past the window. None were inserted.
    #[error("{offered} ancestors offered but the header window has room for {room}")]
    WindowFull {
        /// Number of ancestors offered.
        offered: usize,
        /// Free slots in the window.
        room: usize,
    },
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;
    use crate::testing::{World, deploy_code, eth, writer_runtime};
    use crate::verify::{self, VerifyError};
    use crate::working::WORKING_SET_CAP;
    use ethrex_common::evm::calculate_create_address;
    use ethrex_common::types::BlockHeader;
    use ethrex_common::types::block_access_list::{
        AccountChanges, BalanceChange, CodeChange, NonceChange, SlotChange, StorageChange,
    };
    use ethrex_crypto::NativeCrypto;

    const NOBODY: Address = Address::repeat_byte(0xde);
    const NEWBIE: Address = Address::repeat_byte(0xbe);

    fn slot(n: u64) -> U256 {
        U256::from(n)
    }

    fn write_call(slot: U256, value: U256) -> bytes::Bytes {
        let mut data = Vec::with_capacity(64);
        data.extend_from_slice(&slot.to_big_endian());
        data.extend_from_slice(&value.to_big_endian());
        bytes::Bytes::from(data)
    }

    struct Fixture {
        world: World,
        alice: Address,
        token: Address,
    }

    /// A world where signer 0 deployed a token-like contract with slots 1 and 2 set, and
    /// the contract accepts `(slot, value)` writes.
    async fn fixture() -> Fixture {
        let mut world = World::new().await;
        let alice = world.signer(0).address;
        let initcode = deploy_code(&[(1, 11), (2, 22)], &writer_runtime());
        world.mine(vec![world.deploy(0, 0, initcode)]).await;
        assert!(world.rejected.is_empty(), "{:?}", world.rejected);
        let token = calculate_create_address(alice, 0);
        Fixture {
            world,
            alice,
            token,
        }
    }

    fn pins(f: &Fixture) -> Vec<Key> {
        vec![
            Key::Account(f.alice),
            Key::Slot(f.token, slot(1)),
            Key::Slot(f.token, slot(2)),
            Key::Account(NEWBIE),
        ]
    }

    async fn prove_all(world: &World, at: &VerifiedHeader, keys: &[Key]) -> Vec<Proven> {
        let mut out = Vec::new();
        for key in keys {
            out.push(world.prove(at, *key).await);
        }
        out
    }

    async fn anchored(f: &Fixture) -> State {
        let head = f.world.verified(f.world.head());
        let working = WorkingSet::new(WORKING_SET_CAP, pins(f), Vec::new());
        let mut state = State::new(head.clone(), working);
        state
            .reanchor(head.clone(), &prove_all(&f.world, &head, &pins(f)).await)
            .unwrap();
        state
    }

    /// Every key the store holds reads exactly what a fresh proof at the anchor says.
    async fn assert_matches_world(store: &Store, world: &World, keys: &[Key]) {
        let mut fresh = Store::new(store.anchor().clone());
        for proven in prove_all(world, store.anchor(), keys).await {
            fresh.insert(&proven).unwrap();
        }
        for key in keys {
            match key {
                Key::Account(address) => {
                    let held = store
                        .account(*address)
                        .map(|a| a.map(|a| (a.nonce, a.balance, a.code_hash)));
                    let want = fresh
                        .account(*address)
                        .map(|a| a.map(|a| (a.nonce, a.balance, a.code_hash)));
                    assert_eq!(held, want, "{key:?}");
                }
                Key::Slot(address, slot) => {
                    assert_eq!(
                        store.slot(*address, *slot),
                        fresh.slot(*address, *slot),
                        "{key:?}"
                    );
                }
            }
        }
    }

    #[tokio::test]
    async fn bal_applies_post_values_to_held_keys() {
        let mut f = fixture().await;
        let mut state = anchored(&f).await;
        let before = state.store().account(f.alice).unwrap().unwrap().balance;
        // Block: alice funds NEWBIE, writes slot 1 to 0 and slot 3 to 33 on the token.
        let txs = vec![
            f.world.transfer(0, 1, NEWBIE, eth(1)),
            f.world
                .call(0, 2, f.token, write_call(slot(1), U256::zero())),
            f.world.call(0, 3, f.token, write_call(slot(3), slot(33))),
            f.world.transfer(1, 0, NOBODY, eth(1)),
        ];
        let hash = f.world.mine(txs).await;
        assert!(f.world.rejected.is_empty(), "{:?}", f.world.rejected);
        let header = f.world.verified(hash);
        let block = verify::bal(header.clone(), f.world.bal(hash)).unwrap();
        state.advance(block).unwrap();
        let store = state.store();

        assert_eq!(store.anchor(), &header);
        let alice = store.account(f.alice).unwrap().unwrap();
        assert_eq!(alice.nonce, 4);
        assert!(
            alice.balance < before - eth(1),
            "before {before} after {} nonce {}",
            alice.balance,
            alice.nonce
        );
        assert_eq!(store.slot(f.token, slot(1)), Ok(U256::zero()));
        assert_eq!(store.slot(f.token, slot(2)), Ok(slot(22)));
        assert_eq!(
            store.slot(f.token, slot(3)),
            Err(Miss::Storage(f.token, slot(3)))
        );
        let newbie = store.account(NEWBIE).unwrap().unwrap();
        assert_eq!(newbie.balance, eth(1));
        assert_eq!(newbie.nonce, 0);
        assert_eq!(store.account(NOBODY), Err(Miss::Account(NOBODY)));
        assert_eq!(
            store.account(f.world.signer(1).address),
            Err(Miss::Account(f.world.signer(1).address))
        );
        assert_matches_world(store, &f.world, &pins(&f)).await;
    }

    /// A header that is the anchor's child and commits to `bal`, as the light client would
    /// vouch for it.
    fn synthetic_child(anchor: &VerifiedHeader, bal: &BlockAccessList) -> VerifiedHeader {
        let parent = anchor.header();
        let header = BlockHeader {
            hash: Default::default(),
            parent_hash: anchor.hash(),
            number: anchor.number() + 1,
            timestamp: parent.timestamp + 12,
            block_access_list_hash: Some(bal.compute_hash(&NativeCrypto)),
            ..parent.clone()
        };
        let hash = header.compute_block_hash(&NativeCrypto);
        verify::header(verify::TrustedHash::attested(hash), header).unwrap()
    }

    #[tokio::test]
    async fn emptied_account_becomes_absent_and_new_code_is_kept() {
        let f = fixture().await;
        let mut state = anchored(&f).await;
        let code = bytes::Bytes::from_static(&[0x60, 0x01, 0x00]);
        let mut accounts = vec![
            AccountChanges::new(f.alice)
                .with_balance_changes(vec![BalanceChange::new(1, U256::zero())])
                .with_nonce_changes(vec![NonceChange::new(1, 0)]),
            AccountChanges::new(NEWBIE)
                .with_code_changes(vec![CodeChange::new(1, code.clone())])
                .with_storage_changes(vec![SlotChange::with_changes(
                    slot(7),
                    vec![StorageChange::new(1, slot(70))],
                )]),
            AccountChanges::new(f.token).with_storage_reads(vec![slot(2)]),
        ];
        accounts.sort_by_key(|a| a.address);
        let bal = BlockAccessList::from_accounts(accounts);
        // Alice has nonce 1 and code-less, so a BAL zeroing her balance and nonce empties her.
        let child = synthetic_child(state.anchor(), &bal);
        let block = verify::bal(child, Some(bal)).unwrap();
        state.advance(block).unwrap();
        let store = state.store();
        assert_eq!(store.entry(f.alice), Some(&Entry::Absent));
        assert_eq!(store.slot(f.alice, slot(1)), Ok(U256::zero()));
        let newbie = store.account(NEWBIE).unwrap().unwrap();
        assert_eq!(newbie.code_hash, keccak(&code));
        assert!(newbie.has_storage);
        assert_eq!(store.code(newbie.code_hash).unwrap().code_bytes(), code);
        // A write to a slot we never held stays a miss: absence of the account gave it a
        // known zero, but presence with unknown storage does not.
        assert_eq!(
            store.slot(NEWBIE, slot(7)),
            Err(Miss::Storage(NEWBIE, slot(7)))
        );
        assert_eq!(store.slot(f.token, slot(2)), Ok(slot(22)));
    }

    #[tokio::test]
    async fn plan_extends_short_gaps_and_applies_them() {
        let mut f = fixture().await;
        let mut state = anchored(&f).await;
        let anchor = state.anchor().clone();
        assert_eq!(state.plan(&anchor), Plan::Current);

        for i in 1..=3u64 {
            f.world
                .mine(vec![f.world.call(
                    0,
                    i,
                    f.token,
                    write_call(slot(2), slot(i)),
                )])
                .await;
        }
        let target = f.world.verified(f.world.head());
        assert_eq!(state.plan(&target), Plan::Extend { blocks: 3 });

        let between = f.world.headers_between(anchor.hash(), target.hash());
        for header in verify::chain(&anchor, target.clone(), between).unwrap() {
            let raw = f.world.bal(header.hash());
            state.advance(verify::bal(header, raw).unwrap()).unwrap();
        }
        assert_eq!(state.anchor(), &target);
        assert_eq!(state.plan(&anchor), Plan::Current);
        assert_eq!(state.store().block_hash(anchor.number()), Ok(anchor.hash()));
        assert_eq!(state.store().slot(f.token, slot(2)), Ok(slot(3)));
        assert_matches_world(state.store(), &f.world, &pins(&f)).await;
        assert!(state.snapshot_at(anchor.hash()).is_some());
    }

    #[tokio::test]
    async fn plan_reanchors_long_gaps_and_non_linking_heads() {
        let mut f = fixture().await;
        let state = anchored(&f).await;
        for i in 0..=MAX_BAL_GAP {
            f.world
                .mine(vec![f.world.transfer(1, i, NOBODY, eth(1))])
                .await;
        }
        assert_eq!(
            state.plan(&f.world.verified(f.world.head())),
            Plan::Reanchor
        );

        // A header without a BAL hash reanchors, and a non-child at gap 1 too.
        let anchor = state.anchor();
        let mut no_bal = anchor.header().clone();
        no_bal.hash = Default::default();
        no_bal.parent_hash = anchor.hash();
        no_bal.number = anchor.number() + 1;
        no_bal.block_access_list_hash = None;
        let hash = no_bal.compute_block_hash(&NativeCrypto);
        let no_bal = verify::header(verify::TrustedHash::attested(hash), no_bal).unwrap();
        assert_eq!(state.plan(&no_bal), Plan::Reanchor);

        let mut sibling = anchor.header().clone();
        sibling.hash = Default::default();
        sibling.parent_hash = H256::repeat_byte(1);
        sibling.number = anchor.number() + 1;
        let hash = sibling.compute_block_hash(&NativeCrypto);
        let sibling = verify::header(verify::TrustedHash::attested(hash), sibling).unwrap();
        assert_eq!(state.plan(&sibling), Plan::Reanchor);
        let mut state = state;
        let before = state.anchor().clone();
        let bal = BlockAccessList::from_accounts(vec![]);
        let mut child = sibling.header().clone();
        child.hash = Default::default();
        child.block_access_list_hash = Some(bal.compute_hash(&NativeCrypto));
        let hash = child.compute_block_hash(&NativeCrypto);
        let child = verify::header(verify::TrustedHash::attested(hash), child).unwrap();
        let block = verify::bal(child, Some(bal)).unwrap();
        assert!(matches!(
            state.advance(block),
            Err(StateError::NotChild { .. })
        ));
        assert_eq!(state.anchor(), &before);
    }

    #[tokio::test]
    async fn unknown_keys_are_misses_never_zero() {
        let f = fixture().await;
        let head = f.world.verified(f.world.head());
        let mut store = Store::new(head.clone());
        assert_eq!(store.account(f.alice), Err(Miss::Account(f.alice)));
        assert_eq!(store.slot(f.token, slot(1)), Err(Miss::Account(f.token)));

        store
            .insert(&f.world.prove(&head, Key::Account(f.token)).await)
            .unwrap();
        assert_eq!(
            store.slot(f.token, slot(1)),
            Err(Miss::Storage(f.token, slot(1)))
        );
        let code_hash = store.account(f.token).unwrap().unwrap().code_hash;
        assert_eq!(store.code(code_hash), Err(Miss::Code(code_hash)));
        assert_eq!(
            store.block_hash(head.number() - 1),
            Err(Miss::BlockHash(head.number() - 1))
        );
        assert_eq!(store.block_hash(head.number()), Ok(head.hash()));
        assert!(!store.holds(Key::Slot(f.token, slot(1))));
    }

    #[tokio::test]
    async fn facts_enter_only_at_the_anchor() {
        let mut f = fixture().await;
        let mut state = anchored(&f).await;
        let anchor = state.anchor().clone();
        let stranger = f.world.signer(2).address;
        let fact = f.world.prove(&anchor, Key::Account(stranger)).await;
        state.learn([Key::Account(stranger)]);
        assert_eq!(
            state.merge(anchor.hash(), std::slice::from_ref(&fact)),
            Merge::Kept
        );
        assert!(state.store().holds(Key::Account(stranger)));

        let hash = f
            .world
            .mine(vec![f.world.transfer(0, 1, stranger, eth(1))])
            .await;
        let block = verify::bal(f.world.verified(hash), f.world.bal(hash)).unwrap();
        state.advance(block).unwrap();
        let after = state.store().account(stranger).unwrap().unwrap().balance;
        assert_eq!(after, f.world.account_at_head(stranger).unwrap().balance);
        assert_eq!(
            state.merge(anchor.hash(), std::slice::from_ref(&fact)),
            Merge::Dropped
        );

        let mut store = Store::clone(state.store());
        assert!(matches!(store.insert(&fact), Err(StateError::Stale { .. })));
        let target = state.anchor().clone();
        assert!(matches!(
            state.reanchor(target.clone(), std::slice::from_ref(&fact)),
            Err(StateError::Stale { .. })
        ));
        state
            .reanchor(
                target.clone(),
                &prove_all(&f.world, &target, &pins(&f)).await,
            )
            .unwrap();
        assert_eq!(
            state.unheld().collect::<Vec<_>>(),
            vec![Key::Account(stranger)]
        );
        assert_eq!(
            state.store().account(stranger),
            Err(Miss::Account(stranger))
        );
        assert_matches_world(state.store(), &f.world, &pins(&f)).await;
    }

    #[tokio::test]
    async fn merge_keeps_only_working_set_keys_and_evicted_keys_leave() {
        let f = fixture().await;
        let head = f.world.verified(f.world.head());
        let stranger = f.world.signer(2).address;
        let stranger_fact = f.world.prove(&head, Key::Account(stranger)).await;
        let code_hash = f.world.account_at_head(f.token).unwrap().code_hash;
        let code = f.world.prove_code(code_hash);

        let working = WorkingSet::new(2, [Key::Account(f.alice)], Vec::new());
        let mut state = State::new(head.clone(), working);
        // Not learned: dropped. Code: always kept.
        assert_eq!(
            state.merge(head.hash(), &[stranger_fact.clone(), code]),
            Merge::Kept
        );
        assert!(!state.store().holds(Key::Account(stranger)));
        assert!(state.store().code(code_hash).is_ok());

        for address in [stranger, NEWBIE] {
            state.learn([Key::Account(address)]);
            let fact = f.world.prove(&head, Key::Account(address)).await;
            assert_eq!(
                state.merge(head.hash(), std::slice::from_ref(&fact)),
                Merge::Kept
            );
        }
        assert!(!state.store().holds(Key::Account(stranger)));
        assert!(state.store().holds(Key::Account(NEWBIE)));
        assert_eq!(
            state.unheld().collect::<Vec<_>>(),
            vec![Key::Account(f.alice)]
        );
    }

    #[tokio::test]
    async fn ancestors_extend_the_header_window() {
        let mut f = fixture().await;
        let first = f.world.head();
        for i in 0..3 {
            f.world
                .mine(vec![f.world.transfer(1, i, NOBODY, eth(1))])
                .await;
        }
        let head = f.world.verified(f.world.head());
        let mut store = Store::new(head.clone());
        let ancestors =
            verify::ancestors(&head, f.world.headers_between(first, head.hash())).unwrap();
        assert_eq!(ancestors.len(), 2);
        store.insert_ancestors(ancestors).unwrap();
        assert_eq!(store.headers().count(), 3);
        assert_eq!(
            store.block_hash(head.number() - 2),
            Ok(store.header(head.number() - 2).unwrap().hash())
        );
        assert_eq!(
            store.insert_ancestors(vec![head.clone()]),
            Err(StateError::Unlinked)
        );
        let genesis = f.world.verified(first);
        assert_eq!(
            store.block_hash(genesis.number()),
            Err(Miss::BlockHash(genesis.number()))
        );
        store.insert_ancestors(vec![genesis.clone()]).unwrap();
        assert_eq!(store.block_hash(genesis.number()), Ok(genesis.hash()));
    }

    #[tokio::test]
    async fn bal_without_canonical_order_is_rejected() {
        let f = fixture().await;
        let state = anchored(&f).await;
        let bal = BlockAccessList::from_accounts(vec![
            AccountChanges::new(Address::repeat_byte(2))
                .with_balance_changes(vec![BalanceChange::new(1, eth(1))]),
            AccountChanges::new(Address::repeat_byte(1))
                .with_balance_changes(vec![BalanceChange::new(1, eth(1))]),
        ]);
        let child = synthetic_child(state.anchor(), &bal);
        assert!(matches!(
            verify::bal(child, Some(bal)),
            Err(VerifyError::BalOrdering { .. })
        ));
    }
}
