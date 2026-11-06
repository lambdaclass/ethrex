# Payload pre-seal on a sharded state trie: design

Date: 2026-11-04. Target: draft PR #7496 (`feat/keep-best-unsealed-payload`, base `main`, stacked on #7460). FOCIL extension designed here, implemented later on the focil branches.

## Goal

`getPayload` should pay only for what changed since the held build was last merkleized, not for a full state root. Two parts:

1. Seal with a sharded (16-way parallel) merkleizer instead of the serial `Store::apply_account_updates_batch`.
2. Keep the merkle tree of the held build alive during the slot and update it as the build changes ("pre-seal"), so the seal applies only the post-tx phase and the last change.

## Background

Measured on #7496's head (release, in-memory store, harnesses in `harness/`):

- The seal is linear in the state diff. Almost all of it is the state root.
- Worst case per gas is overwriting existing slots (about 5,060 gas each): 12k slots at 61M gas seal in 64 ms; 60k slots at 304M gas in 294 ms (1M-slot contract, 1M accounts).
- The seal costs more than the unsealed build in every measured case. Before #7496 it ran on every rebuild, outside `getPayload`; #7496 moved one seal onto `getPayload`.
- The seal's state root is serial. Import's merkleizers are parallel (verified: `handle_merkleization` runs 16 shard workers; `handle_merkleization_bal_from_updates` runs 16 state shards on scoped threads).
- Release `getpayload_latency_under_sustained_mempool_inflow` fails 3 of 3 at low load (get_payload 4.0 to 4.3 ms vs a limit of half a build, 4.0 to 4.15 ms).

## Decisions

| Question | Decision |
| --- | --- |
| How does the kept tree absorb a new best build? | Rebase by diff: write only keys whose value differs; reset keys only the old build touched to their parent value |
| Share the new type with import? | Yes: import's BAL merkleizer (`handle_merkleization_bal_from_updates`) moves onto it. The streaming `handle_merkleization` stays as is |
| Delivery | Inside #7496 |
| FOCIL | Designed here (`fork()`, IL rebase); not implemented in #7496 |
| `MAX_PAYLOADS` | 10 → 4 |
| Builds that time out (12 s, no `getPayload`) | Dropped entirely, including the context and the tree |
| Reset the tree when reverted keys pile up | No |

## Architecture

```
                    crates/blockchain/merkle/sharded_trie.rs (new)
 ┌──────────────────────────────────────────────────────────────────────┐
 │ ShardedStateTrie            (opened at parent state root)            │
 │   state shards: [Trie; 16]  one per first nibble of hashed address   │
 │   storage: Map<hashed addr, StorageTrie>                             │
 │     StorageTrie::Single(Trie) | Sharded([Trie; 16]) (>= 2048 slots)  │
 │                                                                      │
 │   apply(&[AccountWrite])   field-level writes, absolute values       │
 │   root() -> H256           re-hash dirty paths only (cached hashes): │
 │                             1. dirty storage tries, in parallel       │
 │                             2. storage roots into account leaves      │
 │                             3. 16 state shards, in parallel           │
 │                             4. assemble + collapse root branch        │
 │   finish() -> AccountUpdatesList   root + trie nodes (import needs)  │
 │   fork() -> Self           FOCIL only, not in #7496                  │
 └──────────────────────────────────────────────────────────────────────┘
        ▲ one-shot: open, apply, finish             ▲ incremental: apply/root repeatedly
 import: BAL merkleizer stages B, B.5, C, D     payload: PreSealer, owned by the held
   become calls on this type; output unchanged    build; rebased whenever it changes
```

### `ShardedStateTrie`

- **State shards.** 16 `Trie`s, each opened at the parent root with `Store::open_state_trie`; shard `i` is only written under first nibble `i` (today's Stage C layout).
- **Storage.** One entry per touched account. `Single` opens one storage trie at the account's parent storage root. `Sharded` holds 16 tries split by first nibble of the hashed slot (today's `compute_sharded_storage_root`, `crates/blockchain/blockchain.rs:5162`, refactored so its 16 tries persist instead of being consumed). An account is `Sharded` when its first batch writes `STORAGE_SHARD_THRESHOLD` (2048) or more slots, and keeps that kind for the life of the tree.
- **`apply`.** Writes are absolute: the last write to a key wins. A slot written as zero is removed. An account whose resulting state is empty is removed (EIP-161), as Stage C does today.
- **`root()`.** Uses the cached node hashes (`NodeRef::Node` holds a `OnceLock<NodeHash>`, cleared along a written path), so it re-hashes only dirty paths. It needs a non-consuming variant of `collect_trie` (`blockchain.rs:5087`) that returns shard `i`'s child hash without collecting nodes. The root branch is assembled and collapsed with `collapse_root_node` (`blockchain.rs:4422`).
- **`finish()`.** Consumes the tree and returns `AccountUpdatesList` with trie nodes, as import needs; built on `collect_trie` and `collect_changes_since_last_hash`.
- **Threads.** Each `root()` and `finish()` runs on `std::thread::scope` threads, like today's BAL path. It never uses the 17-thread merkle pool, whose 16 workers must run together and deadlock when two callers share it (`Blockchain::for_test_harness_with_pool` docs). A call with fewer than 64 dirty keys runs inline instead of spawning threads (initial value; set by measuring a rebase of a few txs).

### `AccountWrite`

Per account: optional nonce, balance and code hash, slot writes (`H256 -> U256`), and the removed flag. Import converts its `BalSynthesisItem`s; payload building converts its diff entries (below).

### Import refactor

`handle_merkleization_bal_from_updates` keeps its signature and output: it opens a `ShardedStateTrie`, applies the synthesized items as one batch and calls `finish()`. The Stage B greedy bin-packing of storage work moves into `root()`/`finish()`. The streaming `handle_merkleization` and the merkle pool are not touched. The BAL path only runs for blocks with a BAL (Amsterdam+, `bal_parallel_trie_enabled`, no witness collection), so pre-Amsterdam import is unaffected.

## Diff and rebase

### Non-consuming diff (`crates/vm/levm/src/db/gen_db.rs`)

New `GeneralizedDatabase::state_diff(&self) -> Result<StateDiff, VMError>`. It produces the same updates as `get_state_transitions` without clearing the caches, and each entry also carries the parent values of what it touches:

```rust
pub struct AccountDiff {
    pub update: AccountUpdate,           // same as get_state_transitions would emit
    pub parent_info: AccountInfo,        // initial_accounts_state[address].info
    pub parent_storage: FxHashMap<H256, U256>, // parent value of each slot in update.added_storage
}
pub type StateDiff = FxHashMap<Address, AccountDiff>;
```

`initial_accounts_state` stays at the parent state during a payload build, since payload builds never flush mid-block (`get_state_transitions_tx` is import-only). A build computes its diff at the end of `build_unsealed_until`, on its own blocking thread, and the diff rides on the `UnsealedPayload` as `Arc<StateDiff>`.

### Rebase rules

The worker holds the `StateDiff` the tree reflects (`held`). `AccountUpdate` is per field: `info: None` means the info equals the parent's, and a slot missing from `added_storage` means the slot equals its parent value. So the rules run per field, for every account in `held` or in the target, with "missing in the target" meaning "the parent value, taken from `held`":

| Field | Value the tree must end with | Write when |
| --- | --- | --- |
| Account info | Target `info` if `Some`; otherwise `held.parent_info` | It differs from what the tree holds (`held` info if `Some`, else parent) |
| Each slot in `held.added_storage` or the target's | Target value if present; otherwise `held.parent_storage[slot]` | It differs from what the tree holds (`held` value if present, else parent) |
| Removed (EIP-161) | Target `removed` | Target removed and the tree has the leaf: remove it. `held` removed and the target not: re-insert with the info from the first row |
| `removed_storage` in either (destroyed with storage; rare after EIP-6780) | | Always: reset, reopen the tree at the parent and apply the target in full |

When an account is only in the target, there is no `held` value to compare, so its fields are written as given. The target never needs parent values of its own, so the final diff at seal is a plain `Vec<AccountUpdate>` from `get_state_transitions`.

Verified: nothing on the payload path flushes VM state. `get_state_transitions_tx` is only called from import's execution paths (`crates/vm/backends/levm/mod.rs`: the parallel BAL path and the sequential streaming path), and `Evm::execute_tx`, used by `apply_tx_to_payload`, does not flush.

## Payload data flow

```
build loop (async)                        PreSealer worker (one blocking task per loop)
──────────────────                        ─────────────────────────────────────────────
initial build B0 = best
  PreSealer::start(parent)
  submit(D0) ───────────────────────────▶ open tree at parent; apply all of D0; root()
rebuild B1 beats best: PreSealer
  moves to B1; submit(D1) ──────────────▶ rebase(D0 → D1); root()
B2, B3 beat best in a burst
  submit(D2), submit(D3) ───────────────▶ only the newest pending diff runs: rebase(D1 → D3)
getPayload: stop loop, seal
  post-tx phase on the VM (withdrawals,
  requests); Uf = get_state_transitions
  finish(Uf) ───────────────────────────▶ rebase(D3 → Uf); root() ──▶ state_root
  tx root, receipts root, rest of header ◀──────────────────────────
```

- **Ownership.** `UnsealedPayload` gains `diff: Arc<StateDiff>` and `preseal: Option<PreSealer>`. When a candidate beats the best, the loop moves the PreSealer to it and submits its diff; a losing candidate has none. Dropping the handle (timeout, eviction) closes the channel and the worker exits, freeing the tree.
- **Coalescing.** `submit` replaces any pending diff that has not started. `finish` replaces pending diffs too.
- **What `getPayload` pays.** The rest of an in-flight rebase, then the post-tx changes and any newer best. The seal already runs on `spawn_blocking`, so it waits on the worker's reply with a blocking receive.
- **No pre-seal.** The L2 `build_payload` path and explicit-transaction builds have no PreSealer; `finalize_payload` seals them in one shot on `ShardedStateTrie` (open, apply, `root()`), replacing the serial `apply_account_updates_batch`.

## Lifecycle and memory

| Change | Effect |
| --- | --- |
| `MAX_PAYLOADS` 10 → 4 (`crates/blockchain/blockchain.rs:145`) | At most 4 entries. A sealed payload stays available for a repeated `getPayload` until 4 newer FCUs |
| A timed-out build is dropped entirely | The loop's 12 s are counted from when it started, at FCU receipt, not from the slot start. When it reaches 12 s without `getPayload`, it drops its best build and PreSealer and returns a new `ChainError::PayloadTimedOut`. A later `getPayload` for that id answers `UnknownPayload` (-38001); the engine handler maps `PayloadTimedOut` to the same error |
| Timed-out entries are purged | `register_payload_build_task` removes entries whose loop timed out (a flag the build task sets when the loop returns `PayloadTimedOut`), under the `payloads` lock it already holds. No new lock, so no deadlock with `getPayload` (which holds the lock while it awaits the task). A build that ended on an error is kept, so its `getPayload` still reports that error |
| The final rebuild at slot end is removed | Today the loop rebuilds once more after the slot timeout if a tx arrived. The build is dropped now, so that rebuild has no use |
| No reset of the tree | Keys reset to their parent value stay loaded, so the tree holds every key touched by any best in the slot |

Behavior change: a CL that calls `getPayload` more than 12 s after the FCU used to get the best block (main and #7496 keep it); now it gets `UnknownPayload`.

Memory held by a trie after overwriting K existing keys and hashing (release, counting allocator, in-memory trie; includes the dirty set and cached hashes):

| Keys in trie (depth) | 1k written | 12k written | 60k written |
| --- | --- | --- | --- |
| 100k (~4 levels) | 2.3 MiB | 17 MiB | 55 MiB |
| 1M (~5 levels) | 3.1 MiB | 27 MiB | 103 MiB |

- 1.8 to 3.3 KB per written key; about 0.8 KB more per extra trie level; value size barely matters.
- Extrapolated to mainnet depth (about 2 levels deeper than 1M keys): 3 to 4 KB per key, about 45 MiB for the 60M-gas worst case and about 200 MiB at 300M gas. Not measured.
- Per loop, peak memory is about the same as today's seal, which loads the same nodes; the difference is that they stay loaded from the first pre-seal until `getPayload`, the timeout or eviction. Only running loops hold a tree and at most 4 entries exist, so the bound is 4 trees at once.
- The held `StateDiff` with parent values is estimated at 1 to 2 MiB at 12k slots (not measured).

## Errors

- A failed rebase (store read error, trie error) is kept by the worker; `finish` returns it. The seal logs a warning and falls back to a one-shot `ShardedStateTrie` root from `Uf`. A broken pre-seal costs time and never yields a wrong root.
- A panicking worker surfaces as a join error and takes the same fallback.
- One-shot errors propagate as today (`ChainError` from `finalize_payload`).

## FOCIL extension (design only, not in #7496)

| FOCIL step (focil branch) | Tree operation |
| --- | --- |
| Late IL appended to the held build (`apply_inclusion_list_transactions`) | Recompute the build's `state_diff()` and submit it: rebase held → held + IL. Appends overwrite values, so the delta is only the IL txs' writes |
| IL-first rebuild beats the held build | The same rebase as on main |
| Takeover by a later FCU with the same attributes (new id) | `fork()` the tree for the new loop; the old id keeps its stopped build and its tree as `PayloadOrTask::Unsealed`, so the timeout purge (which matches timed-out `Task`s) does not drop it |
| IL check on import | Not affected: it runs at the pre-withdrawals state; the tree only produces the post-state root |
| `getPayload` | Same as main |

`fork()`: clone each trie's root `NodeRef` and dirty and pending-removal sets, and reopen its `TrieDB` at the parent root (`Trie` is not `Clone`, because its DB is a `Box<dyn TrieDB>`). Trie writes are copy-on-write (`Arc::make_mut` in `NodeRef::get_node_mut`), so the forks share every unchanged node; a fork costs about the size of the dirty sets. The PreSealer needs a `fork()` request that returns a new worker with the forked tree and the same `held` diff.

Porting means: add `fork()` to `ShardedStateTrie` and `PreSealer`, call it in the takeover's context clone, and submit a new diff after each IL append.

## Testing and measurement

- **Merkleizer equivalence.** Random parent states and random write batches (new accounts, EIP-161 removals, slots cleared to zero, hot accounts with 2048 or more slots). After each batch, `root()` equals the serial `apply_account_updates_batch` from the parent with the combined diff.
- **Rebase equivalence.** From random held and target diffs, the rebased root equals a fresh one-shot root of the target, including the `removed_storage` reset.
- **Import.** `finish()` returns the same root and the same node sets (compared as sets) as the current BAL path on the same input. EF blockchain tests and `ethrex_tests` pass. Import benchmark before and after with `import-bench --with-bal` (`tooling/import_benchmark`) on the bal-devnet mainnet-mix chain.
- **Payload.** `unsealed_payload_tests` pass. A pre-sealed `getPayload` gives the same state root as a one-shot seal of the same build. A timed-out build gives `UnknownPayload`: the loop's clock moves to `tokio::time::Instant`, so a paused-clock test reaches 12 s without waiting.
- **Measurement.** Rerun the seal-scaling and large-diff harnesses (`harness/seal_scaling_tmp.rs`, `harness/seal_diff_tmp.rs`; copied into `test/tests/blockchain/` only while measuring) and the release latency test. Expected: `getPayload` pays for the post-tx phase plus the last change.

## Results

Measured 2026-11-05 on `feat/payload-preseal` (release, in-memory store, the harnesses above copied into the test tree for the run). The 1-min load was 4.5 to 11.5: under the plan's limit of 12, but the machine was not idle. The baseline was not rerun; "Before" is the Background numbers.

Each harness configuration ran its reps twice, as getPayload would see them:

- **One-shot:** cancel right after the initial build, before the first pre-seal finished (the runs confirm it had not). The seal either waits for that in-flight pre-seal or replaces it; either way it pays for a full sharded root, which is what L2 and explicit builds now pay.
- **Pre-sealed:** wait 2 s first, so the first pre-seal is done and the seal pays for the post-tx phase plus the final rebase.

Large diffs (`seal_diff`: a 1M-slot contract, 1M accounts), seal median of 3:

| Workload | Slots | Before (serial) | One-shot | Pre-sealed |
| --- | --- | --- | --- | --- |
| UpdateHot | 12k (61M gas) | 64 ms | 13.8 ms | 2.9 ms |
| UpdateHot | 30k | | 56.4 ms | 13.2 ms |
| UpdateHot | 60k (304M gas) | 294 ms | 67.4 ms | 15.6 ms |
| ClearHot | 12k | | 22.3 ms | 2.6 ms |
| ClearHot | 60k | | 87.6 ms | 19.2 ms |
| UpdateSpread | 11.4k | | 18.2 ms | 3.1 ms |
| UpdateSpread | 57k | | 50.2 ms | 12.8 ms |

Small and medium diffs (`seal_scaling`): with a 2 s `sleep` as the wait, the pre-sealed seal came out slower than the one-shot, even where the state root is trivial. A rerun replaced the sleep with a 2 s busy spin (the pre-seal still finishes first), which showed that the slowdown came from the idle CPU, not the pre-seal. Seal median of 5:

| Workload | One-shot | Pre-sealed, after a 2 s sleep | Pre-sealed, after a 2 s spin |
| --- | --- | --- | --- |
| FixedRecipients, 16000 txs (about 70 accounts) | 24.8 ms | 54.7 ms | 25.0 ms |
| FreshSlots, 8000 txs | 18.2 ms | 36.3 ms | 14.2 ms |
| FreshRecipients, 16000 txs | 44.4 ms | 58.6 ms | 28.6 ms |

With a small state diff the seal is mostly the transactions and receipts roots, which this design does not change: FixedRecipients seals in 25 ms either way.

`getpayload_latency_under_sustained_mempool_inflow`, release, 3 runs at a 1-min load of 10 to 11.5: passes 3 of 3. `get_payload` took 3.75, 3.12 and 2.81 ms against one full build of 9.7, 8.8 and 14.3 ms. Before, it failed 3 of 3 (4.0 to 4.3 ms against a limit of 4.0 to 4.15 ms). Builds were slower under this load, which loosens the `build / 2` bound, so the pass alone overstates the gain. In absolute terms the median `get_payload` went from about 4.1 ms to 3.1 ms.

Not measured: memory (`harness/mem_probe_tmp.rs` was not rerun), RocksDB or a devnet, and the import benchmark (no bal-devnet `import-bench` database on the machine).

## Behavior changes for the PR body

- `getPayload` seals from the pre-sealed tree; its cost is the post-tx phase plus the last change, instead of a full state root.
- The seal's state root is sharded across 16 threads in every path, L2 `build_payload` included.
- `MAX_PAYLOADS` is 4.
- A build that times out is dropped; a late `getPayload` gets `UnknownPayload`.
- The extra rebuild at slot timeout is gone.
- Up to 4 running loops each hold a tree for their build (memory table above).

## Out of scope

- Moving the streaming `handle_merkleization` onto the new type.
- Making mempool rebuilds append to the held build.
- Pruning reverted paths or resetting the tree on growth.
- A release-mode or real-state (RocksDB) measurement on a devnet; useful before marking #7496 ready.

## Risks

- The import refactor touches a consensus path for Amsterdam blocks; the node-set equivalence test and the benchmark guard it.
- `state_diff()` must match `get_state_transitions` exactly; the rebase equivalence test covers it through the root, and a unit test compares the two update lists on the same VM state.
- The `getpayload_latency` test's bound (`get_payload < build / 2`) is not changed by this design; whether it passes in release depends on the post-tx cost. It is not edited without confirmation.
