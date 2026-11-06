//! `rebase_writes` must turn a tree that reflects one build's diff into one
//! that reflects another's: after the writes, its root equals a fresh tree's
//! with the target applied, whatever the tree held before.
//!
//! Builds are modelled as the post state of each account they touch, and their
//! diffs are derived from it the way the VM's `state_diff` does, with parent
//! values read from the store. So every update is one the VM could emit:
//! a slot is reported only when it differs from its parent value, info only
//! when it differs from the parent's, and `removed` only for an account the
//! parent has that the build leaves empty.
//!
//! `presealed_get_payload_matches_one_shot` checks the whole pre-seal: a
//! payload sealed from the tree the build loop kept is the one-shot block.

use std::{
    collections::{BTreeMap, BTreeSet, btree_map::Entry},
    time::Duration,
};

use bytes::Bytes;
use ethrex_blockchain::{
    preseal::{Rebase, rebase_writes},
    sharded_trie::{AccountWrite, ShardedStateTrie},
};
use ethrex_common::{
    Address, H256, U256,
    constants::EMPTY_KECCAK_HASH,
    seeded_hash::SeededHashMap,
    types::{
        AccountDiff, AccountInfo, AccountUpdate, BlockHeader, Code, StateDiff, TxKind, Withdrawal,
        compute_withdrawals_root,
    },
    utils::keccak,
};
use ethrex_crypto::NativeCrypto;
use ethrex_storage::Store;

use super::payload_build_loop_tests::{
    address_for, blockchain_for, payload_block, read_private_keys, setup_store, signed_transfer,
};
use super::sharded_trie_tests::{HOT, Rng, address, hot_parent_state, parent_state, slot};
use tokio_util::sync::CancellationToken;

/// An account as a build leaves it.
#[derive(Clone, Debug)]
struct Post {
    info: AccountInfo,
    /// Code the build deployed, whose hash is `info.code_hash`. `None` keeps
    /// the parent's code.
    code: Option<Bytes>,
    /// Slots the build wrote, by unhashed key. A slot written back to its
    /// parent value is not part of the diff, as in the VM.
    storage: BTreeMap<H256, U256>,
}

/// One build: the post state of each account it touched, by generated index.
type Build = BTreeMap<u64, Post>;

/// The parent state the builds run on.
struct Parent {
    store: Store,
    header: BlockHeader,
    /// Generated accounts in the parent; every third one is a contract with
    /// `slots` slots.
    accounts: u64,
    slots: u64,
}

impl Parent {
    async fn new(accounts: u64, slots: u64) -> Self {
        let (store, header) = parent_state(accounts, slots).await;
        Self {
            store,
            header,
            accounts,
            slots,
        }
    }

    /// `genesis(30, 10)` plus the hot contract with 3000 slots.
    async fn hot() -> Self {
        let (store, header) = hot_parent_state(3000).await;
        Self {
            store,
            header,
            accounts: 30,
            slots: 10,
        }
    }

    /// Account `i`'s info in the parent, or the default info if the parent
    /// lacks it, as the VM's `initial_accounts_state` holds it.
    fn info(&self, i: u64) -> AccountInfo {
        self.store
            .get_account_info_by_hash(self.header.hash(), address(i))
            .expect("read parent account")
            .unwrap_or_default()
    }

    fn slot_value(&self, i: u64, key: H256) -> U256 {
        self.store
            .get_storage_at_root(self.header.state_root, address(i), key)
            .expect("read parent slot")
            .unwrap_or_default()
    }

    /// The diff the VM reports for `build`, with parent values.
    fn diff(&self, build: &Build) -> StateDiff {
        build
            .iter()
            .filter_map(|(i, post)| Some((address(*i), self.account_diff(*i, post)?)))
            .collect()
    }

    /// As the VM's `account_update` plus `state_diff`'s parent values.
    fn account_diff(&self, i: u64, post: &Post) -> Option<AccountDiff> {
        let parent_info = self.info(i);
        let info = (post.info != parent_info).then(|| post.info.clone());
        let code = (post.info.code_hash != parent_info.code_hash).then(|| {
            let bytecode = post.code.clone().unwrap_or_default();
            Code::from_bytecode(bytecode, &NativeCrypto)
        });
        let mut update = AccountUpdate::new(address(i));
        let mut parent_storage = SeededHashMap::default();
        for (key, value) in &post.storage {
            let parent_value = self.slot_value(i, *key);
            if *value != parent_value {
                update.added_storage.insert(*key, *value);
                parent_storage.insert(*key, parent_value);
            }
        }
        update.removed = post.info.is_empty() && !parent_info.is_empty();
        if !update.removed && info.is_none() && update.added_storage.is_empty() {
            return None;
        }
        update.info = info;
        update.code = code;
        Some(AccountDiff {
            update,
            parent_info,
            parent_storage,
        })
    }

    /// Account `i` as the parent has it: the start of a post state.
    fn untouched(&self, i: u64) -> Post {
        Post {
            info: self.info(i),
            code: None,
            storage: BTreeMap::new(),
        }
    }

    /// A random post state for account `i`, from scratch.
    fn random_post(&self, rng: &mut Rng, i: u64) -> Post {
        let mut post = self.untouched(i);
        if post.info.is_empty() {
            // A new account: a transfer recipient, or a contract the build
            // deployed, with nonce 1 as EIP-161 gives it.
            post.info.balance = U256::from(1 + rng.below(1 << 40));
            if rng.chance(50) {
                let code = Bytes::from(rng.next().to_be_bytes().to_vec());
                post.info.code_hash = keccak(&code);
                post.info.nonce = 1;
                post.code = Some(code);
                self.write_random_slots(rng, &mut post);
            }
            return post;
        }
        if rng.chance(10) {
            return removal();
        }
        let writes_slots = is_contract(&post) && rng.chance(70);
        if !writes_slots || rng.chance(50) {
            // A recipient gains balance; a sender also bumps its nonce.
            post.info.balance = U256::from(1 + rng.below(1 << 40));
            if rng.chance(50) {
                post.info.nonce += 1 + rng.below(3);
            }
        }
        if writes_slots {
            self.write_random_slots(rng, &mut post);
        }
        post
    }

    /// Writes 1 to 12 slots: parent slots overwritten or deleted, and new ones.
    fn write_random_slots(&self, rng: &mut Rng, post: &mut Post) {
        for _ in 0..1 + rng.below(12) {
            let j = if rng.chance(60) {
                rng.below(self.slots)
            } else {
                self.slots + rng.below(self.slots + 8)
            };
            post.storage.insert(slot(j), random_value(rng));
        }
    }

    /// The post state of account `i` in a build that shares part of the
    /// previous build's transactions: kept, changed, redone or not touched.
    fn next_post(&self, rng: &mut Rng, i: u64, previous: &Post) -> Option<Post> {
        match rng.below(10) {
            0..=2 => Some(previous.clone()),
            3 | 4 => None,
            5 => Some(self.random_post(rng, i)),
            _ if previous.info.is_empty() => Some(self.random_post(rng, i)),
            _ => {
                let mut post = previous.clone();
                if !self.info(i).is_empty() && rng.chance(10) {
                    return Some(removal());
                }
                if rng.chance(50) {
                    post.info.balance = U256::from(1 + rng.below(1 << 40));
                }
                post.storage.retain(|_, value| match rng.below(4) {
                    0 => false,
                    1 => {
                        *value = random_value(rng);
                        true
                    }
                    _ => true,
                });
                if is_contract(&post) && rng.chance(50) {
                    self.write_random_slots(rng, &mut post);
                }
                Some(post)
            }
        }
    }

    /// Up to `n` random accounts, a quarter of them new.
    fn random_build(&self, rng: &mut Rng, n: usize) -> Build {
        let mut build = Build::new();
        self.add_random_accounts(rng, &mut build, n);
        build
    }

    /// Picks `n` accounts and adds those `build` does not touch yet.
    fn add_random_accounts(&self, rng: &mut Rng, build: &mut Build, n: usize) {
        for _ in 0..n {
            let i = if rng.chance(25) {
                self.accounts + rng.below(self.accounts / 2 + 1)
            } else {
                rng.below(self.accounts)
            };
            if let Entry::Vacant(entry) = build.entry(i) {
                entry.insert(self.random_post(rng, i));
            }
        }
    }

    /// A build that overlaps `previous`, plus up to `fresh` accounts it did
    /// not touch.
    fn next_build(&self, rng: &mut Rng, previous: &Build, fresh: usize) -> Build {
        let mut build = Build::new();
        for (i, post) in previous {
            if let Some(post) = self.next_post(rng, *i, post) {
                build.insert(*i, post);
            }
        }
        self.add_random_accounts(rng, &mut build, fresh);
        build
    }

    /// `n` random writes to the hot contract over slots `0..4000` (the parent
    /// has `0..3000`).
    fn hot_post(&self, rng: &mut Rng, n: usize) -> Post {
        let mut post = self.untouched(HOT);
        while post.storage.len() < n {
            post.storage
                .insert(slot(rng.below(4000)), random_value(rng));
        }
        post
    }

    /// A fresh tree with `diff` applied, and its root.
    fn tree_with(&self, diff: &StateDiff) -> (ShardedStateTrie, H256) {
        let mut tree =
            ShardedStateTrie::open(self.store.clone(), self.header.state_root).expect("open tree");
        tree.apply(diff.values().map(|d| AccountWrite::from_update(&d.update)));
        let root = tree.root().expect("root");
        (tree, root)
    }

    /// The oracle.
    fn fresh_root(&self, diff: &StateDiff) -> H256 {
        self.tree_with(diff).1
    }

    /// Rebases a hashed tree that holds `held` onto `target` and checks it
    /// against a fresh tree with `target`, then rebases it back to the parent
    /// and checks the parent root. The second step catches storage left stale
    /// under an account the target removes: the first root cannot see it.
    fn check_rebase(&self, held: &StateDiff, target: &StateDiff, context: &str) {
        let (mut tree, _) = self.tree_with(held);
        assert_eq!(
            rebase(&mut tree, held, target),
            self.fresh_root(target),
            "{context}"
        );
        assert_eq!(
            rebase(&mut tree, target, &StateDiff::default()),
            self.header.state_root,
            "{context}, then back to the parent"
        );
    }
}

/// A quarter of the writes delete the slot.
fn random_value(rng: &mut Rng) -> U256 {
    if rng.chance(25) {
        U256::zero()
    } else {
        U256::from(1 + rng.below(1 << 32))
    }
}

fn is_contract(post: &Post) -> bool {
    post.info.code_hash != *EMPTY_KECCAK_HASH
}

/// An account the build leaves empty, which the VM reports as removed
/// (EIP-161) with the empty info.
fn removal() -> Post {
    Post {
        info: AccountInfo::default(),
        code: Some(Bytes::new()),
        storage: BTreeMap::new(),
    }
}

fn updates(diff: &StateDiff) -> impl Iterator<Item = &AccountUpdate> {
    diff.values().map(|d| &d.update)
}

fn writes(held: &StateDiff, target: &StateDiff) -> Vec<AccountWrite> {
    match rebase_writes(Some(held), updates(target)) {
        Rebase::Writes(writes) => writes,
        Rebase::Reset => panic!("no diff here has removed_storage"),
    }
}

/// Moves `tree`, which reflects `held`, to `target` and returns its root.
fn rebase(tree: &mut ShardedStateTrie, held: &StateDiff, target: &StateDiff) -> H256 {
    tree.apply(writes(held, target));
    tree.root().expect("root")
}

/// The hashed addresses `writes` name.
fn written(writes: &[AccountWrite]) -> BTreeSet<H256> {
    writes.iter().map(|w| w.hashed_address).collect()
}

/// `post` with balance `balance`.
fn with_balance(mut post: Post, balance: u64) -> Post {
    post.info.balance = U256::from(balance);
    post
}

/// `post` with slot writes `(slot index, value)`; zero deletes.
fn with_slots(mut post: Post, writes: &[(u64, u64)]) -> Post {
    for (j, value) in writes {
        post.storage.insert(slot(*j), U256::from(*value));
    }
    post
}

/// A contract deployed by the build at an address the parent lacks.
fn deployed(seed: u64, writes: &[(u64, u64)]) -> Post {
    let code = Bytes::from(seed.to_be_bytes().to_vec());
    let post = Post {
        info: AccountInfo {
            code_hash: keccak(&code),
            balance: U256::zero(),
            nonce: 1,
        },
        code: Some(code),
        storage: BTreeMap::new(),
    };
    with_slots(post, writes)
}

#[tokio::test]
async fn parent_reads_see_genesis_slots() {
    // The diffs' parent values come from these reads, so check they see the
    // generated storage (slot `j` holds `j + 1`) and keys go in unhashed.
    let parent = Parent::new(30, 10).await;
    assert_eq!(parent.slot_value(3, slot(4)), U256::from(5));
    assert_eq!(parent.slot_value(3, slot(40)), U256::zero());
    assert_eq!(parent.slot_value(4, slot(4)), U256::zero());
    assert_eq!(parent.info(4).balance, U256::from(1_000_004));
    assert!(parent.info(100).is_empty());
}

#[tokio::test]
async fn rebase_matches_fresh_tree() {
    // Neither side writes to the store, so one parent serves every seed.
    let parent = Parent::new(300, 40).await;
    for seed in 1..=30 {
        let mut rng = Rng(seed);
        let build = parent.random_build(&mut rng, 120);
        let a = parent.diff(&build);
        let b = parent.diff(&parent.next_build(&mut rng, &build, 40));
        parent.check_rebase(&a, &b, &format!("seed {seed}: A to B"));
        parent.check_rebase(&b, &a, &format!("seed {seed}: B to A"));
        assert!(
            writes(&a, &a).is_empty(),
            "seed {seed}: rebasing onto the held diff writes nothing"
        );
    }
}

/// The payload loop rebases one tree over and over, so the tree keeps every
/// key an older best touched. The hot contract's storage trie opens split in
/// some seeds and whole in others, and both kinds see large reverts.
#[tokio::test]
async fn rebase_chain_matches_fresh_tree() {
    let parent = Parent::hot().await;
    for seed in 1..=8 {
        let mut rng = Rng(seed);
        let mut build = parent.random_build(&mut rng, 20);
        let first_hot = if seed % 2 == 0 { 2500 } else { 5 };
        build.insert(HOT, parent.hot_post(&mut rng, first_hot));
        let mut held = parent.diff(&build);
        let (mut tree, _) = parent.tree_with(&held);
        assert_eq!(
            tree.is_sharded_storage(keccak(address(HOT))),
            first_hot >= 2048,
            "seed {seed}: the first write picks the kind"
        );
        for step in 1..=5 {
            build = parent.next_build(&mut rng, &build, 8);
            match rng.below(4) {
                0 => {
                    build.remove(&HOT);
                }
                1 => {
                    build.insert(HOT, parent.hot_post(&mut rng, 2500));
                }
                2 => {
                    let n = 1 + rng.below(300) as usize;
                    build.insert(HOT, parent.hot_post(&mut rng, n));
                }
                // Whatever `next_build` made of the previous hot writes.
                _ => {}
            }
            let target = parent.diff(&build);
            assert_eq!(
                rebase(&mut tree, &held, &target),
                parent.fresh_root(&target),
                "seed {seed} step {step}"
            );
            assert!(
                writes(&target, &target).is_empty(),
                "seed {seed} step {step}: rebasing onto the held diff writes nothing"
            );
            held = target;
        }
        // Every key any build touched must be back at its parent value,
        // including storage under accounts that were removed along the way.
        assert_eq!(
            rebase(&mut tree, &held, &StateDiff::default()),
            parent.header.state_root,
            "seed {seed}: back to the parent"
        );
    }
}

#[tokio::test]
async fn rebase_named_cases() {
    let parent = Parent::new(30, 10).await;
    let p = |i| parent.untouched(i);
    // Accounts 0, 3, 6, ... are contracts with slots 0..10 holding j + 1.
    let cases: Vec<(&str, Build, Build)> = vec![
        (
            "accounts only in A revert",
            Build::from([
                (4, with_balance(p(4), 5)),
                (
                    3,
                    with_slots(with_balance(p(3), 6), &[(1, 50), (11, 7), (2, 0)]),
                ),
            ]),
            Build::from([(7, with_balance(p(7), 8))]),
        ),
        (
            "info only in A, storage only in B",
            Build::from([(6, with_balance(p(6), 9))]),
            Build::from([(6, with_slots(p(6), &[(2, 0), (12, 4)]))]),
        ),
        (
            "slot in both with equal values",
            Build::from([
                (9, with_slots(p(9), &[(1, 100), (2, 200), (15, 7)])),
                (12, with_slots(with_balance(p(12), 5), &[(1, 100)])),
            ]),
            Build::from([
                (9, with_slots(p(9), &[(1, 100), (2, 300), (16, 8)])),
                (12, with_slots(with_balance(p(12), 5), &[(1, 100)])),
            ]),
        ),
        (
            "removal in A only",
            Build::from([
                (10, removal()),
                (11, removal()),
                (12, removal()),
                (15, removal()),
            ]),
            Build::from([
                // Back with its parent info and a slot write: the tree must
                // not revive the empty fields the removal left.
                (12, with_slots(p(12), &[(3, 9)])),
                (11, with_balance(p(11), 4)),
                (15, with_balance(p(15), 4)),
            ]),
        ),
        (
            "removal in B only",
            Build::from([
                (13, with_balance(p(13), 3)),
                (18, with_slots(p(18), &[(1, 0), (20, 6)])),
            ]),
            Build::from([(13, removal()), (14, removal()), (18, removal())]),
        ),
        (
            "removal in both",
            Build::from([(19, removal()), (21, removal())]),
            Build::from([(19, removal()), (21, removal()), (22, removal())]),
        ),
        (
            "new account in A, absent from B",
            Build::from([
                (40, deployed(1, &[(0, 5), (1, 6)])),
                (41, with_balance(p(41), 7)),
                (42, deployed(2, &[(0, 1)])),
            ]),
            Build::from([(42, deployed(3, &[(0, 1), (2, 2)]))]),
        ),
    ];
    for (name, a, b) in &cases {
        let a = parent.diff(a);
        let b = parent.diff(b);
        parent.check_rebase(&a, &b, &format!("{name}: A to B"));
        parent.check_rebase(&b, &a, &format!("{name}: B to A"));
        assert!(writes(&a, &a).is_empty(), "{name}: A to A writes nothing");
        assert!(writes(&b, &b).is_empty(), "{name}: B to B writes nothing");
    }

    // An account whose update is the same in both builds gets no write.
    let a = parent.diff(&cases[2].1);
    let b = parent.diff(&cases[2].2);
    assert_eq!(
        written(&writes(&a, &b)),
        BTreeSet::from([keccak(address(9))]),
        "only the account whose slots differ"
    );
    let a = parent.diff(&cases[5].1);
    let b = parent.diff(&cases[5].2);
    assert_eq!(
        written(&writes(&a, &b)),
        BTreeSet::from([keccak(address(22))]),
        "only the account B alone removes"
    );
}

/// A diff that destroyed an account with storage reports zero for the parent
/// value of every slot the build did not read, so no write can undo it.
#[tokio::test]
async fn rebase_reports_reset_on_removed_storage() {
    let parent = Parent::new(30, 10).await;
    let plain = parent.diff(&Build::from([(4, with_balance(parent.untouched(4), 5))]));
    // Contract 3, destroyed and deployed again at the same address with one
    // slot: its parent value is 2, but the build never read it.
    let code = Bytes::from_static(&[0x60, 0x01]);
    let mut recreated = AccountUpdate::new(address(3));
    recreated.removed_storage = true;
    recreated.info = Some(AccountInfo {
        code_hash: keccak(&code),
        balance: U256::zero(),
        nonce: 1,
    });
    recreated.code = Some(Code::from_bytecode(code, &NativeCrypto));
    recreated.added_storage.insert(slot(1), U256::from(9));
    let mut parent_storage = SeededHashMap::default();
    parent_storage.insert(slot(1), U256::zero());
    let mut destroyed = plain.clone();
    destroyed.insert(
        address(3),
        AccountDiff {
            update: recreated,
            parent_info: parent.info(3),
            parent_storage,
        },
    );

    assert_eq!(
        rebase_writes(Some(&destroyed), updates(&plain)),
        Rebase::Reset,
        "held"
    );
    assert_eq!(
        rebase_writes(Some(&plain), updates(&destroyed)),
        Rebase::Reset,
        "target"
    );
    assert_eq!(
        rebase_writes(Some(&destroyed), updates(&destroyed)),
        Rebase::Reset,
        "both"
    );

    // A tree at the parent has nothing to undo: the target goes in as it is.
    let Rebase::Writes(mut from_parent) = rebase_writes(None, updates(&destroyed)) else {
        panic!("a tree at the parent needs no reset");
    };
    let mut expected: Vec<AccountWrite> =
        updates(&destroyed).map(AccountWrite::from_update).collect();
    from_parent.sort_by_key(|w| w.hashed_address);
    expected.sort_by_key(|w| w.hashed_address);
    assert_eq!(from_parent, expected);
}

/// A held diff without the parent value of a slot it wrote cannot revert that
/// slot, so the rebase falls back to a reset rather than guess.
#[tokio::test]
async fn rebase_resets_when_held_lacks_a_parent_value() {
    let parent = Parent::new(30, 10).await;
    let mut held = parent.diff(&Build::from([(
        3,
        with_slots(parent.untouched(3), &[(1, 50)]),
    )]));
    held.get_mut(&address(3))
        .expect("account 3 in the diff")
        .parent_storage
        .clear();

    assert_eq!(
        rebase_writes(Some(&held), std::iter::empty()),
        Rebase::Reset,
        "reverting the slot needs its parent value"
    );
    // A target that writes the slot needs no parent value.
    let target = parent.diff(&Build::from([(
        3,
        with_slots(parent.untouched(3), &[(1, 60)]),
    )]));
    assert!(matches!(
        rebase_writes(Some(&held), updates(&target)),
        Rebase::Writes(_)
    ));
}

/// A payload sealed from the pre-sealed tree has the roots of a one-shot build
/// of the same transactions, after the build loop moved the tree across
/// several best builds.
///
/// It drives `build_payload_loop` as `initiate_payload_build` does and seals
/// what it returns as `get_payload` does, so it can read how many diffs the
/// PreSealer applied: through `get_payload` the build is gone by then, and a
/// test where the loop never rebased would pass without checking anything.
#[tokio::test(flavor = "multi_thread")]
async fn presealed_get_payload_matches_one_shot() {
    // 300 transfers before the loop starts, 3 per sender, then 50 more that
    // continue the nonces of the first 50 senders while the loop runs.
    const SENDERS: usize = 100;
    const NONCES_BEFORE: u64 = 3;
    const LATE: usize = 50;
    // Spaced so the loop rebuilds, and the best build changes, many times;
    // a rebuild of a few hundred transfers takes a fraction of this window.
    const LATE_INTERVAL: Duration = Duration::from_millis(40);

    let keys = read_private_keys(SENDERS);
    let accounts: Vec<Address> = keys.iter().map(address_for).collect();
    let store = setup_store(&accounts).await;
    let blockchain = blockchain_for(&store);
    let mut transfer_recipient = None;
    for key in &keys {
        for nonce in 0..NONCES_BEFORE {
            let tx = signed_transfer(key, nonce).await;
            if let (None, TxKind::Call(to)) = (transfer_recipient, tx.to()) {
                transfer_recipient = Some(to);
            }
            blockchain.add_transaction_to_pool(tx).await.unwrap();
        }
    }
    // One template for both builds: `payload_block` picks a random coinbase.
    let mut template = payload_block(&store);
    // Withdrawals run after the transactions, so the seal's final rebase must
    // fold them into the held build's diff: one to an account a transaction
    // also pays (not a sender: senders start at U256::MAX), one to an address
    // nothing else touches, and one to the coinbase, which every build's fees
    // change.
    let withdrawals = vec![
        Withdrawal {
            index: 0,
            validator_index: 0,
            address: transfer_recipient.expect("a transfer was queued"),
            amount: 1_000_000,
        },
        Withdrawal {
            index: 1,
            validator_index: 1,
            address: Address::repeat_byte(0x77),
            amount: 2_000_000,
        },
        Withdrawal {
            index: 2,
            validator_index: 2,
            address: template.header.coinbase,
            amount: 3_000_000,
        },
    ];
    template.header.withdrawals_root = Some(compute_withdrawals_root(&withdrawals, &NativeCrypto));
    template.body.withdrawals = Some(withdrawals);

    let token = CancellationToken::new();
    let build = tokio::spawn(
        blockchain
            .clone()
            .build_payload_loop(template.clone(), token.clone()),
    );
    for key in &keys[..LATE] {
        let tx = signed_transfer(key, NONCES_BEFORE).await;
        blockchain.add_transaction_to_pool(tx).await.unwrap();
        tokio::time::sleep(LATE_INTERVAL).await;
    }
    token.cancel();
    let unsealed = build.await.unwrap().unwrap();

    let applied = unsealed
        .preseal_diffs_applied()
        .expect("the loop's best build has a PreSealer");
    // A lower bound: the newest diff may still be in flight.
    println!(
        "pre-seal applied {applied} diffs ({} rebases) for a block of {} txs",
        applied.saturating_sub(1),
        unsealed.transactions().len()
    );
    assert!(
        applied >= 3,
        "the PreSealer applied {applied} diffs; the test needs at least 2 rebases"
    );

    let sealer = blockchain.clone();
    let presealed = tokio::task::spawn_blocking(move || sealer.seal_payload(unsealed))
        .await
        .unwrap()
        .unwrap();
    let transactions = presealed.payload.body.transactions.clone();
    assert!(
        transactions.len() > SENDERS * NONCES_BEFORE as usize,
        "no late transaction made it into the block"
    );
    // The withdrawal-only recipient appears only in the seal's final updates,
    // so the final rebase must have written it.
    assert!(
        presealed
            .account_updates
            .iter()
            .any(|update| update.address == Address::repeat_byte(0x77)),
        "the withdrawal recipient is missing from the sealed state"
    );
    let one_shot = blockchain
        .build_payload_with_transactions(template, transactions)
        .unwrap();

    let (presealed, one_shot) = (&presealed.payload.header, &one_shot.payload.header);
    assert_eq!(presealed.state_root, one_shot.state_root, "state root");
    assert_eq!(
        presealed.receipts_root, one_shot.receipts_root,
        "receipts root"
    );
    assert_eq!(
        presealed.transactions_root, one_shot.transactions_root,
        "transactions root"
    );
}
