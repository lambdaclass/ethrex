# Payload Pre-seal on a Sharded State Trie Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `getPayload` pays only for the post-tx phase plus the last change to the held build, instead of a full serial state root.

**Architecture:** A new `ShardedStateTrie` keeps 16 state-trie shards and per-account storage tries open across updates and recomputes the root by re-hashing only dirty paths, on scoped threads. Import's BAL merkleizer moves onto it (one-shot). The payload build loop gives each held build a `PreSealer` worker that rebases the tree onto every new best by diff; the seal sends the final diff and gets the root back.

**Tech Stack:** Rust, ethrex workspace (`ethrex-blockchain`, `ethrex-levm`, `ethrex-common`, `ethrex-trie`, `ethrex-storage`), tokio, `std::thread::scope`.

**Spec:** `docs/internal/l1/payload_preseal/design.md` (read it first; every rule below comes from it).

**Branch:** `feat/keep-best-unsealed-payload` (draft PR #7496). All paths below are relative to the repository root.

**Ground rules for every task:**
- Commits need an unsandboxed shell (1Password signing). Never `--no-verify`. One git command per Bash call (a hook rejects compound git commands). No attribution lines, no session links.
- Never bare `git stash`.
- Do not modify, skip or weaken an existing test to make it pass; report instead.
- Lint per crate with `cargo clippy -p <crate> --all-targets --features ethrex-blockchain/c-kzg -- -D warnings` (the `c-kzg` feature avoids a pre-existing `needless_return` in `ethrex-crypto`).
- `ethrex_tests` needs network access to `sp1-circuits.s3.us-east-2.amazonaws.com` on a fresh release build, and UDP bind for its p2p tests (run unsandboxed if they fail with EPERM).
- Write comments that say why; no em-dashes in prose.

---

## File structure

| File | Responsibility |
| --- | --- |
| `crates/blockchain/sharded_trie.rs` (new) | `ShardedStateTrie`, `AccountWrite`: sharded, incremental state root; one-shot `finish()` for import |
| `crates/blockchain/preseal.rs` (new) | `rebase_writes` (pure diff to writes) and `PreSealer` (worker that owns a tree) |
| `crates/blockchain/blockchain.rs` | `mod sharded_trie; mod preseal;`, `handle_merkleization_bal_from_updates` on the new type, helpers made `pub(crate)`, `MAX_PAYLOADS = 4` |
| `crates/blockchain/payload.rs` | `UnsealedPayload` carries its diff and PreSealer; loop moves the PreSealer; seal uses it with a one-shot fallback; timeout drop and purge |
| `crates/blockchain/error.rs` | `ChainError::PayloadTimedOut` |
| `crates/networking/rpc/engine/payload.rs` | map `PayloadTimedOut` to `UnknownPayload` |
| `crates/common/types/account_update.rs` | `AccountDiff`, `StateDiff` |
| `crates/vm/levm/src/db/gen_db.rs` | shared per-account diff, `state_diff()` |
| `crates/vm/backends/mod.rs` | `Evm::state_diff()` |
| `test/tests/blockchain/sharded_trie_tests.rs` (new) | equivalence vs serial `apply_account_updates_batch` |
| `test/tests/blockchain/preseal_tests.rs` (new) | rebase equivalence, pre-sealed vs one-shot `getPayload`, timeout |
| `docs/l1/architecture/block_execution.md` | describe pre-seal |

---

### Task 1: `ShardedStateTrie` core (state shards, single storage tries, `apply`, `root`)

**Files:**
- Create: `crates/blockchain/sharded_trie.rs`
- Modify: `crates/blockchain/blockchain.rs` (add `pub mod sharded_trie;` next to `pub mod payload;` at line 50; make `branchify` (line 5061) and `collapse_root_node` (line 4422) `pub(crate)`)
- Create: `test/tests/blockchain/sharded_trie_tests.rs`; Modify: `test/tests/blockchain/mod.rs` (add `mod sharded_trie_tests;` in alphabetical order)

**Semantics (the contract the tests check).** Model the state as a map from account to `exists`, fields (nonce, balance, code hash) and a storage map, starting at the parent state. Writes apply in order:
- `removed: true`: the account no longer exists. Its storage map is kept, invisible while it does not exist.
- otherwise the account exists. A write to an account that does not exist starts from its parent fields (the empty account if the parent has none). `Some` fields overwrite; if `removed_storage`, the storage map is cleared first; then each slot write sets the slot (zero deletes it).
- An account whose resulting state is the empty account (`AccountState::default()`) is removed, as import's Stage C does (EIP-161). The VM never emits such an update without `removed`, so this does not change any payload root.

`root()` is the MPT root of the existing accounts, each with the storage root of its storage map. This is what `Store::apply_account_updates_batch` computes for one update per account, so the tests compare against it; for several batches, compare against the batches merged per address with `AccountUpdate::merge` (its rules match the ones above: `removed` takes the last value, info and slots the last write).

- [ ] **Step 1: Write the type skeleton with docs**

```rust
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

use ethrex_common::{H256, U256, constants::EMPTY_TRIE_HASH, types::{AccountState, AccountUpdate, Code}};
use ethrex_storage::{Store, error::StoreError};
use ethrex_trie::{Trie, TrieNode};
use rustc_hash::FxHashMap;

/// Shards of the state trie, one per first nibble of the hashed address.
pub const SHARDS: usize = 16;
/// An account whose storage trie is first opened with at least this many slot
/// writes gets it split in 16 by slot nibble (same threshold as import's
/// `STORAGE_SHARD_THRESHOLD`).
pub const HOT_STORAGE_SLOTS: usize = 2048;
/// Below this many dirty accounts plus slots, `root()` runs inline: spawning
/// 16 threads costs more than hashing a few paths.
const INLINE_THRESHOLD: usize = 64;

/// One account write with absolute values: the last write to a field wins.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AccountWrite {
    pub hashed_address: H256,
    /// The account stops existing. Its storage is kept for a later write that
    /// makes it exist again.
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
}
```

`TouchedAccount` holds: `parent: Option<Option<AccountState>>` (the parent leaf, read on first touch; `Some(None)` means it did not exist), `current: Option<AccountState>` (`None` means it does not exist), `storage: Option<StorageTrie>`, `storage_root: H256` (last computed). `PendingWrite` holds the merged fields of every write since the last `root()`: `removed: Option<bool>` (last value wins), `removed_storage: bool` (sticky), `nonce`, `balance`, `code_hash` (last `Some` wins), `slots: Vec<(H256, U256)>` (in order; a `removed_storage` write drops the slots queued before it). `enum StorageTrie { Single(Trie), Sharded(Vec<Trie>) }`.

- [ ] **Step 2: Write the failing equivalence test** (`test/tests/blockchain/sharded_trie_tests.rs`)

```rust
//! `ShardedStateTrie` must give the same root as the serial
//! `Store::apply_account_updates_batch` for the same net state change.

use ethrex_blockchain::sharded_trie::{AccountWrite, ShardedStateTrie};
use ethrex_common::{Address, H256, U256, types::{AccountInfo, AccountUpdate, BlockHeader}};
use ethrex_storage::{EngineType, Store};

/// Deterministic generator, so a failure is reproducible from its seed.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13; self.0 ^= self.0 >> 7; self.0 ^= self.0 << 17; self.0
    }
    fn below(&mut self, n: u64) -> u64 { self.next() % n }
}

/// Builds a store whose genesis holds `accounts` accounts, every third one
/// with `slots` storage slots, and returns it with the genesis header.
async fn parent_state(accounts: u64, slots: u64) -> (Store, BlockHeader) { /* genesis from
   fixtures/genesis/l1.json plus generated alloc entries; see seal_scaling harness setup in
   docs/internal/l1/payload_preseal/harness/seal_scaling_tmp.rs for the pattern (Store::new("store.db", EngineType::InMemory),
   add_initial_state) */ }

/// One random batch of `AccountUpdate`s over existing and new addresses:
/// info changes, new accounts, slot inserts, overwrites, deletions (zero) and
/// removals. Never produces an update whose result is an empty account that is
/// not `removed`: the VM never emits that.
fn random_updates(rng: &mut Rng, accounts: u64, slots: u64, n: usize) -> Vec<AccountUpdate> { .. }

#[tokio::test]
async fn one_batch_matches_serial() {
    for seed in 1..=20 {
        let (store, parent) = parent_state(300, 40).await;
        let mut rng = Rng(seed);
        let updates = random_updates(&mut rng, 300, 40, 120);
        let serial = store
            .apply_account_updates_batch(parent.hash(), &updates)
            .unwrap()
            .unwrap()
            .state_trie_hash;
        let mut tree = ShardedStateTrie::open(store.clone(), parent.state_root).unwrap();
        tree.apply(updates.iter().map(AccountWrite::from_update));
        assert_eq!(tree.root().unwrap(), serial, "seed {seed}");
    }
}

#[tokio::test]
async fn repeated_batches_match_serial_of_net_change() {
    // Apply 5 batches with a root() after each; after batch k the root must
    // equal the serial root of the merged updates of batches 1..=k
    // (`AccountUpdate::merge` per address gives the net change, except that a
    // removal followed by a write must keep earlier storage: merge already does).
}
```

Fill `parent_state`, `random_updates` and the second test completely; keep the generator rules above. `random_updates` never sets `removed_storage`: `AccountUpdate::merge` keeps slots written before a reset while the tree drops them, so only a dedicated one-batch test (`removed_storage_matches_serial`: an account with parent storage, `removed_storage: true` plus 3 new slots) covers it. New accounts that get slots also get a nonce and code, so they are never empty. Also add `small_tree_with_root_leaf` (parent state with 1 account, so the state root is a leaf, not a branch) and `emptying_every_account` (remove all accounts; expected root is `EMPTY_TRIE_HASH`).

- [ ] **Step 3: Run it to verify it fails**

Run: `cargo test -p ethrex-test --test ethrex_tests sharded_trie_tests`
Expected: compile error (`ShardedStateTrie` has no `open`/`apply`/`root`).

- [ ] **Step 4: Implement `open`, `apply`, `root`**

- `open(store, parent_state_root)`: 16 `store.open_state_trie(parent_state_root)?`, empty maps.
- `apply(writes)`: route each write to shard `hashed_address[0] >> 4` and merge into `pending` with the rules in Step 1.
- `root()`, in this order:
  1. **Storage.** Collect the accounts with queued slots or `removed_storage`. For each, if `parent` is unknown read it with a read-only `store.open_state_trie(parent_state_root)?` (one per worker thread, as Stage B does in `handle_merkleization_bal_from_updates`). Open the storage trie on first use with `store.open_storage_trie(hashed_address, parent_state_root, root)` where `root` is the parent storage root, or `EMPTY_TRIE_HASH` after `removed_storage` (a reset replaces the trie). Apply slots in hashed-key order (insert RLP of the value; remove on zero), prefetching with `prefetch_sorted` at 64 or more slots. Set `storage_root = trie.hash_no_commit(&NativeCrypto)`. Spread accounts over up to 16 scoped threads by greedy bin-packing on slot count (copy the Stage B loop). Inline when below `INLINE_THRESHOLD`.
  2. **State shards**, 16 scoped threads, shard `i` with `&mut state[i]`, `&mut accounts[i]`, drained `pending[i]`: for each pending account, on first touch read `parent` from `state[i].get(path)` (the leaf is still the parent's) unless step 1 already read it, and start `current` from it. If the write removes the account set `current = None`. Otherwise, when `current` is `None`, restart it from `parent` (or `AccountState::default()` when the parent has none); apply the fields; whenever `storage` is `Some` (Single or Sharded, reset or not, written this batch or not), set `current.storage_root` to the `storage_root` step 1 last computed for that trie, which is always current because step 1 runs before step 2 in every `root()`; and set `current = None` if the result equals `AccountState::default()`. Then `remove` the leaf when `current` is `None`, else `insert` its RLP.
  3. **Assemble.** For each shard, take its root node, `branchify` it (it may be a leaf or extension on a small trie), write it back as the shard's root, and take `choices[i]`; compute its hash with `NodeRef::compute_hash`. Put the 16 children in a `BranchNode`, call `collapse_root_node(&store, parent_state_root, None, root)`; `None` means `EMPTY_TRIE_HASH`, otherwise hash the node.

- [ ] **Step 5: Run the tests**

Run: `cargo test -p ethrex-test --test ethrex_tests sharded_trie_tests`
Expected: all pass. A mismatch is a bug in the tree, not the test.

- [ ] **Step 6: Lint and commit**

```bash
cargo fmt
cargo clippy -p ethrex-blockchain -p ethrex-test --all-targets --features ethrex-blockchain/c-kzg -- -D warnings
git add crates/blockchain/sharded_trie.rs crates/blockchain/blockchain.rs test/tests/blockchain/sharded_trie_tests.rs test/tests/blockchain/mod.rs
git commit -m "feat(l1): add a sharded state trie that recomputes its root incrementally"
```

Commit message body: why (payload seal is serial; this type re-hashes only dirty paths on 16 threads and avoids the exclusive merkle pool).

---

### Task 2: Hot accounts (storage split in 16)

**Files:** Modify `crates/blockchain/sharded_trie.rs`, `test/tests/blockchain/sharded_trie_tests.rs`.

- [ ] **Step 1: Failing tests**: `hot_account_matches_serial` (one account with 3000 slots in the parent; batch writes 2500 slots: overwrites, inserts, 300 deletions), `hot_account_cleared` (delete every slot; storage root `EMPTY_TRIE_HASH`), and `hot_account_across_batches` (3 batches of 1000 writes each to the same account, root after each). Same serial comparison as Task 1.
- [ ] **Step 2: Run, expect FAIL** only if Task 1 opened every account as `Single` and is slow; correctness may already pass. Add an assertion through a `#[doc(hidden)] pub fn is_sharded_storage(&self, hashed_address: H256) -> bool` that the account is sharded, so the test fails until Step 3.
- [ ] **Step 3: Implement**: when a storage trie is first opened with `HOT_STORAGE_SLOTS` or more queued slots, open 16 tries at the same root (`StorageTrie::Sharded`) and bucket slots by `hashed_key[0] >> 4`. Hash: 16 scoped threads, each applies its bucket, `branchify`s its root and returns `choices[nibble]`; assemble a `BranchNode` and collapse with `collapse_root_node(store, parent_state_root, Some(hashed_address), root)` (`None` means `EMPTY_TRIE_HASH`). Hot accounts run after the normal bins, one at a time (each already uses 16 threads), as Stage B.5 does. An account keeps its kind for the life of the tree.
- [ ] **Step 4: Run** `cargo test -p ethrex-test --test ethrex_tests sharded_trie_tests` (all pass).
- [ ] **Step 5: Lint and commit**: `feat(l1): shard hot storage tries in the sharded state trie`.

---

### Task 3: `finish()` and import's BAL merkleizer on the new type

**Files:** Modify `crates/blockchain/sharded_trie.rs`, `crates/blockchain/blockchain.rs:1596-1950` (`handle_merkleization_bal_from_updates`); Test: temporary differential test, then `ethrex_tests` and EF tests.

`finish(self) -> Result<AccountUpdatesList, StoreError>` must return the same `state_trie_hash`, the same `state_updates` and `storage_updates` node sets (compared as sorted sets) and the same `code_updates` as today's function for one batch. Today's node sets come from: Stage B `collect_changes_since_last_hash` per storage trie; Stage B.5 `compute_sharded_storage_root` (with its serial fallbacks for `occupied <= 1` and emptied tries); Stage C `collect_trie(index, shard)` per shard; Stage D `collapse_root_node` + `commit` of the root.

- [ ] **Step 1: Keep the old function as a reference, temporarily.** Copy the current body of `handle_merkleization_bal_from_updates` into `#[doc(hidden)] pub fn bal_merkleization_reference(&self, prepared, parent_header)` and add a temporary test in `test/tests/blockchain/sharded_trie_tests.rs` (`bal_path_matches_reference`) that synthesizes random `BalSynthesisItem` maps (including accounts with 2048+ slots and full storage clears) over a parent state and compares root and sorted node sets of the reference and the new path.
- [ ] **Step 2: Implement `finish()`** by consuming the tree: storage tries as above (Single: `collect_changes_since_last_hash`; Sharded: the `compute_sharded_storage_root` collection, including both serial fallbacks); shards with `collect_trie`; root with `collapse_root_node` and `commit` into `state_updates` (with the `RLP_NULL` entry for an empty trie, as Stage D). `code_updates`: `apply` pushes every `AccountWrite::code` into a `code_updates: Vec<(H256, Code)>` kept in the tree; `finish` returns it.
- [ ] **Step 3: Rewrite `handle_merkleization_bal_from_updates`**: convert each `BalSynthesisItem` into an `AccountWrite` (hash address and slot keys; `removed` false; fields as given; `code: item.code_hash.zip(item.code.clone())`). The empty-account rule from Task 1 gives Stage C's `state_trie.remove` for an account that ends up empty. Then `ShardedStateTrie::open(self.storage.clone(), parent_header.state_root)?`, `apply`, `finish`.
- [ ] **Step 4: Run the differential test** and `cargo test -p ethrex-test --test ethrex_tests` (all pass, including `storage_sharding_tests` and the BAL tests).
- [ ] **Step 5: Keep the reference for future edits.** `bal_merkleization_reference` is called from the `ethrex-test` crate, so `#[cfg(test)]` cannot hide it; keep it `#[doc(hidden)] pub` (the pattern `compute_sharded_storage_root` already uses) with a doc comment saying it is the pre-refactor BAL merkleizer kept only so `bal_path_matches_reference` can catch a node-set regression in `finish()`. Keep the differential test. Say why in the commit message so a reviewer does not flag it as dead code.
- [ ] **Step 6: EF blockchain tests** (Amsterdam fixtures exercise the BAL path): `make -C tooling/ef_tests/blockchain test-levm` (downloads vectors; needs network). All pass.
- [ ] **Step 7: Import benchmark.** If a bal-devnet mainnet-mix import-bench database is available (`tooling/import_benchmark/README.md`, `--with-bal`), run it on the base commit and on this commit and record both in the commit message. If not, say so in the commit message and in the final report; do not invent numbers.
- [ ] **Step 8: Lint and commit**: `refactor(l1): run the BAL merkleizer on the sharded state trie`.

---

### Task 4: One-shot seal on the sharded trie

**Files:** Modify `crates/blockchain/payload.rs:1460-1470` (`finalize_payload`).

- [ ] **Step 1: Failing check**: in `test/tests/blockchain/sharded_trie_tests.rs` add `payload_seal_root_matches_serial`: build a payload over a mempool of 200 transfers (helpers from `payload_build_loop_tests.rs`: `read_private_keys`, `setup_store`, `payload_block`), and assert the built block's `state_root` equals `store.apply_account_updates_batch(parent, &result.account_updates)` root. This passes today; it guards the change.
- [ ] **Step 2: Replace** `self.storage.apply_account_updates_batch(context.parent_hash(), &account_updates)?.ok_or(ChainError::ParentStateNotFound)?.state_trie_hash` with: read the parent header (`self.storage.get_block_header_by_hash(context.parent_hash())?.ok_or(ChainError::ParentStateNotFound)?`), `ShardedStateTrie::open(self.storage.clone(), parent.state_root)?`, `apply(account_updates.iter().map(AccountWrite::from_update))`, `root()?`. Add a helper `fn one_shot_state_root(&self, parent_hash, updates: &[AccountUpdate]) -> Result<H256, ChainError>` since Task 7 reuses it as the fallback.
- [ ] **Step 3: Run** `cargo test -p ethrex-test --test ethrex_tests` and `cargo test -p ethrex-blockchain` (all pass).
- [ ] **Step 4: Lint and commit**: `perf(l1): compute the payload state root on the sharded trie`.

---

### Task 5: A diff that does not consume the VM

**Files:** Modify `crates/common/types/account_update.rs`, `crates/vm/levm/src/db/gen_db.rs:580-690`, `crates/vm/backends/mod.rs:258`.

- [ ] **Step 1: Types** in `crates/common/types/account_update.rs`:

```rust
/// One account's change in a build, with the parent values it replaces.
/// A rebase uses the parent values to undo a change a newer build no longer makes.
#[derive(Debug, Clone, PartialEq)]
pub struct AccountDiff {
    /// Exactly what `get_state_transitions` emits for this account.
    pub update: AccountUpdate,
    /// The account's info in the parent state.
    pub parent_info: AccountInfo,
    /// Parent value of every slot in `update.added_storage`.
    pub parent_storage: SeededHashMap<H256, U256>,
}

/// A build's full change over its parent state.
pub type StateDiff = SeededHashMap<Address, AccountDiff>;
```

- [ ] **Step 2: Failing test** in `crates/vm/levm` (unit test next to `get_state_transitions`, or `test/tests/levm/` if that is where `GeneralizedDatabase` tests live; search with `grep -rn "get_state_transitions" test/tests crates/vm/levm/tests`): execute two transfers and one SSTORE on a `GeneralizedDatabase`, call `state_diff()`, then `get_state_transitions()`, and assert the updates are equal (sorted by address) and `parent_info`/`parent_storage` match the pre-state; call `state_diff()` twice and assert the same result (it must not clear anything).
- [ ] **Step 3: Implement**: move the per-account body of `get_state_transitions` (lines 587 to 680: from `if new_state_account.is_unmodified()` to `account_updates.push`) into `fn account_diff(&self, address, new_state_account) -> Result<Option<AccountDiff>, VMError>` that also fills the parent values from `initial_accounts_state` (for a destroyed account, the parent value of a slot is its initial value if present, else zero). `get_state_transitions` maps it to `.update` and keeps its clears; `state_diff(&self) -> Result<StateDiff, VMError>` collects it without clearing. Add `Evm::state_diff(&self)` in `crates/vm/backends/mod.rs`.
- [ ] **Step 4: Run** the new test plus `cargo test -p ethrex-levm` and `cargo test -p ethrex-test --test ethrex_tests` (all pass).
- [ ] **Step 5: Lint and commit**: `feat(levm): read a build's state diff without consuming the VM`.

---

### Task 6: Rebase

**Files:** Create `crates/blockchain/preseal.rs` (rebase only); Modify `crates/blockchain/blockchain.rs` (`mod preseal;`); Create `test/tests/blockchain/preseal_tests.rs`.

- [ ] **Step 1: Signature**

```rust
/// What moving a tree from the build it reflects to a target build takes.
pub enum Rebase {
    Writes(Vec<AccountWrite>),
    /// A diff destroyed an account that had storage; rebuild from the parent.
    Reset,
}

/// Writes that turn a tree reflecting `held` into one reflecting `target`.
/// A field or slot missing from the target is at its parent value, which
/// `held` carries.
pub fn rebase_writes<'a>(
    held: Option<&StateDiff>,
    target: impl IntoIterator<Item = &'a AccountUpdate>,
) -> Rebase
```

Rules (spec, "Rebase rules"), per account in `held` or in the target: `removed_storage` in either is `Reset`; account only in the target: `AccountWrite::from_update`; only in `held`: write `parent_info` in full (and `removed: false`) if `held` changed info or removed it, and each `held` slot back to its parent value; in both: target info (or `held.parent_info`) in full if it differs from `held`'s (or `held` removed it and the target does not), `removed: true` if the target removes it, and every slot of either side whose target value (target, else `held` parent) differs from `held`'s.

- [ ] **Step 2: Failing tests** (`test/tests/blockchain/preseal_tests.rs`): `rebase_matches_fresh_tree`: for 30 seeds, build two random `StateDiff`s A and B over the same parent (reuse the generator style of Task 1, and produce `AccountDiff`s by pairing each update with the parent values read from the store), open a tree, apply A, `root()`, apply `rebase_writes(Some(&A), B.values().map(|d| &d.update))`, `root()`, and compare with a fresh tree with B applied. Include: an account only in A (must revert), info-only in A and storage-only in B for the same account (info must go back to the parent), a slot in both with equal values, removal in A only, removal in B only. `rebase_reports_reset_on_removed_storage`.
- [ ] **Step 3: Run, expect compile failure; implement; run until all pass.**
- [ ] **Step 4: Lint and commit**: `feat(l1): rebase a state tree from one build's diff to another's`.

---

### Task 7: `PreSealer` and the build loop

**Files:** Modify `crates/blockchain/preseal.rs`, `crates/blockchain/payload.rs`; Test `test/tests/blockchain/preseal_tests.rs`.

- [ ] **Step 1: `PreSealer`** in `preseal.rs`:

```rust
/// Keeps a held build's state tree up to date on its own thread, so the
/// seal only pays for what changed since the last update.
pub(crate) struct PreSealer {
    shared: Arc<(Mutex<Slot>, Condvar)>,
}
struct Slot {
    next: Option<Job>,
    closed: bool,
}
enum Job {
    Rebase(Arc<StateDiff>),
    Finish(Vec<AccountUpdate>, std::sync::mpsc::Sender<Result<H256, ChainError>>),
}
impl PreSealer {
    /// Starts the worker thread ("payload-preseal"); the tree opens at
    /// `parent_state_root` on the first job.
    pub(crate) fn start(store: Store, parent_state_root: H256) -> Self;
    /// Makes the tree reflect `diff`. Replaces a pending diff that has not started.
    pub(crate) fn submit(&self, diff: Arc<StateDiff>);
    /// Rebases onto `updates` and returns the root. Replaces pending diffs.
    /// Blocks: call it from a blocking thread.
    pub(crate) fn finish(self, updates: Vec<AccountUpdate>) -> Result<H256, ChainError>;
}
impl Drop for PreSealer { /* set closed, notify: the worker exits and drops the tree */ }
```

Worker loop: wait on the condvar for `next` or `closed`; take the job; on `Rebase` compute `rebase_writes(held, diff.values().map(|d| &d.update))`, apply (`Reset`: reopen the tree and apply the target in full), `root()`, set `held = diff`; on `Finish` do the same with the updates and send the root; on error keep it and return it from the next `Finish`. `finish` must not run `Drop`'s close before the reply: set the job, then wait on the receiver, then let the handle drop.

- [ ] **Step 2: Failing test** `presealed_get_payload_matches_one_shot` (`preseal_tests.rs`): with the helpers of `payload_build_loop_tests.rs`, load 300 transfers, `initiate_payload_build`, feed 50 more transfers over 1 s (so the loop rebases several times), `get_payload`, then rebuild the same transactions with `build_payload_with_transactions` (one-shot) and assert equal `state_root`, `receipts_root` and `transactions_root`.
- [ ] **Step 3: Wire it**:
  - `UnsealedPayload` gains `diff: Arc<StateDiff>` and `preseal: Option<PreSealer>`. `build_unsealed` fills `diff` with `context.vm.state_diff()?` at the end (both callers: the initial build and the rebuilds run it on their own thread; explicit builds pay it too, which is small).
  - In `build_payload_loop`: after the initial build, `let preseal = PreSealer::start(self.storage.clone(), parent_state_root)`; `preseal.submit(best.diff.clone())`; `best.preseal = Some(preseal)`. When a candidate beats `best`: `candidate.preseal = best.preseal.take()`, then `submit(candidate.diff.clone())` on it. The parent state root comes from `self.storage.get_block_header_by_hash(payload.header.parent_hash)`.
  - `seal_payload` destructures `UnsealedPayload { context, preseal, .. }` and, after `apply_withdrawals` and `extract_requests`, calls `finalize_payload(&mut context, preseal)`. `finalize_payload` gains the parameter `preseal: Option<PreSealer>` (`build_payload_t8n` passes `None`): with one, `let updates = context.vm.get_state_transitions()?` and `preseal.finish(updates.clone())`, and on `Err` it logs `warn!` and uses `one_shot_state_root` (Task 4); without one, `one_shot_state_root`.
  - `UnsealedPayload` stays move-only (no `Clone`); check `beats` and `Debug` still compile.
- [ ] **Step 4: Run** `cargo test -p ethrex-test --test ethrex_tests preseal_tests unsealed_payload_tests payload_build_loop_tests` and `cargo test -p ethrex-blockchain` (all pass).
- [ ] **Step 5: Lint and commit**: `feat(l1): pre-seal the held payload while the build loop waits`. The message notes that the initial build computes its `state_diff()` on the tokio worker that runs the loop (the initial build already runs there), so `block_building_ms` now includes that copy.

---

### Task 8: Lifecycle: `MAX_PAYLOADS = 4`, drop timed-out builds

**Files:** Modify `crates/blockchain/blockchain.rs:145`, `crates/blockchain/payload.rs` (loop, `register_payload_build_task`), `crates/blockchain/error.rs`, `crates/networking/rpc/engine/payload.rs:1822`; Test `crates/blockchain/payload.rs` unit tests (next to `evicted_build_task_is_cancelled`, line ~2148).

- [ ] **Step 1: Failing tests**:
  - `timed_out_build_is_dropped` (`#[tokio::test(start_paused = true)]`): register a build with an empty mempool, so the loop parks in its `select!` right after the initial build and no rebuild runs on `spawn_blocking` (`tokio::time::advance` only moves the clock while every task is idle). Advance the clock 13 s, yield until the task finishes, then `get_payload` returns `ChainError::PayloadTimedOut`.
  - `finished_entries_are_purged_on_register`: after the above, register a new build and assert the timed-out entry is gone.
  - Update the expectation of `evicted_build_task_is_cancelled` only through the constant (it already uses `MAX_PAYLOADS`).
- [ ] **Step 2: Implement**:
  - `const MAX_PAYLOADS: usize = 4;` with a comment: entries kept for a repeated `getPayload`; each running entry can hold a build context and a state tree.
  - `ChainError::PayloadTimedOut` with a metric label `"payload_timed_out"` in the match at `error.rs:69`.
  - Loop clock: `tokio::time::Instant` instead of `std::time::Instant` for `start` (so paused-clock tests reach the deadline).
  - When the loop leaves by the deadline (not cancelled), return `Err(ChainError::PayloadTimedOut)`; `best` and its PreSealer drop. Delete the post-loop "final rebuild" block (payload.rs ~620-645) and its comment: a dropped build needs no last rebuild.
  - `register_payload_build_task`: before the `MAX_PAYLOADS` check, `payloads.retain(|(_, entry)| !matches!(entry, PayloadOrTask::Task(task) if task.task.is_finished()))`. Comment why it is safe: `get_payload` removes an entry before it awaits its task, so a finished `Task` still listed was never requested.
  - Engine: in `crates/networking/rpc/engine/payload.rs`, map `ChainError::PayloadTimedOut` like `UnknownPayload` (same message).
- [ ] **Step 3: Run** `cargo test -p ethrex-blockchain` and `cargo test -p ethrex-test --test ethrex_tests` (all pass).
- [ ] **Step 4: Lint and commit**: `feat(l1): drop payload builds that time out and keep at most 4`.

---

### Task 9: Docs, measurements, PR body

**Files:** Modify `docs/l1/architecture/block_execution.md`; scratch harnesses (not committed).

- [ ] **Step 1: Docs**: in the payload-building section of `docs/l1/architecture/block_execution.md`, describe the pre-seal (tree per held build, rebase on each new best, seal applies the post-tx phase), the sharded trie, `MAX_PAYLOADS = 4` and the timeout drop. Commit: `docs(l1): describe payload pre-sealing`.
- [ ] **Step 2: Measure** (release, low machine load; check `uptime` first and wait while the 1-min load is above 12): copy `docs/internal/l1/payload_preseal/harness/seal_scaling_tmp.rs` and `seal_diff_tmp.rs` into `test/tests/blockchain/`, register them in `mod.rs`, run each with `cargo test --release -p ethrex-test --test ethrex_tests <name> -- --ignored --nocapture --test-threads 1`, and remove them again (`git status` clean). Also run `getpayload_latency_under_sustained_mempool_inflow` 3 times in release. Record seal and `get_payload` times next to the earlier numbers in the design doc's results section.
- [ ] **Step 3: PR body**: draft an update of #7496's description (Description, Behavior changes from the spec's "Behavior changes for the PR body", new numbers, Testing). Do not push or edit the PR without the user's go-ahead.
