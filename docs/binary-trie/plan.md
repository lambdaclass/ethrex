# EIP-7864 Binary Trie Backend — Implementation Plan

## 1. Executive Summary

Add an EIP-7864 binary trie backend to ethrex as a research-oriented, opt-in overlay for L1 mainnet followers. We port the existing binary trie data structure from the `eip-7864-plan` branch wholesale, implement `StateReader`/`StateCommitter` for it as `BinaryBackend`, and compose it with the existing `MptBackend` through a `TransitionBackend` that implements EIP-7612 pure-overlay semantics (reads: overlay then MPT; writes: overlay only, with atomic first-write CoW for `AccountInfo`). Activation is gated behind a new `--binary-transition` CLI flag and fires **fully automatically** when two runtime preconditions hold simultaneously: snap sync complete (`snap_enabled=false`) and follower caught up to finalized head (`caught_up=true`). No admin RPC, no manual trigger. The implementation is one-way: reorgs deeper than 128 blocks (the MPT layer cache depth) are already fatal on any non-archive ethrex node and that property is inherited here; no new reorg-detection logic is added. The MPT is frozen at the switch block and never migrated; the binary trie grows organically from post-switch writes. The implementation is one-way (no genesis-binary, no rollback) and does not propose blocks or verify binary state roots against block headers. **Activation is restart-required** (not hot-swap) to avoid mutating the `Store`'s backend under concurrent readers.

## 2. Goals and Non-Goals

### Goals
- Port the `eip-7864-plan` binary trie crate into `crates/common/binary-trie/` (new hyphenated crate).
- Implement `BinaryBackend: StateReader + StateCommitter` paralleling `MptBackend`, with `fn hash(&mut self) -> Result<H256, StateError>` matching the real trait.
- Implement `TransitionBackend { base: MptBackend, overlay: BinaryBackend }` with pure-overlay read/write semantics and the **overlay stem integrity invariant** (see §3).
- Add `BackendKind::Binary`, `BackendKind::Transition`, `NodeUpdates::Binary`, `Merkleizer::Binary`, `Merkleizer::Transition`, `StateBackend::Binary`, `StateBackend::Transition`.
- Wire a `binary_wiring.rs` parallel to `mpt_wiring.rs` (FKV generator, trie provider, disk commits, layer cache glue).
- Add `--binary-transition` CLI flag and **fully automatic** runtime activation logic (fires when `snap_enabled=false` AND `caught_up=true`) that persists metadata and exits the process for operator restart.
- Add `eth_getBinaryProof` RPC method; make `eth_getProof` return a structured error in binary/transition mode.
- Add metrics counters/gauges for binary trie operations.
- Emit execution witness only in MPT mode; return error otherwise.
- Unit tests per module against ported Python-generated test vectors; integration tests for overlay read/write/tombstone/code-chunk/restart/reorg semantics.

### Non-Goals
- No fresh-from-genesis binary trie (attempting to start in `Binary` mode errors out).
- No state-root verification against block headers post-switch (header carries MPT root; we do not compare).
- No block proposal in binary/transition mode; follower-only.
- No zkVM guest integration for binary trie.
- No snap sync for binary trie; snap stays MPT-only.
- No migration of existing MPT data into binary (overlay only).
- No EF blockchain-test integration in this PR.
- No rollback/cancel of transition. Reorgs beyond the MPT layer cache's 128-block depth are already fatal on any non-archive ethrex node regardless of backend; this feature inherits that property and does not add explicit reorg-vs-switch-block detection.
- **No hot-swap of the backend at runtime** — activation writes metadata and exits for operator restart.

## 2a. Implementation Rules (hard constraints)

These rules apply to every phase, every task, every PR in this series. They are not guidelines.

1. **No deferrals, no skipping.** Every task in this plan must be implemented as specified. A task may not be skipped, postponed to a follow-up PR, replaced by a stub, or marked "not needed" — regardless of who proposes it (implementer, reviewer, or agent). If a task turns out to be genuinely blocked, the implementer stops and surfaces the blocker to the human for an explicit decision. Silent deferrals are the single most common way plans rot into half-implementations; that path is closed.
2. **No `TODO`, `FIXME`, `todo!()`, or `unimplemented!()` in merged code.** Inter-phase scaffolding is allowed during a phase's execution but each phase's checkpoint verifies the phase's scaffolding has been replaced with real implementations. A phase cannot be marked complete while any of its own introductions contain these markers. `unreachable!()` with a justifying comment is permitted only for variants documented as unreachable in the spec (e.g., `BackendKind::Binary` reachable only via transition).
3. **No "v1 fallback" or "acceptable for v1" shortcuts.** If the Locked Design Decisions table says "16-shard rayon workers", the implementation ships with 16-shard rayon workers. An implementer discovering a hidden difficulty does NOT pick a simpler path unilaterally; they surface it.
4. **No scope growth either.** Conversely, tasks outside the locked §2 Non-Goals do not get added just because they'd be nice. Out-of-scope is as firm as in-scope.
5. **Every checkpoint is a hard gate.** A phase's checkpoint criteria are not aspirational. If `cargo test` fails, the phase is not complete. If a named test is missing, the phase is not complete.
6. **Reviewer escalation over silent acceptance.** If `plan-reviewer` or `code-reviewer` flags a missing task, the response is to implement the task, not to argue it wasn't needed. "Already covered" / "redundant" / "implicit" are disallowed justifications unless the plan itself explicitly marks the item as implicit-in-X.

## 3. Locked Design Decisions

| Decision | Choice | Rationale |
|---|---|---|
| Transition semantics | EIP-7612 pure overlay with atomic-group CoW on first write | Avoids expensive migration; MPT stays canonical until deletable |
| Overlay stem integrity | First post-switch write to an MPT-resident account atomically writes all 4 `BASIC_DATA` sub-leaves + `CODE_HASH` to overlay (copying any unset sub-leaves from MPT) | Reads are branch-free: an account is either "in overlay" or "in base"; prevents partial-stem read hazard |
| Hash | BLAKE3 (locked) | Matches EIP-7864 spec; avoids extra abstraction |
| Key derivation | `get_tree_key` / `get_stem_for_base` (BLAKE3) | Per EIP |
| Sparse-stem values | `BTreeMap<u8, [u8; 32]>` | ~100–200 B per stem vs. 8.5 KB dense |
| Code chunking | EIP-7864 (31-byte payload + 1-byte PUSH offset, at `CODE_OFFSET + chunk_id`) | Spec compliance |
| Post-switch code resolution | Dual-write: chunks are written to binary trie for state_root correctness; `code_hash → bytecode` is also written to legacy `AccountCodes` table. **All code reads go through `AccountCodes` by `code_hash`** | Matches MPT semantics; no chunk-reconstruction path needed |
| Pre-switch code | Fetched from legacy `AccountCodes` table via `code_hash` | No re-chunking of pre-switch contracts |
| State root validation | Disabled post-switch | Header has MPT root; comparing binary root is useless |
| Activation model | **Restart-required**; activation writes the 4 metadata keys + format byte atomically, logs, exits process | Avoids mutating `Store`'s `StateBackend` under concurrent readers (VmDatabase refs, `Arc<Store>` in RPC handlers) |
| Format marker bytes | `0=Mpt`, `1=Binary`, `2=Transition` | `BackendKind::Transition` is a persisted, restart-recoverable state |
| FKV tables | `BINARY_FLATKEYVALUE` (new; separate per backend). **Overrides `docs/shared-trie/adding-a-backend.md`** spec of shared FKV — doc to be updated in Phase 9 | User preference; cleanest separation, no discriminator coupling |
| DB tables | `BINARY_TRIE_NODES` (new) | Separate namespace, u64 `NodeId` keys, `0xFF`-prefixed meta keys |
| Tombstone (trie layer) | Reserved side-table keys `[0xFE, stem...]` in `BINARY_TRIE_NODES` (not a trie leaf) | Avoids reserving a sub-index byte; O(1) lookup; persists across restarts |
| Tombstone (cache layer) | Explicit framed sentinel in `TrieLayerCache`: entries prefixed `[0x00, ...value]` for a real value, `[0x01]` (single byte) for a tombstone. Empty `Vec<u8>` is not used as a sentinel | Disambiguates from "not present" which is `None` |
| Empty binary root | `pub const EMPTY_BINARY_ROOT: H256 = H256([0u8; 32])` in `binary-trie/src/hash.rs`; never conflated with `EMPTY_TRIE_HASH` | Distinct semantics, distinct constant |
| Layer cache | Two independent caches: existing `TrieLayerCache` (MPT) + new `BinaryTrieLayerCache` (binary) | Different key spaces, different value shapes |
| Witness generation | Disabled in `Binary` / `Transition` mode (returns error) | zkVM out of scope |
| Reorg policy | ≤128 blocks handled by layer caches; crossing switch block = fatal | One-way transition |
| Genesis-binary | Unsupported; returns error | Entry path is only via transition |
| Parallel merkleization | **Single-tree, level-parallel via `rayon::par_iter`.** Workflow: (1) apply updates serially to one `BinaryTrieState`; (2) walk the tree once to bucket dirty nodes (cached_hash=None) by depth; (3) process levels bottom-up, `par_iter` over each level's dirty nodes — within a level all hashes are independent by construction. No sharding, no workers, no channels. Uses `Blockchain::merkle_pool: Arc<rayon::ThreadPool>`. | Binary trie's 2-way branching does not admit the 16-way sharding MPT uses (shard-by-top-4-bits creates a shared skeletal spine). Level-parallel merkelize scales with core count, targets the actual bottleneck (BLAKE3 throughput), avoids coordination complexity, and fits the binary tree's structure directly. Lives alongside `MptMerkleizer` as its own thing; no pretense of architectural parity. |
| Sparse StemNode hashing | StemNode's internal 256-leaf merkle short-circuits on `hash([0;64]) = [0;32]` per EIP-7864. A stem with K occupied sub-indices rehashes in ~K·8 hashes instead of ~511 (full subtree). | ~30× speedup per-stem on typical occupancies (1-5 slots). Largest single algorithmic win; orthogonal to outer parallelism. |
| `BinaryMerkleizer` API surface | Streaming `feed_updates(Vec<AccountUpdate>) / finalize() -> MerkleOutput`, identical shape to `MptMerkleizer` at the public boundary. Held inside `Merkleizer::Binary` / `Merkleizer::Transition` enum arms. Integrates with `execute_block_pipeline`'s merkleizer thread with zero call-site changes. **Internal implementation is single-threaded apply + level-parallel merkelize — no shard workers.** | Pipeline code stays backend-agnostic; internal implementation is free to optimize for the tree's own structure. |
| Per-backend storage primitives | Binary backend provides layer cache (`BinaryTrieLayerCache` mirroring `TrieLayerCache` — same bloom filter, same 128-layer commit threshold), trie-update worker dispatch (`NodeUpdates::Binary` arm in `apply_trie_updates` / `write_node_updates_direct`), FKV table (`BINARY_FLATKEYVALUE`), FKV-backed fast reads. **FKV population is inline on commit** (not via background generator) because binary trie starts empty post-switch and grows only from commits; snap sync stays MPT. **Merkleization is NOT sharded** (see the merkleization row above); parity with MPT applies only at the storage / cache / layer level where the two backends genuinely share requirements. | Acknowledges that merkleization and storage have different parallelism characteristics between the two backends. |
| RPC | `eth_getProof` returns `{code: -32099}` with pointer message; new `eth_getBinaryProof` | Preserves JSON-RPC shape guarantees |

## 4. Crate / Module Layout Diff

### New
```
crates/common/binary-trie/                  # new crate (hyphenated on disk)
  Cargo.toml
  lib.rs, error.rs, hash.rs, key_mapping.rs,
  node.rs, node_store.rs, merkle.rs, trie.rs,
  proof.rs, state.rs, db.rs, layer_cache.rs, witness.rs
  backend.rs                                # NEW: BinaryBackend impl of StateReader/StateCommitter
  merkleizer.rs                             # NEW: BinaryMerkleizer with inherent feed_updates / finalize
  testgen/generate_vectors.py
  testgen/test_vectors.json                   # (originally placed under tests/; consolidated to testgen/ in Phase 2 follow-up e6ff086e68)
  testgen/vectors_{accounts,storage,codechunk,negative}.json  # Phase 2
  tests/test_vectors.rs                       # uses include_str!("../testgen/<file>.json") after Phase 2
  tests/state_backend.rs                    # NEW: StateReader/StateCommitter conformance
crates/storage/binary_wiring.rs             # NEW: FKV gen, trie provider, disk commits, cache layer
crates/storage/transition_wiring.rs         # NEW: TransitionBackend wiring, activation, restart
docs/binary-trie/
  plan.md                                   # (this file)
  overview.md
  design-decisions.md
  rpc.md
  testing.md
  operational.md
```

### Modified
```
crates/common/state-backend/src/lib.rs      # BackendKind::{Binary,Transition}, NodeUpdates::Binary
crates/storage/state_backend.rs             # StateBackend::{Binary,Transition}, genesis dispatch
crates/storage/merkleizer.rs                # Merkleizer::{Binary,Transition}
crates/storage/api/tables.rs                # BINARY_TRIE_NODES, BINARY_FLATKEYVALUE, transition meta keys, TABLES array + size bump
crates/storage/store.rs                     # backend_kind_to_byte(1,2); byte_to_backend_kind; apply_trie_updates dispatch; write_node_updates_direct dispatch; from_backend restart path
crates/networking/p2p/sync_manager.rs       # surface snap_enabled + caught-up hook for activation gate
crates/blockchain/blockchain.rs             # no logic change; activation module hooks observer
crates/networking/rpc/eth/account.rs        # eth_getProof branches on BackendKind; new GetBinaryProofRequest
crates/networking/rpc/rpc.rs                # register eth_getBinaryProof
cmd/ethrex/cli.rs                           # add --binary-transition flag
cmd/ethrex/initializers.rs                  # wire flag into runtime
docs/shared-trie/adding-a-backend.md        # update FKV-shared claim → per-backend; update NodeUpdates::Binary definition (add deleted_stems)
Cargo.toml (workspace)                      # add ethrex-binary-trie member + workspace dep
```

### Not modified
- `ethrex-common`
- `ethrex-trie` (MPT backend is untouched)
- `ethrex-vm`, `ethrex-levm`, `ethrex-guest-program`
- All of `crates/l2/`

## 5. Core Types Introduced

```rust
// crates/common/state-backend/src/lib.rs
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendKind { Mpt, Binary, Transition }

#[expect(clippy::type_complexity)]
pub enum NodeUpdates {
    Mpt { state_updates: Vec<(Vec<u8>, Vec<u8>)>,
          storage_updates: Vec<(H256, Vec<(Vec<u8>, Vec<u8>)>)> },
    Binary {
        node_diffs: Vec<(Vec<u8>, Vec<u8>)>,
        deleted_stems: Vec<[u8; 31]>,
        // Inline FKV updates — binary trie populates its flat-state table inline
        // on commit rather than via a background generator.
        // Key: 32-byte `stem || sub_index`; value: Some(leaf bytes) or None for deletion.
        fkv_entries: Vec<([u8; 32], Option<[u8; 32]>)>,
    },
}

// Trait excerpt (already present in this repo; do not widen):
// trait StateCommitter {
//     fn hash(&mut self) -> Result<H256, StateError>;
//     fn commit(self) -> Result<MerkleOutput, StateError>;
//     ...
// }
```

```rust
// crates/storage/state_backend.rs
pub enum StateBackend {
    Mpt(MptBackend),
    Binary(BinaryBackend),
    Transition(TransitionBackend),
}

pub struct TransitionBackend {
    pub base: MptBackend,        // read-only, frozen at switch block
    pub overlay: BinaryBackend,  // all writes go here
    pub switch_block: u64,
    pub frozen_mpt_root: H256,
}
```

```rust
// crates/storage/merkleizer.rs
pub enum Merkleizer {
    Mpt(MptMerkleizer),
    Binary(BinaryMerkleizer),
    Transition(BinaryMerkleizer), // wraps binary; MPT side is frozen
}
```

```rust
// crates/common/binary-trie/backend.rs
pub struct BinaryBackend {
    state: BinaryTrieState,   // from ported state.rs
    code_reader: CodeReader,  // legacy MPT code table for pre-switch AND post-switch contracts
    storage_opener: Arc<dyn BinaryTrieProvider>, // new dep-inversion seam
    // Tombstones for SELFDESTRUCT (trie-layer stem tombstones)
    deleted_stems: FxHashSet<[u8; 31]>,
    // Side-index for storage enumeration (binary trie lacks prefix iteration)
    storage_keys: RwLock<FxHashMap<Address, FxHashSet<H256>>>,
}

pub trait BinaryTrieProvider: Send + Sync {
    fn load_node(&self, id: u64) -> Result<Option<Vec<u8>>, BinaryTrieError>;
    fn load_meta(&self, key: &[u8]) -> Result<Option<Vec<u8>>, BinaryTrieError>;
    fn is_deleted_stem(&self, stem: &[u8; 31]) -> Result<bool, BinaryTrieError>;
}

// StateCommitter impl:
// fn hash(&mut self) -> Result<H256, StateError>;    // NOTE the Result — must propagate
```

```rust
// crates/common/binary-trie/merkleizer.rs
pub struct BinaryMerkleizer {
    state: BinaryTrieState,
    code_updates: Vec<(H256, Code)>,
    accumulator: Option<FxHashMap<Address, AccountUpdate>>,
}

impl BinaryMerkleizer {
    pub fn new(parent_root: H256, precompute_witnesses: bool,
               provider: Arc<dyn BinaryTrieProvider>) -> Result<Self, StateError>;
    pub fn feed_updates(&mut self, updates: Vec<AccountUpdate>) -> Result<(), StateError>;
    pub fn finalize(self) -> Result<MerkleOutput, StateError>;
}
```

```rust
// crates/common/binary-trie/hash.rs
pub const EMPTY_BINARY_ROOT: H256 = H256([0u8; 32]);
// Framing for TrieLayerCache entries in the binary namespace:
pub const CACHE_VALUE_TAG: u8 = 0x00;      // entry = [0x00 || real_value_bytes]
pub const CACHE_TOMBSTONE_TAG: u8 = 0x01;  // entry = [0x01] (single byte, unambiguous)
```

## 6. Phase Breakdown

### Phase 0 — Preflight (clean-context entry)

**For a fresh agent picking this plan up with no prior conversation history.** Do NOT skip. Every item must be completed before Phase 1 begins. If any item fails, STOP and escalate per §11 (Escalation Protocol); do not work around the failure.

#### 0a. Reading order (in this sequence)

1. `docs/binary-trie/overview.md` — scope, crate layout, activation state machine.
2. `docs/binary-trie/design-decisions.md` — why each locked choice was made.
3. This document's §2a (Implementation Rules) — the six hard constraints, especially "no deferrals, no skipping".
4. This document's §3 (Locked Design Decisions) — the full table.
5. This document's §9a (Resolved Questions) — what was considered and explicitly ruled out.
6. `docs/binary-trie/operational.md` — user-facing behavior you're implementing.
7. `docs/binary-trie/rpc.md` — wire format for `eth_getBinaryProof` (Phase 8 reference).
8. `docs/binary-trie/testing.md` — test-vector workflow.
9. `docs/shared-trie/spec.md` §§ 1–4 and 8 — the abstraction layer you're building on top of.
10. `docs/shared-trie/adding-a-backend.md` — the onboarding guide this PR is the first real test of.

Do NOT read any other docs, source code, or branches until Phase 0 items 0b–0e are complete.

#### 0b. Environment verification

Run each command; every one MUST succeed before proceeding. All paths are absolute.

```bash
# Current branch must be `shared-trie-binary` (already branched from shared-trie by the user).
git -C /data2/edgar/work/ethrex rev-parse --abbrev-ref HEAD
# Expected output: shared-trie-binary

# Working tree must be clean. If not, escalate — do not stash or commit unknown changes.
git -C /data2/edgar/work/ethrex status --porcelain
# Expected output: (empty)

# Shared-trie refactor must be merged into this branch's history. Confirm the landing commit is reachable.
git -C /data2/edgar/work/ethrex log --oneline shared-trie-binary | rg -c "shared trie abstraction"
# Expected output: 1

# Workspace currently compiles.
cd /data2/edgar/work/ethrex && cargo check --workspace
# Expected: exit code 0.

# MPT backend tests currently pass.
cd /data2/edgar/work/ethrex && cargo test -p ethrex-trie
# Expected: all tests pass.

# Reference branch is accessible via gh API.
gh api repos/lambdaclass/ethrex/git/trees/eip-7864-plan --jq '.sha' >/dev/null
# Expected: exit code 0.
```

#### 0c. Ground-truth checks against the codebase

The plan references specific files and line numbers. Verify they still hold (the plan was written on this branch; if commits have landed since, some line numbers may have shifted — absolute paths are authoritative, line numbers are cross-refs).

- `crates/common/state-backend/src/lib.rs` exists and exports `StateReader`, `StateCommitter`, `AccountMut`, `MerkleOutput`, `NodeUpdates`, `StateError`, `BackendKind`, `CodeReader`.
- `crates/common/trie/backend.rs` exists and contains `pub struct MptBackend`.
- `crates/common/trie/merkleizer.rs` exists and contains `pub struct MptMerkleizer` with `feed_updates` / `finalize` methods.
- `crates/storage/state_backend.rs` exists with `pub enum StateBackend { Mpt(MptBackend) }` (single-arm).
- `crates/storage/merkleizer.rs` exists with `pub enum Merkleizer { Mpt(MptMerkleizer) }` (single-arm).
- `crates/storage/mpt_wiring.rs` exists.
- `crates/storage/api/tables.rs` has `pub const TABLES: [&str; N]` where `N >= 19` (grep for the exact current value; Phase 5 bumps it by 2).
- `crates/networking/p2p/sync.rs` has `pub enum SyncMode { Full, Snap }` and `snap_enabled: Arc<AtomicBool>` somewhere on the `Syncer` struct.
- `crates/blockchain/blockchain.rs` holds `merkle_pool: Arc<rayon::ThreadPool>` on the `Blockchain` struct.

Grep commands to verify:

```bash
cd /data2/edgar/work/ethrex
rg -l "pub trait StateReader" crates/common/state-backend/src/lib.rs
rg -l "pub struct MptBackend" crates/common/trie/backend.rs
rg -n "pub const TABLES:" crates/storage/api/tables.rs
rg -n "merkle_pool.*Arc<rayon::ThreadPool>" crates/blockchain/blockchain.rs
```

Every command must return at least one match. If any returns empty, STOP and escalate.

#### 0d. Reference-branch fetch protocol

Throughout phases you will fetch ported files from `eip-7864-plan`. Canonical fetch command:

```bash
gh api "repos/lambdaclass/ethrex/contents/crates/common/binary_trie/<FILE>?ref=eip-7864-plan" --jq '.content' | base64 -d > crates/common/binary-trie/<FILE>
```

Note the path transformation: source `crates/common/binary_trie/` (underscore, flat layout) → destination `crates/common/binary-trie/` (hyphen, workspace convention).

The 13 source files to fetch in Phase 1 (exact list):
`Cargo.toml`, `lib.rs`, `error.rs`, `hash.rs`, `key_mapping.rs`, `node.rs`, `node_store.rs`, `merkle.rs`, `trie.rs`, `proof.rs`, `state.rs`, `db.rs`, `layer_cache.rs`, `witness.rs`.

Plus: `testgen/generate_vectors.py`, `tests/test_vectors.json`, `tests/test_vectors.rs`. (Historical note: Phase 2 follow-up `e6ff086e68` moved `test_vectors.json` from `tests/` to `testgen/` for consistency with the other vector files. Port to `tests/test_vectors.json` in Phase 1 as written here; the move happens naturally when Phase 2 regen writes it to the new location.)

#### 0e. Constraint recap (read before every phase)

- **No deferrals, no skipping.** §2a rule 1.
- **No `TODO` / `todo!()` / `unimplemented!()` / `FIXME` in merged code.** §2a rule 2.
- **Every checkpoint is a hard gate.** §2a rule 5.
- **Reviewer escalation over silent acceptance.** §2a rule 6.

If you find yourself wanting to skip a task, replace it with a stub, or mark it "redundant with X", STOP and escalate per §11.

#### 0f. Phase 0 exit criteria

- [ ] All of §0a (reading) complete.
- [ ] All of §0b (environment) passes.
- [ ] All of §0c (ground-truth) passes.
- [ ] §0d fetch command verified with a test fetch of one small file (e.g., `hash.rs`; delete the test artifact after).
- [ ] §0e constraints acknowledged in a status note.

Only after all five boxes are ticked: begin Phase 1.

---

### Phase 1 — Port the `ethrex-binary-trie` crate (pure data structures)

**Entry criteria:** Clean tree on `shared-trie-binary` branched from `shared-trie`.

**Files created**
- `crates/common/binary-trie/Cargo.toml` — workspace member; deps: `blake3`, `bytes`, `ethrex-common`, `fastbloom`, `hex`, `lru`, `rustc-hash`, `thiserror`, `serde`. NO `ethrex-state-backend` yet.
- `crates/common/binary-trie/lib.rs`
- `error.rs, hash.rs, key_mapping.rs, node.rs, node_store.rs, merkle.rs, trie.rs, proof.rs, state.rs, db.rs, layer_cache.rs, witness.rs` — verbatim port from `eip-7864-plan` via `gh api ... | base64 -d`.

**Files modified**
- Workspace `Cargo.toml` — add `crates/common/binary-trie` to `members` and `[workspace.dependencies] ethrex-binary-trie = { path = "crates/common/binary-trie" }`.

**Tasks**
- [ ] Task 1.1: Fetch all 13 source files from branch `eip-7864-plan` and write to `crates/common/binary-trie/`. (Low)
- [ ] Task 1.2: Add crate to workspace `Cargo.toml` (`[workspace] members`, `[workspace.dependencies]`). (Low)
- [ ] Task 1.3: Run `cargo check -p ethrex-binary-trie` and fix any leftover rename fallout from `binary_trie` → `binary-trie` package name. (Low)
- [ ] Task 1.4: Ensure `lib.rs` re-exports `BinaryTrie`, `BinaryTrieState`, `BinaryTrieError`, `BinaryTrieWitness`, `BinaryTrieProof`, `TrieBackend`, `WriteOp`, `node_key`, `serialize_node`, `META_ROOT`, `META_NEXT_ID`. (Low)
- [ ] Task 1.5: Add `pub const EMPTY_BINARY_ROOT: H256 = H256([0u8; 32])` plus `CACHE_VALUE_TAG=0x00` and `CACHE_TOMBSTONE_TAG=0x01` in `hash.rs`. Re-export from `lib.rs`. (Low)
- [ ] Task 1.6: Port `tests/test_vectors.json` and `tests/test_vectors.rs`. Make them run under `cargo test -p ethrex-binary-trie`. (Low)
- [ ] Task 1.7: Port `testgen/generate_vectors.py` and a `testgen/README.md` explaining how to regenerate vectors. (Low)
- [ ] Task 1.8: Add `#![warn(unused_crate_dependencies)]` and rustdoc `//!` crate-level comment. (Low)
- [ ] **Checkpoint: Verify Phase 1 complete** — List every file; run `cargo test -p ethrex-binary-trie`; confirm vectors pass. `EMPTY_BINARY_ROOT` and framing tag constants exist. No stubs. (Low)

**Tests that MUST pass**
- `cargo test -p ethrex-binary-trie` (ported unit tests + vectors).

**Known hazards**
- Reference branch file names may collide with existing crates — `ethrex-binary-trie` vs. module `binary_trie`. Use hyphenated crate name, underscore module if needed.
- The reference branch uses `blake3 = "1"` directly; verify the workspace's pinned version matches so we don't double-link `blake3`.

**Handoff**: A standalone, passing `ethrex-binary-trie` crate with no knowledge of `state-backend`, `storage`, or `trie` crates.

---

### Phase 2 — Test vector expansion

**Entry criteria:** Phase 1 checkpoint passed. **Note:** Phase 2 does NOT block Phase 3. Phase 3 may begin after Phase 1 is green; Phase 2 may run in parallel with Phase 3 or strictly after it.

**Files created**
- `crates/common/binary-trie/testgen/vectors_accounts.json` — account update sequences (insert, overwrite, selfdestruct).
- `crates/common/binary-trie/testgen/vectors_storage.json` — storage slot sequences across multiple accounts.
- `crates/common/binary-trie/testgen/vectors_codechunk.json` — sequences exercising chunks at CODE_OFFSET boundaries (0, 127, 128, 1023).

**Files modified**
- `crates/common/binary-trie/testgen/generate_vectors.py` — extend to emit the three new vector files. Script must run from a repo checkout with `python3 ./testgen/generate_vectors.py`.
- `crates/common/binary-trie/tests/test_vectors.rs` — add `verify_accounts_vectors`, `verify_storage_vectors`, `verify_codechunk_vectors`. All three iterate the JSON, apply operations, and compare computed root + per-leaf values to vector expectations.

**Tasks**
- [ ] Task 2.1: Extend `generate_vectors.py` with account update generator (nonce/balance/code_hash cycles for 50 addresses). (Medium)
- [ ] Task 2.2: Extend `generate_vectors.py` with storage generator (100 slots across 10 accounts, including zero-writes for deletes). (Medium)
- [ ] Task 2.3: Extend `generate_vectors.py` with code-chunk generator producing keys at chunk_ids 0, 1, 127, 128, 1000, 1023. (Medium)
- [ ] Task 2.4: Regenerate all JSON vectors; commit JSON artifacts. (Low)
- [ ] Task 2.5: Add the three `verify_*_vectors` Rust test functions against the ported `BinaryTrie` and `BinaryTrieState`. (Medium)
- [ ] Task 2.6: Add negative-case vectors: absent key proof verification; selfdestructed-account-then-reinsert. (Medium)
- [ ] **Checkpoint: Verify Phase 2 complete** — `cargo test -p ethrex-binary-trie` runs all three vector suites and negative cases. (Low)

**Tests that MUST pass**
- All ported + all new vector tests.

**Known hazards**
- Python reference implementation must match EIP-7864 exactly, including the `hash([0x00]*64) → [0x00]*32` merkelization special case (NOT applied to `tree_hash`).
- JSON byte ordering: confirm big-endian U256 encoding in vector generator matches `U256::to_big_endian` in Rust.

**Handoff**: Frozen cross-language vectors; any future trie change that breaks them is a regression.

---

### Phase 3 — `BinaryBackend` implementing `StateReader` + `StateCommitter`

**Entry criteria:** Phase 1 checkpoint passed (Phase 2 is NOT a blocker for this phase).

**Files created**
- `crates/common/binary-trie/backend.rs` — `BinaryBackend` struct + `BinaryTrieProvider` trait.

**Files modified**
- `crates/common/binary-trie/Cargo.toml` — add `ethrex-state-backend`, `ethrex-crypto` (for keccak of `code_hash`) workspace deps.
- `crates/common/binary-trie/lib.rs` — `pub mod backend; pub use backend::{BinaryBackend, BinaryTrieProvider, EmptyBinaryTrieProvider};`.
- `crates/common/state-backend/src/lib.rs` — extend `NodeUpdates` with `Binary { node_diffs, deleted_stems }`; extend `BackendKind` with `Binary` and `Transition`.
- `crates/storage/store.rs` — update `backend_kind_to_byte` and `byte_to_backend_kind` to handle bytes `1` and `2`; update the unit tests in `backend_format_tests`.

**Tasks**
- [ ] Task 3.1: Define `BinaryTrieProvider` trait in `backend.rs` with `load_node`, `load_meta`, `is_deleted_stem` methods. Provide `EmptyBinaryTrieProvider` that returns `None`/`false` for in-memory/genesis paths. (Low)
- [ ] Task 3.2: Define `BinaryBackend` struct (see §5). Add `new()` (empty), `new_with_db(provider, code_reader)`, `from_state(BinaryTrieState, ...)`, `from_witness(...)`. (Medium)
- [ ] Task 3.3: Implement `StateReader` for `BinaryBackend`:
  - `account(addr)`: compute stem via `get_stem_for_base`; read `basic_data` leaf at sub-index `BASIC_DATA_LEAF_KEY`; read `code_hash` leaf at `CODE_HASH_LEAF_KEY`. Because of the **overlay stem integrity invariant** (Task 3.4), if `basic_data` is present then `code_hash` must also be present; if either is absent and the stem is not tombstoned, return `None` (caller falls through to MPT in Transition mode). Unpack to `AccountInfo`. (Medium)
  - `storage(addr, slot)`: compute key via `get_tree_key_for_storage_slot`; return `H256::zero()` if absent.
  - `code(addr, code_hash)`: delegate to `code_reader` (legacy MPT code table). Post-switch deploys are dual-written there too (see Task 3.8); reads NEVER reconstruct from chunks. (Medium)
- [ ] Task 3.4: Implement `StateCommitter::update_accounts` with **atomic stem-group writes**:
  - For `None` account (selfdestruct): add stem to `deleted_stems`; remove all leaves under that stem from the trie.
  - For `Some(info)`: pack `(version=0, code_size, nonce, balance)` into 32 bytes via `pack_basic_data`; insert at `BASIC_DATA_LEAF_KEY`. Insert `code_hash` at `CODE_HASH_LEAF_KEY`. Both inserts happen together — never write one without the other. In `TransitionBackend` (Phase 6) the first write also CoW-copies from MPT; in standalone `BinaryBackend` the struct itself has no MPT, so the invariant holds by construction. (Medium)
- [ ] Task 3.5: Implement `StateCommitter::update_storage`: for each `(slot, value)`, compute key via `get_tree_key_for_storage_slot`, insert or delete; update `storage_keys` side-index. (Medium)
- [ ] Task 3.6: Implement `StateCommitter::clear_storage`: look up `storage_keys[addr]`; remove each key from the trie; clear the index. (Medium)
- [ ] Task 3.7: Implement `StateCommitter::hash` and `commit` with the correct signatures:
  - `fn hash(&mut self) -> Result<H256, StateError>` — call `merkelize(&mut self.trie)` on the inner state; map any `BinaryTrieError` to `StateError::Other(...)`. **Do NOT drop the `Result`.**
  - `commit(self) -> Result<MerkleOutput, StateError>`: collect all dirty node diffs via `NodeStore::take_dirty()` (need to add this accessor in `node_store.rs`) into `Vec<(Vec<u8>, Vec<u8>)>` (8-byte node IDs as keys + serialized nodes as values, plus meta keys `META_ROOT` and `META_NEXT_ID`). Drain `deleted_stems` into `NodeUpdates::Binary.deleted_stems`. Return `MerkleOutput { root, node_updates: NodeUpdates::Binary{...}, code_updates, accumulated_updates }`. Tombstone encoding at the `TrieLayerCache` layer uses `[CACHE_TOMBSTONE_TAG]` (single byte `0x01`); real values are framed as `[CACHE_VALUE_TAG, ...bytes]`. This framing is applied when constructing the cache layer (Phase 5 Task 5.6), not in `commit` itself. (High)
- [ ] Task 3.8: Add `BinaryBackend::code_chunks_from_bytecode(bytes) -> Vec<([u8; 32], [u8; 32])>` helper that chunkifies code and returns key/value pairs to be inserted alongside basic_data when a contract is deployed. Plumb it in `update_accounts` when `acct_mut.code` is `Some`. **Also** return the `(code_hash, bytecode)` pair in the `code_updates` vec so the legacy `AccountCodes` table receives a dual-write; this is how post-switch code retrieval works. (Medium)
- [ ] Task 3.9: Unit tests in `backend.rs` for: basic account insert/read, storage insert/read/delete, selfdestruct produces tombstone, code deployment inserts chunks AND produces a `code_updates` entry for `AccountCodes`, `hash()` returns `Err` path when the provider errors. (Medium)
- [ ] **Checkpoint: Verify Phase 3 complete** — `cargo check -p ethrex-binary-trie` passes; all new tests pass; `NodeUpdates::Binary` and `BackendKind::{Binary,Transition}` exist; byte mapping is `0/1/2`; `hash` returns `Result<H256, StateError>`; stem-group write invariant has a test. (Low)

**Tests that MUST pass**
- `cargo test -p ethrex-binary-trie` (all vectors + new backend tests).
- `cargo test -p ethrex-storage backend_format_tests` (updated byte mapping).

**Known hazards**
- `BinaryTrieState::apply_account_update` on the reference branch uses specific field semantics; audit it matches our `AccountMut` shape (especially `code_size` propagation).
- Ported `state.rs` uses its own `AccountUpdate` or `GenesisAccount`; we must map ethrex `AccountInfo`/`AccountUpdate` at the boundary, not expose reference types.
- The stem-group write invariant means `update_accounts` MUST always emit both `BASIC_DATA` and `CODE_HASH` leaves together, even on pure balance/nonce updates. Enforce with a helper; add a debug assertion.

**Handoff**: `BinaryBackend` can be constructed and passes `StateReader`/`StateCommitter` conformance tests in isolation.

---

### Phase 4 — `BinaryMerkleizer`: single-tree, level-parallel merkelize, sparse stem hashing

**Entry criteria:** Phase 3 checkpoint passed.

**Design principle for this phase: maximize BLAKE3 throughput on the binary trie's natural DAG structure.** Apply and merkelize are the two phases: apply is cheap (bit-path traversal, no hashing), merkelize IS the work (~100k BLAKE3 calls for a 4000-stem mainnet block). The architecture must target merkelize throughput. Sharding-by-top-4-bits (the MPT pattern) does not fit binary trie's 2-way branching — a shard would create a 4-level skeletal spine above its "real" subtree, breaking clean root-combination. Instead:

1. **Serial apply** — one `BinaryTrieState`, serial `apply_account_update` loop in `feed_updates`. Apply is ~1-5 ms for a 10k-update block; not worth parallelizing.
2. **Level-parallel merkelize** — at `finalize`, walk the tree once to collect dirty nodes (those with `cached_hash == None`) bucketed by depth. Process levels bottom-up; within a level, `rayon::par_iter` over dirty nodes. Correctness: within a single level, all hashes are independent (a node at level N depends only on children at level N+1 or deeper). Scales with core count, zero coordination overhead.
3. **Sparse StemNode hashing** — EIP-7864 specifies `hash([0x00] * 64) = [0x00] * 32`. A stem with K occupied sub-indices hashes in ~K·8 BLAKE3 calls (paths from occupied leaves to the stem subtree root, skipping zero siblings) instead of ~511 (full 256-leaf walk). ~30× speedup per stem on typical occupancies (1-5 slots). Orthogonal to outer parallelism.

**No shard workers, no crossbeam channels, no `DROP_SENDER`, no `catch_unwind` worker wrappers, no `watcher_rx`.** The public API (`feed_updates` / `finalize`) matches `MptMerkleizer` exactly at the enum-dispatch boundary so `execute_block_pipeline` sees no difference. Internal implementation is free to diverge. Uses `Blockchain::merkle_pool: Arc<rayon::ThreadPool>` for the level-parallel merkelize.

**Files created**
- `crates/common/binary-trie/merkleizer.rs` — `BinaryMerkleizer` struct, `new`, `new_bal`, `feed_updates`, `finalize`. Internally single-threaded apply + level-parallel merkelize. No inner worker module.

**Files modified**
- `crates/common/binary-trie/lib.rs` — `pub mod merkleizer; pub use merkleizer::BinaryMerkleizer;`.
- `crates/common/binary-trie/merkle.rs` — rewrite (or extend) the StemNode internal merkle so it short-circuits on `ZERO_HASH` and skips zero subtrees. Add per-stem fast path: K·8 hashes, not 511. Match against Python reference vectors to verify correctness.
- `crates/common/binary-trie/state.rs` — add `collect_dirty_levels(&self) -> Vec<Vec<NodeId>>` (single serial walk, bucket dirty nodes by depth). Add `merkelize_parallel(&mut self, pool: &rayon::ThreadPool) -> [u8; 32]` (level-by-level par_iter over dirty nodes). Keep the existing serial `state_root()` for tests.
- `crates/storage/Cargo.toml` — add `ethrex-binary-trie = { workspace = true }`.
- `crates/storage/state_backend.rs`:
  - Add `StateBackend::Binary(BinaryBackend)` arm. **Do NOT add `StateBackend::Transition`** — Phase 6 scope (requires `TransitionBackend` struct).
  - Add match arms to every `impl StateReader`, `impl StateCommitter`, and every inherent method. Compiler will flag every missed arm. `hash()` arms return `Result` and propagate errors.
- `crates/storage/merkleizer.rs`:
  - Add `Merkleizer::Binary(BinaryMerkleizer)` and `Merkleizer::Transition(BinaryMerkleizer)` arms; implement `feed_updates` + `finalize` for each (delegate). Both variants wrap the same `BinaryMerkleizer` because Transition's MPT side is frozen read-only and all merkelization happens in binary.
  - Add `Merkleizer::new_binary(parent_root, precompute_witnesses, provider, pool)` and `Merkleizer::new_bal_binary` constructors. Signature matches `new_mpt` / `new_bal_mpt` modulo the provider type.

**NOT in scope this phase (Phase 5/6):**
- `crates/storage/store.rs` `NodeUpdates::Binary` arms in `apply_trie_updates` / `write_node_updates_direct` — stay as Phase-3 `unreachable!()` scaffolding until Phase 5 replaces them with real calls into `binary_wiring.rs`.
- `StateBackend::Transition` variant — Phase 6.
- `binary_wiring.rs`, DB tables, FKV — Phase 5.

**Tasks**
- [ ] Task 4.1: Implement sparse StemNode internal merkelization in `merkle.rs`. A StemNode's 256-leaf merkle short-circuits when a subtree is entirely empty (all values `None`) by returning `ZERO_HASH`. Each rehash of a stem with K occupied leaves performs ~K·8 BLAKE3 calls instead of ~511. Test: reference-vector parity with the Python `_merkelize` for single-stem / empty-stem / sparse-stem / full-stem cases — bit-for-bit equality. (Large; single biggest win)
- [ ] Task 4.2: Implement `BinaryTrieState::collect_dirty_levels(&self) -> Vec<Vec<NodeId>>`. Serial tree walk. Returns a `Vec` indexed by depth; `result[d]` contains node IDs at depth `d` whose `cached_hash.is_none()`. Depth is measured from root (0) to leaves. StemNodes are their own "leaves" of the outer tree (no deeper in this structure). Add a unit test against a scripted sequence of updates. (Medium)
- [ ] Task 4.3: Implement `BinaryTrieState::merkelize_parallel(&mut self, pool: &rayon::ThreadPool) -> [u8; 32]`. Calls `collect_dirty_levels`, iterates levels bottom-up, uses `pool.install(|| level.par_iter().for_each(...))` or equivalent to hash each dirty node in parallel within a level. Each hash: read children's `cached_hash` (which is Some at this point because children are at a deeper level already processed), compute `merkle_hash_64(left || right)` for InternalNode or the sparse stem hash for StemNode, set `cached_hash`. Returns root hash. Uses interior mutability (AtomicCell, Mutex, or per-node cell) to set `cached_hash` under `&self`. Pick the cheapest pattern; document in a rustdoc. (Large)
- [ ] Task 4.4: Implement `BinaryMerkleizer` struct with fields: `state: BinaryTrieState`, `pool: Arc<rayon::ThreadPool>`, `provider: Arc<dyn BinaryTrieProvider>`, `code_updates: Vec<(H256, Code)>`, `fkv_entries: Vec<([u8; 32], Option<[u8; 32]>)>`, `deleted_stems: Vec<[u8; 31]>`, `accumulator: Option<Vec<AccountUpdate>>` (for precompute_witnesses). (Medium)
- [ ] Task 4.5: `BinaryMerkleizer::new(parent_root, precompute_witnesses, provider, pool)` — constructs a fresh state (or loads parent state via provider if Phase 5 wires that), initializes empty accumulators. No workers spawned. (Low)
- [ ] Task 4.6: `BinaryMerkleizer::feed_updates(&mut self, updates: Vec<AccountUpdate>) -> Result<(), StateError>` — serial loop calling `self.state.apply_account_update(&update)` for each. Collect `code_updates` inline when `update.code.is_some()`. Collect `fkv_entries` inline by diffing before/after leaf values (or by having `apply_account_update` return the diff; prefer the latter if the ported state supports it, else diff here). Handle `removed`/`removed_storage` flags via existing state methods. Clone into accumulator if enabled. (Medium)
- [ ] Task 4.7: `BinaryMerkleizer::finalize(self) -> Result<MerkleOutput, StateError>` — calls `state.merkelize_parallel(&self.pool)` to get root. Drains `node_diffs` from the dirty `NodeStore` via `BinaryTrieState::take_trie_dirty()` (Phase 3 accessor). Returns `MerkleOutput { root, node_updates: NodeUpdates::Binary { node_diffs, deleted_stems, fkv_entries }, code_updates, accumulated_updates }`. (Medium)
- [ ] Task 4.8: `BinaryMerkleizer::new_bal(...)` — BAL (Block Access List)-optimized constructor. Because apply is already serial and cheap, "BAL-optimized" for binary trie means: accept a pre-deduplicated list of account updates (BAL-sourced) and skip the dedup logic. Implementation: same as `new()` but sets a flag that causes `feed_updates` to skip merge-with-previous logic. Interface-level symmetry with `new_bal_mpt`, not behavioral parity. (Low)
- [ ] Task 4.9: Add `StateBackend::new_binary_with_db(provider, code_reader) -> StateBackend`. (Low)
- [ ] Task 4.10: Extend `StateBackend::apply_account_updates` with the `Binary` arm (delegates through `StateCommitter` trait methods, same body as MPT). (Low)
- [ ] Task 4.11: Extend `init_witness`, `record_witness_accesses`, `apply_updates_with_witness_state`, `advance_witness_to`, `finalize_witness`, `collect_witness_codes` for `Binary` arm: return `StateError::Other("witness generation unsupported on binary backend")`. (Low)
- [ ] Task 4.12: `StateBackend::compute_genesis_root` / `compute_genesis_block` already handle `BackendKind::Binary` / `Transition` via `panic!` (added in Phase 3 by user edit). Leave as-is; Phase 4 doesn't change this. (No-op task; documented for completeness.)
- [ ] Task 4.13: Add `Merkleizer::Binary(BinaryMerkleizer)` and `Merkleizer::Transition(BinaryMerkleizer)` variants + their `feed_updates` / `finalize` match arms. Add `Merkleizer::new_binary` / `new_bal_binary` constructors. (Medium)
- [ ] Task 4.14: Unit test: round-trip. Feed N updates into a `BinaryMerkleizer`, call `finalize`; separately feed the same updates into a `BinaryTrieState`, call `state_root()`. Assert bit-for-bit equality. Runs for: single-account, 10 accounts, 100 accounts, SELFDESTRUCT case, code deployment case. (Medium)
- [ ] Task 4.15: Unit test: sparse stem parity. For N stems with varying sub-index occupancies (1, 2, 5, 128, 256), assert the new sparse merkelize matches the Python reference vectors exactly. (Medium)
- [ ] Task 4.16: Micro-benchmark (in `benches/` or as a `#[test] #[ignore]`): 10k modified stems; measure total `finalize` time serial vs level-parallel. Record the ratio in the phase-4 handoff; target ≥ 3× speedup on an 8-core machine. Not a gating test but a correctness-of-parallelism sanity check. (Low)
- [ ] **Checkpoint: Verify Phase 4 complete** — `cargo build --workspace` passes; enum matches on `NodeUpdates`, `BackendKind`, `StateBackend`, `Merkleizer` all complete for `Binary` arm; `BinaryMerkleizer` public shape matches `MptMerkleizer` at the enum boundary; sparse stem merkelize matches Python reference vectors; level-parallel merkelize produces the same root as serial `state_root()`. (Low)

**Tests that MUST pass**
- `cargo build --workspace`
- `cargo test -p ethrex-binary-trie` — existing Phase 1-3 tests + new Phase 4 tests.
- `cargo test -p ethrex-storage` — no regressions.
- `cargo clippy -p ethrex-binary-trie --lib --no-deps -- -D warnings` clean.

**Known hazards**
- Adding `BackendKind::Binary` usage in `StateBackend` forces exhaustive-match updates across `store.rs`, `state_backend.rs`, `merkleizer.rs`, plus any L1/L2 code that inspects `BackendKind`. The compiler flags these; iterate `cargo check --workspace`.
- `cached_hash` interior mutability under `&self` for `merkelize_parallel`: picking the wrong primitive (e.g., `Mutex` per node) adds serious overhead. Recommend `AtomicCell<Option<[u8; 32]>>` or `parking_lot::Mutex`, or restructure to take `&mut self` and use `Rayon::join` instead of `par_iter`. The key constraint: no data race on `cached_hash` when two sibling hashes at the same level run concurrently (they don't share `cached_hash` slots since each hashes its own node).
- Sparse stem merkelize correctness: easy to get wrong if you don't handle the depth-8 binary reduction right. Test against Python reference vectors extensively.
- `fkv_entries` emission during `feed_updates`: ensure inline diff computation doesn't double-count. Test: sequence of `set → overwrite → delete` produces a single `Some(final_value)` (or `None` if final is delete) per key, not three entries.

**Handoff**: `BinaryMerkleizer` feeds updates, parallelizes merkelize across cores, emits `NodeUpdates::Binary { node_diffs, deleted_stems, fkv_entries }`. Phase 5 wires these diffs into disk commit + layer cache.

---

### Phase 5 — `binary_wiring.rs`: FKV, trie provider, disk commit, layer cache

**Entry criteria:** Phase 4 checkpoint passed.

**Design principle for this phase: full parity with `mpt_wiring.rs`.** Every production primitive MPT has must have a binary equivalent. No MPT-only shortcut is acceptable. Specifically:

| MPT primitive | Binary equivalent (must exist) |
|---|---|
| `mpt_wiring::flatkeyvalue_generator` (background thread that denormalizes MPT leaves into FKV; needed because snap sync dumps raw trie nodes) | **No equivalent**. Binary FKV is populated inline by `binary_commit_nodes_to_disk` in the same write transaction as the trie nodes. Justified because binary trie starts empty post-switch and grows only from commits — there is no "load snap-synced nodes and denormalize" scenario (snap sync stays MPT-only). |
| `TrieLayerCache` with bloom filter + 128-layer commit threshold | `BinaryTrieLayerCache` — same bloom filter, same commit threshold (`DB_COMMIT_THRESHOLD = 128`) |
| `trie_update_worker` | Same worker reused; selects `NodeUpdates` variant |
| `Store::fkv_ctl` | No new field. `apply_trie_updates` does not need to stop/resume a binary FKV generator because none exists. The MPT `fkv_ctl` remains and goes dormant after switch (MPT side is frozen; no new entries to flatten). |
| `TrieProvider` dep-inversion trait | `BinaryTrieProvider` (new, parallel) |
| `StoreTrieProvider` impl | `StoreBinaryTrieProvider` impl |
| `mpt_apply_prefix` cache keying | `binary_apply_prefix` keying (documented in `binary_wiring.rs`) |
| `mpt_wiring::build_mpt_cache_layer` | `binary_wiring::build_binary_cache_layer` |
| `mpt_wiring::write_mpt_node_updates_direct` | `binary_wiring::binary_commit_nodes_to_disk` |
| MPT snap sync helpers | Stubs only (snap sync stays MPT-only; documented) |
| MPT `eth_getProof` reader | Binary reader backing `eth_getBinaryProof` (Phase 8) |

The intent is that the shared-trie abstraction layer (`StateReader`/`StateCommitter`/`Merkleizer` enum, `TrieLayerCache::put_batch` taking `Vec<(Vec<u8>, Vec<u8>)>`) already supports both backends equally. This phase exercises that equality.

**Files created**
- `crates/storage/binary_wiring.rs` — all binary-specific `Store` extensions and helpers.

**Files modified**
- `crates/storage/api/tables.rs`:
  - Add `pub const BINARY_TRIE_NODES: &str = "BinaryTrieNodes"`, `pub const BINARY_FLATKEYVALUE: &str = "BinaryFlatKeyValue"`.
  - **Update the `pub const TABLES: [&str; 19]` array at `crates/storage/api/tables.rs:120`**: bump `19` → `21`, append `BINARY_TRIE_NODES` and `BINARY_FLATKEYVALUE`. This array is iterated by `InMemoryBackend` and `RocksDBBackend::open` to create column families — a missed entry means the tables are never created and writes fail silently.
  - Document format discriminator bytes `1`/`2` alongside `STATE_BACKEND_FORMAT_KEY`.
- `crates/storage/store.rs`:
  - Register the two new tables (via the TABLES bump above).
  - Add `binary_trie_cache: Arc<RwLock<Arc<BinaryTrieLayerCache>>>` field.
  - No new FKV control channel for binary (there's no binary FKV generator). `apply_trie_updates`, on the `NodeUpdates::Binary` arm, writes binary FKV entries inline via `binary_commit_nodes_to_disk` (see Task 5.5). Post-switch MPT FKV generator is quiesced (no new MPT writes arrive; `fkv_ctl` remains as a field but carries no traffic after switch).
  - In `from_backend`, no binary FKV generator is spawned. For `Transition`, the MPT FKV generator is left dormant. Both FKV tables remain readable for historical lookups.
  - Dispatch to `binary_wiring::binary_commit_nodes_to_disk` in `apply_trie_updates` and `write_node_updates_direct`.
- `crates/storage/lib.rs` — `pub mod binary_wiring;`.

**Tasks**
- [ ] Task 5.1: Add the two DB table constants. **Update `TABLES: [&str; 19]` → `[&str; 21]`** at `crates/storage/api/tables.rs:120`, appending both new table names. Verify both tables are created on both `InMemoryBackend` and `RocksDBBackend::open` with a smoke test. (Low)
- [ ] Task 5.2: Implement `StoreBinaryTrieProvider { store: Store }` that implements `BinaryTrieProvider` by reading `BINARY_TRIE_NODES` via `store.backend`. `load_node` reads by 8-byte node ID; `load_meta` reads `META_ROOT`/`META_NEXT_ID`/any `0xFF`-prefixed key; `is_deleted_stem` reads from a reserved meta range (keys `[0xFE, stem...]`). (Medium)
- [ ] Task 5.3: Implement `Store::make_binary_trie_provider() -> Arc<dyn BinaryTrieProvider>` factory. (Low)
- [ ] Task 5.4: Implement `Store::new_binary_state_reader(root) -> StateBackend` and `Store::new_binary_state_writer() -> StateBackend`. Both use `make_binary_trie_provider` and `make_code_reader`. (Medium)
- [ ] Task 5.5: Implement `binary_commit_nodes_to_disk(backend, node_diffs, deleted_stems, fkv_entries)`. This writes nodes, tombstones, AND inline FKV entries in a single atomic transaction:
  - For each `(key, value)` in `node_diffs`: if value empty, delete key in `BINARY_TRIE_NODES`; else put in `BINARY_TRIE_NODES`.
  - For each tombstoned stem: write `[0xFE, stem...]` key in `BINARY_TRIE_NODES` with value `[]` (presence-only marker at the persistence layer; the cache-layer sentinel is different — see Task 5.6).
  - For each `(tree_key, new_leaf_value)` in `fkv_entries` (derived from the stem's modified sub-indices — see Task 5.7 for how the merkleizer produces them): put in `BINARY_FLATKEYVALUE`. If `new_leaf_value` is `None` (deletion), delete the key in `BINARY_FLATKEYVALUE`. When a stem is fully tombstoned (SELFDESTRUCT), delete ALL existing `BINARY_FLATKEYVALUE` entries for every sub-index that was occupied on that stem (use the `storage_keys` side-index from `BinaryTrieState` to enumerate them).
  - All three writes go in one `begin_write` / `commit` block — partial commit must not be possible. If the batch fails, no writes land. (Medium)
- [ ] Task 5.6: Implement `build_binary_cache_layer(node_diffs, deleted_stems) -> Vec<(Vec<u8>, Vec<u8>)>` using the explicit framing from §3 / `hash.rs`:
  - Real node values: framed as `[CACHE_VALUE_TAG (0x00), ...node_bytes]`.
  - Tombstoned stem keys (`[0xFE, stem...]`): value is `[CACHE_TOMBSTONE_TAG (0x01)]` (single byte, unambiguous).
  - **Empty `Vec<u8>` is never used as a sentinel.** Decode must check the first byte; `0x01` length-1 = tombstone; `0x00`-prefixed longer = value with the prefix stripped; anything else = decode error.
  - Specify exact bytes in a unit test that round-trips both variants.
  (Medium)
- [ ] Task 5.7: Extend `BinaryMerkleizer::finalize` (Phase 4 Task 4.4) to produce `fkv_entries: Vec<([u8; 32], Option<[u8; 32]>)>` alongside `node_diffs` in its `MerkleOutput`. Exact format:
  - Key (32 bytes): `stem[0..31] || sub_index[0..1]` — the canonical 32-byte tree key for each modified leaf. Matches the key an `eth_getBinaryProof` RPC response references.
  - Value: `Some([u8; 32])` for an inserted/updated leaf (raw 32-byte value — packed `BASIC_DATA` for sub-index 0, raw `code_hash` for sub-index 1, raw U256 big-endian for storage leaves; no framing, no RLP); `None` for a per-leaf deletion.
  - For a SELFDESTRUCTed stem: emit one `None` entry per previously-occupied sub-index (looked up via the side-index; the merkleizer has access to it through `BinaryTrieState`).
  - Extend `NodeUpdates::Binary` to carry `fkv_entries` so `apply_trie_updates` can forward them to `binary_commit_nodes_to_disk` (Task 5.5). Update §5 of this plan and `docs/shared-trie/adding-a-backend.md` in Phase 9 to reflect the new field.
  - **Rationale**: MPT has a background FKV generator because snap sync dumps raw trie nodes and FKV must be rebuilt from them. Binary trie has no such scenario (snap sync stays MPT-only; binary trie starts empty post-switch and grows only from commits), so FKV is populated inline on every commit. This is NOT a deferral of the MPT-style generator — it is a different architecture appropriate to the data flow. (High)
- [ ] Task 5.8: Extend `crates/common/binary-trie/layer_cache.rs` (ported `BinaryTrieLayerCache`) as needed to expose `put_batch`, `get`, `get_commitable`, `commit` on the shape expected by `apply_trie_updates`. The `put_batch`/`get` decode layer MUST use the `CACHE_VALUE_TAG` / `CACHE_TOMBSTONE_TAG` framing introduced in Task 5.6. Empty values are an error, not a sentinel. (Medium)
- [ ] Task 5.9: Unit test: round-trip — construct a binary backend via `Store::new_binary_state_writer`, apply updates via `StateCommitter`, commit through the merkleizer path, reopen store, read through `Store::new_binary_state_reader`, confirm same values. (Medium)
- [ ] Task 5.10: Unit test: inline FKV write — apply a batch of updates that touches N stems; assert `BINARY_FLATKEYVALUE` has exactly the expected entries immediately after commit (no background thread; sync only); assert reads short-circuit via FKV rather than trie traversal. (Medium)
- [ ] Task 5.11: Unit test: SELFDESTRUCT FKV cleanup — seed a stem with 10 storage slots, all written to `BINARY_FLATKEYVALUE`; SELFDESTRUCT the account; assert all 10 FKV entries are deleted in the same commit, tombstone is present, and reads return zero (not the pre-SELFDESTRUCT values). (High)
- [ ] Task 5.12: Unit test: tombstone framing — write a tombstone, read back via `BinaryTrieLayerCache::get`, confirm a real empty value is rejected, confirm `[0x01]` decodes as tombstone, confirm `[0x00, ..bytes]` decodes as value. (Low)
- [ ] Task 5.13: Unit test: atomic commit — inject a DB write failure partway through a `binary_commit_nodes_to_disk` batch; assert that neither trie nodes nor FKV entries land (the batch is all-or-nothing). (Medium)
- [ ] **Checkpoint: Verify Phase 5 complete** — all wiring tests pass; `unimplemented!()` call sites from Phase 4 are replaced by real implementations; `TABLES` length matches the new count; `BINARY_FLATKEYVALUE` is populated inline and stays in sync with the trie across every commit. (Low)

**Tests that MUST pass**
- `cargo test -p ethrex-storage` (new binary wiring tests + MPT regression).
- Round-trip test covering insert → commit → reopen → read.
- Framing round-trip test.

**Known hazards**
- Key-space collisions: MPT uses nibble-path bytes; binary uses 8-byte node IDs. Since they live in different tables (`ACCOUNT_TRIE_NODES` vs. `BINARY_TRIE_NODES`) there is no collision, but **the tombstone prefix `0xFE` must be disjoint from both valid `NodeId` LE prefixes and meta-key `0xFF` prefix** — document in `node_store.rs`.
- Two layer caches live simultaneously in `Store` during `Transition` mode. Each is keyed by its own root hash; they never cross-read. Document this explicitly in `Store` struct rustdoc.
- `BinaryTrieLayerCache` is per-block; ensure the `parent` field is wired correctly on each feed.

**Handoff**: Binary backend is fully persistent and FKV-accelerated when used in isolation (`BackendKind::Binary`).

---

### Phase 6 — `TransitionBackend` + `transition_wiring.rs`

**Entry criteria:** Phase 5 checkpoint passed.

**Files created**
- `crates/storage/transition_wiring.rs` — transition-specific `Store` factories + activation primitives.

**Files modified**
- `crates/storage/state_backend.rs` — fill in real `StateBackend::Transition` match arms (see §5 for struct). Reads fall through base; writes hit overlay only after CoW prep.
- `crates/storage/store.rs` — `from_backend` for `BackendKind::Transition` reads the three transition meta keys and reconstructs `TransitionBackend` on startup.
- `crates/storage/api/tables.rs` — document three new `MISC_VALUES` keys: `TRANSITION_SWITCH_BLOCK_KEY = b"transition_switch_block"`, `TRANSITION_MPT_FROZEN_ROOT_KEY = b"transition_mpt_frozen_root"`, `TRANSITION_BINARY_ROOT_KEY = b"transition_binary_root"`.

**Tasks**
- [ ] Task 6.0: Extend `BinaryBackend` with the query methods `TransitionBackend` needs for CoW decisions. Add these inherent methods on `BinaryBackend` (not on `StateReader`, since MPT doesn't need them):
  - `pub fn stem_has_basic_data(&self, stem: &[u8; 31]) -> Result<bool, StateError>` — true iff overlay has a leaf at `(stem, BASIC_DATA_LEAF_KEY)` that is NOT a tombstone.
  - `pub fn stem_is_tombstoned(&self, stem: &[u8; 31]) -> Result<bool, StateError>` — true iff the tombstone side-table (`[0xFE, stem...]`) has an entry.
  - `pub fn insert_stem_group(&mut self, stem: &[u8; 31], leaves: &[(u8, [u8; 32])]) -> Result<(), StateError>` — atomic multi-leaf insert on a single stem, used for CoW-to-overlay. Reuses the ported `BinaryTrie::insert_multi` single-walk insertion. (Medium)
- [ ] Task 6.1: Implement `TransitionBackend { base: MptBackend, overlay: BinaryBackend, switch_block: u64, frozen_mpt_root: H256 }`. Constructor takes both backends by value. `base` is treated as read-only by convention: `TransitionBackend` never calls `StateCommitter` methods on `base`. The read-only aspect is enforced at the type-system level by not passing `&mut base` anywhere internally, plus a unit test (Task 6.11) that inspects `base`'s state after a series of TransitionBackend mutations and asserts nothing in base changed. No runtime `into_readonly()` wrapper is introduced. (Medium)
- [ ] Task 6.2: Implement `StateReader` for `TransitionBackend` with the stem-integrity invariant. Use `BinaryBackend::stem_is_tombstoned` / `stem_has_basic_data` from Task 6.0:
  - `account(addr)`: derive `stem = get_stem_for_base(addr)`. If `overlay.stem_is_tombstoned(&stem)?` → return `None` (no MPT fallthrough). Else if `overlay.stem_has_basic_data(&stem)?` → reconstruct `AccountInfo` from `overlay.account(addr)` (by invariant, `CODE_HASH` is also present). Else fall through to `base.account(addr)`. Increment metrics counters per path.
  - `storage(addr, slot)`: derive stem. If `overlay.storage(addr, slot)?` returns `Some(val)` → return `val`. Else if `overlay.stem_is_tombstoned(&stem)?` → return `H256::zero()`. Else fall through to `base.storage(addr, slot)`.
  - `code(addr, code_hash)`: single path — look up `code_reader` (which wraps legacy `AccountCodes`). Pre-switch codes are there from original execution; post-switch codes are dual-written there (Task 3.8). No chunk-reconstruction. (High)
- [ ] Task 6.3: Implement `StateCommitter` for `TransitionBackend` with **atomic stem-group CoW on first touch**. Use `BinaryBackend::insert_stem_group` from Task 6.0:
  - `update_accounts`: for each account being touched, derive `stem`. If `overlay.stem_is_tombstoned(&stem)?` → this is a post-selfdestruct recreation; clear the tombstone and treat as a fresh overlay account (no CoW from base). Else if `overlay.stem_has_basic_data(&stem)?` returns false → perform **CoW**: call `base.account(addr)?` to get the MPT `AccountInfo` (or zeros if absent from base too — this is a brand-new account post-switch); call `insert_stem_group(&stem, &[(BASIC_DATA_LEAF_KEY, packed), (CODE_HASH_LEAF_KEY, code_hash.0)])` to atomically write both leaves. Then apply the update. A selfdestruct writes the overlay tombstone side-table entry and skips CoW.
  - `update_storage`: delegate to overlay. No CoW needed for storage slots — overlay naturally shadows base; an unset slot in overlay with an unset tombstone falls through on read.
  - `clear_storage(addr)`: derive stem; write `[0xFE, stem...]` tombstone side-table entry in overlay so base's storage is hidden; clear overlay's own storage index for this address.
  - `hash() -> Result<H256, StateError>`: propagate `overlay.hash()`'s `Result` — do not drop it. We do NOT compose base + overlay into a single root; state-root validation is disabled post-switch.
  - `commit() -> Result<MerkleOutput, StateError>`: commit overlay; return its `MerkleOutput`. (High)
- [ ] Task 6.4: Add `Store::new_transition_state_reader(switch_block, mpt_root, binary_root) -> StateBackend` factory in `transition_wiring.rs`. Internally opens a read-only `MptBackend` at `mpt_root` and a `BinaryBackend` at `binary_root`. (Medium)
- [ ] Task 6.5: Add `Store::persist_transition_metadata(switch_block, mpt_root, binary_root)` that writes the three `MISC_VALUES` keys AND updates `STATE_BACKEND_FORMAT_KEY` to byte `2` in a single write transaction. (Medium)
- [ ] Task 6.6: Add `Store::load_transition_metadata() -> Result<Option<(u64, H256, H256)>>`. (Low)
- [ ] Task 6.7: Update `Store::from_backend` — when on-disk format byte is `2`, load transition metadata, construct `TransitionBackend`, wire through `StateBackend::Transition`. Fail fast if any of the three meta keys is missing. No binary FKV generator is spawned (binary FKV is populated inline per Phase 5); MPT FKV generator is left dormant. (Medium)
- [ ] Task 6.8: Update `Merkleizer::new_transition(parent_root, provider)` — wraps `BinaryMerkleizer`; MPT side never takes writes during block exec. (Medium)
- [ ] Task 6.9: Update `apply_trie_updates` for `NodeUpdates::Binary` to work whether backend is `Binary` or `Transition` (same path). (Low)
- [ ] Task 6.10: Unit test: CoW invariant — pre-touch account A exists in MPT with `nonce=5, balance=10, code_size=0, code_hash=K`. Post-switch, call `update_accounts` with only a balance change. Assert that `overlay` stem for A now contains all four fields pulled from MPT plus the updated balance, and `CODE_HASH=K` is preserved (not zero). (High)
- [ ] Task 6.11: Unit test: round-trip read/write on `TransitionBackend` — pre-write reads from base, post-write reads from overlay, tombstoned stems hide base; re-creation after selfdestruct clears tombstone. Also: read-only-base invariant — after executing a series of `update_accounts` / `update_storage` / `clear_storage` calls, inspect `base`'s trie root and flat-state tables and assert none changed relative to the constructor-time snapshot. (High)
- [ ] Task 6.12: Unit test: restart — persist metadata, recreate `Store`, confirm `TransitionBackend` is reconstructed and reads match. (Medium)
- [ ] **Checkpoint: Verify Phase 6 complete** — transition mode can be persisted and restored; reads and writes behave per spec; CoW invariant test passes; `hash()` propagates `Result` through both Transition and Binary paths. (Low)

**Tests that MUST pass**
- Transition round-trip test.
- Restart-reconstruction test.
- CoW invariant test.
- `cargo test -p ethrex-storage`.

**Known hazards**
- **Tombstone cascade**: If account A is selfdestructed post-switch, overlay tombstone hides base. If A is later re-created, overlay must clear the tombstone on the new insert. Add explicit test for this.
- **Frozen MPT root**: We must never write into `base.storage_tries` during transition. The read-only invariant is enforced by not exposing `StateCommitter` on `&base` — confirm `MptBackend::new_with_db` can be used without accidentally mutating it via `StoreVmDatabase`.
- **Storage cache poisoning**: `MptBackend::storage_root_cache` is a `Mutex`; reads populate it. That's fine because the MPT is frozen; the cache just accelerates repeated reads.
- **CoW latency**: the first post-switch write to an MPT-resident account pays one MPT read. Amortized cost is bounded (once per account ever written); measure in Phase 10.

**Handoff**: `TransitionBackend` is feature-complete in isolation. All that remains is the activation trigger + CLI.

---

### Phase 7 — CLI flag + fully-automatic activation + restart-required transition

**Entry criteria:** Phase 6 checkpoint passed.

**Activation model: two-precondition automatic trigger.** When `--binary-transition` is set, activation fires as soon as **both** of these are simultaneously true:

1. `snap_enabled` has flipped false (snap sync has completed).
2. Follower is caught up — current head matches or exceeds the CL-reported finalized block (i.e., full-sync catch-up has finished).

There is **no operator RPC method, no signal, no env var, no manual trigger.** Observer ticks after each block commit; first block where both preconditions hold is the switch block. This matches the user's original spec ("snap sync done + N finalized blocks") and keeps the operator surface minimal: flag on at startup → node transitions itself once it's ready.

**Files created**
- `crates/blockchain/transition_activator.rs` — observer that polls snap_enabled + caught_up after each committed block and fires once both are true.

**Files modified**
- `cmd/ethrex/cli.rs` — add `--binary-transition` `ArgAction::SetTrue` flag to the main options struct with the help text "Enable opt-in MPT→binary trie transition (research preview; L1 follower only). Activates automatically once snap sync completes and the follower catches up to finalized head."
- `cmd/ethrex/initializers.rs` — pass the flag into the blockchain/sync setup and instantiate `TransitionActivator` when enabled.
- `crates/networking/p2p/sync_manager.rs` — expose `snap_enabled.clone()` and a `caught_up: Arc<AtomicBool>` set true when the follower reaches `head >= finalized` for at least one iteration.
- `crates/blockchain/blockchain.rs` — no functional change; add a getter `Blockchain::store(&self) -> &Store` if not already present, required by the activator to call `persist_transition_metadata`. Add `activation_lock: Arc<Mutex<()>>` field (or introduce a `TransitionCoordinator` struct that owns it).
- `crates/storage/store.rs` — add a `Store::activation_lock() -> Arc<Mutex<()>>` accessor so both `execute_block_pipeline` and the activator share the same mutex handle.

**Tasks**
- [ ] Task 7.1: Add `--binary-transition` flag in `cli.rs`; plumb through to the initializer. (Low)
- [ ] Task 7.2: Add `Arc<AtomicBool>` `caught_up` field to `SyncManager` (`crates/networking/p2p/sync_manager.rs`). This field does **not exist today** — it is new infrastructure introduced by this plan. Derivation path:
  - `SyncManager` already holds `last_fcu_head: Arc<Mutex<H256>>`, updated each time `engine_forkchoiceUpdated` is received.
  - Extend the FCU path (`crates/networking/rpc/engine/forkchoice.rs` — confirm exact filename at implementation time) so each FCU also records the **finalized block hash and number** from the FCU message payload. Store as a new field `last_fcu_finalized: Arc<Mutex<(H256, u64)>>` on `SyncManager`.
  - After each successful block commit in `blockchain.rs`, compare the freshly-committed block number against `last_fcu_finalized.1`. If `committed_number >= finalized_number`, set `caught_up.store(true, Ordering::Release)`; otherwise no-op (never set it back to false based on this check — it's a one-shot latch).
  - Expose `SyncManager::is_caught_up(&self) -> bool` that does `self.caught_up.load(Ordering::Acquire)`. The activator reads it via this getter.
  - Once latched true, `caught_up` stays true for the process lifetime. It does not un-latch on reorgs. Activation is gated on it being true once — the precondition "has been caught up at least once" is what we want; transient rewinds don't unqualify us.
  (Medium)
- [ ] Task 7.3: Implement `TransitionActivator::tick(store, head_block) -> TickResult` — runs after each successful block commit; returns `TickResult::Skip` unless both preconditions hold; returns `TickResult::Activate(head_block_number)` when they do. Uses `AtomicBool::load(Ordering::Acquire)`. Idempotent: after activation fires once, subsequent ticks return `Skip` because the format byte is already `2`. (Medium)
- [ ] Task 7.4: Implement `TransitionActivator::activate(store, head_block)` with **restart-required, not hot-swap** semantics. Steps **executed in order**:
  0. **Acquire `activation_lock: Mutex<()>`.** `execute_block_pipeline` must also acquire this lock at entry and block until release. This prevents any block from being applied concurrently with the metadata write.
  1. Re-verify both preconditions inside the lock (they may have flipped during the brief lock contention; if either is now false, release and retry on next tick).
  2. Send `FKVGeneratorControlMessage::Stop` to the MPT FKV generator and wait for ack.
  3. Force-flush `TrieLayerCache` — call `Store::force_commit_layers()` (add this method) that drains all layers to disk.
  4. Wait for `trie_update_worker` queue to drain.
  5. Read `frozen_mpt_root` = `head_block.header.state_root`. This is the MPT root **after executing `head_block`**, equivalently "the post-state of the last pre-transition block" — identical framing. The switch block is defined as `head_block.number + 1`: the first block whose execution writes go to the binary overlay. All `rpc.md` / operational descriptions MUST use this definition consistently.
  6. Use `EMPTY_BINARY_ROOT` (the constant from `hash.rs`, equal to `H256([0u8; 32])`) as the initial `binary_root`. This value is written to `TRANSITION_BINARY_ROOT_KEY`.
  7. Call `store.persist_transition_metadata(head_block.number + 1, frozen_mpt_root, EMPTY_BINARY_ROOT)`. `transition_switch_block = head_block.number + 1` (first binary block). This writes the three `MISC_VALUES` keys AND flips `STATE_BACKEND_FORMAT_KEY` to byte `2` atomically in a single DB transaction.
  8. Log a clear message: `"Binary trie transition activated at block N. Frozen MPT root: 0x... Restart the process with --binary-transition to enter transition mode."`
  9. Emit a graceful shutdown signal (via the existing shutdown channel) and return. Release the activation lock on drop. The process exits with code 0. **No in-process swap of the `StateBackend` occurs.**
  10. On restart (already handled in Task 6.7): `Store::from_backend` reads format byte `2` + transition meta keys and constructs `TransitionBackend`. The operator must re-supply `--binary-transition` to avoid a config-mismatch error; the restart path refuses to start in MPT mode when format byte is `2`. (High)
- [ ] Task 7.5: Integration test `binary_transition_auto_activation`: start an in-memory `Store` with `--binary-transition` simulated; seed `snap_enabled=true` then flip false; seed `caught_up=false` then flip true; drive the activator's tick; assert metadata is persisted and the shutdown signal fires. (Medium)
- [ ] Task 7.6: Integration test `binary_transition_restart_cycle`: after the auto-activation test above, reopen the same `Store`, assert backend kind is `Transition` and reads behave overlay→base. (Medium)
- [ ] Task 7.7: Integration test `binary_transition_waits_for_caught_up`: activator with `snap_enabled=false, caught_up=false` must not fire; flipping `caught_up=true` triggers activation on the next tick. Ensures we never activate mid-catch-up. (Medium)
- [ ] Task 7.8: When `--binary-transition` is NOT set, zero binary-trie code paths run. Confirm via a smoke test that asserts `TransitionActivator` is never constructed. (Low)
- [ ] Task 7.9: Integration test `binary_transition_locked_without_flag`: open a `Store` with `STATE_BACKEND_FORMAT_KEY = 2` on disk and `--binary-transition` absent. Assert `Store::from_backend` returns `StoreError::Custom` with a message containing "format byte 2 (transition) but --binary-transition was not passed" or equivalent. Assert the process does not proceed to block execution. (Medium)
- [ ] **Checkpoint: Verify Phase 7 complete** — all four integration tests pass against an in-memory `Store`. Devnet smoke test is **manual** and documented in `operational.md`, not a PR gate. (Low)

**Tests that MUST pass**
- `cargo test binary_transition_auto_activation` (in-memory `Store`).
- `cargo test binary_transition_restart_cycle`.
- `cargo test binary_transition_waits_for_caught_up`.
- `cargo test binary_transition_locked_without_flag`.

**Known hazards**
- **Force-flush ordering**: layer cache must drain BEFORE we read `frozen_mpt_root`; otherwise the MPT-on-disk differs from the header.
- **Concurrent writes during activation**: handled by `activation_lock` (Step 0 of Task 7.4). `execute_block_pipeline` acquires the same lock; activation holds it for the entire metadata-write window.
- **`caught_up` is a one-shot latch**: once true, never un-latches for the process lifetime. A reorg that temporarily rewinds the head does NOT clear it. This is intentional — the latch says "we have been caught up at least once", which is the precondition we want.
- **Deep-reorg during activation window**: if a deep reorg is in progress at the moment the activator acquires the lock, the MPT state may transiently reflect a non-canonical root. Activating the force-flush (Step 3) before reading `frozen_mpt_root` reduces but does not eliminate this race. Operators should confirm finality depth on the frozen root post-activation before treating the transition as authoritative. Mitigation is acceptable because (a) reorgs deeper than finality are Byzantine events on post-merge mainnet and (b) the frozen root is logged at activation and can be compared externally.
- **No hot-swap means** the activator cannot return the operator to a running node. The process exits; operator relaunches the same command to resume in transition mode. Document prominently in `operational.md`.
- **Partial write on crash**: if the process dies between step 7 and step 9, the format byte is already `2` and the restart path works normally. If it dies before step 7, on restart the format byte is still `0` and the observer re-fires on next startup.
- **Format byte `2` with flag absent**: `Store::from_backend` must detect this and return a `StoreError::Custom` explaining the mismatch. Test covered by Task 7.11.

**Handoff**: A node launched with `--binary-transition` transitions itself as soon as it's ready. No operator action required between startup and restart.

---

### Phase 8 — RPC + metrics

**Entry criteria:** Phase 7 checkpoint passed.

**Files created**
- `crates/networking/rpc/eth/binary_proof.rs` — `GetBinaryProofRequest` handler.
- `crates/blockchain/metrics/binary_trie.rs` — metric definitions.

**Files modified**
- `crates/networking/rpc/eth/account.rs` — `GetProofRequest::call` branches on `store.backend_kind()`:
  - `Mpt` → current behavior.
  - `Binary` / `Transition` → return JSON-RPC error `{code: -32099, message: "state moved to binary trie at block {switch_block}; use eth_getBinaryProof"}`.
- `crates/networking/rpc/rpc.rs` — register `"eth_getBinaryProof"`.
- `crates/networking/rpc/eth/mod.rs` — export `binary_proof`.
- `crates/blockchain/metrics/mod.rs` — register binary_trie metrics module.

**Tasks**
- [ ] Task 8.1: Define the `eth_getBinaryProof` response shape in `binary_proof.rs`. **The shape below is normative** and must match the wire-format example in `docs/binary-trie/rpc.md` exactly (serde field names, nullability, nesting). If they diverge during implementation, update `rpc.md` and this task together in the same commit.
  ```rust
  #[derive(Serialize, Deserialize)]
  pub struct BinaryAccountProofResponse {
      pub address: Address,
      pub binary: Option<BinaryProof>,        // None when account absent from overlay
      pub fallback_mpt: Option<MptAccountProof>, // present when stem is absent from overlay; also populated for pre-switch blocks
      pub overlay_root: H256,
      pub frozen_mpt_root: H256,
  }
  #[derive(Serialize, Deserialize)]
  pub struct BinaryProof {
      pub stem: Bytes,                         // 31-byte stem
      pub basic_data: Bytes,                   // packed 32-byte BASIC_DATA leaf
      pub code_hash: H256,
      pub stem_siblings: Vec<H256>,
      pub stem_depth: u32,
      pub stem_node_hash: Option<H256>,        // Some for different-stem absence proofs, None otherwise
      pub storage: Vec<BinaryStorageProof>,
  }
  #[derive(Serialize, Deserialize)]
  pub struct BinaryStorageProof {
      pub key: H256,
      pub sub_index: u8,
      pub value: H256,
      pub subtree_siblings: [H256; 8],
  }
  ```
- [ ] Task 8.2: Implement `GetBinaryProofRequest::call`:
  1. Resolve block → state root.
  2. **If `block.number < transition_switch_block`**: `binary = None`; `fallback_mpt = Some(...)` populated from MPT for the requested block's state root.
  3. Else open `StateBackend` and match on arm:
     - `Binary`: call `BinaryBackend::get_account_proof(addr, keys)`; `fallback_mpt = None`; `binary = Some(...)`.
     - `Transition`: try overlay first. If stem is present in overlay, `binary = Some(...)`, `fallback_mpt = None`. If stem is absent from overlay, `binary = None`, `fallback_mpt = Some(...)` from `base` (MPT proof against `frozen_mpt_root`).
     - `Mpt`: return JSON-RPC error (see Task 8.4's companion in §8.4a). (Medium)
- [ ] Task 8.3: Implement `BinaryBackend::get_account_proof(addr, keys) -> BinaryProof` using `BinaryTrie::get_proof` (already in ported `proof.rs`). (Medium)
- [ ] Task 8.4: Modify `GetProofRequest::call` to return structured error `{code: -32099, message: "state moved to binary trie at block X, use eth_getBinaryProof", data: {switch_block, frozen_mpt_root}}` when `store.backend_kind()` is `Binary` or `Transition`. Add `Store::backend_kind(&self) -> BackendKind` accessor. Define the error code as a named constant `RPC_ERROR_BINARY_MODE: i64 = -32099` in the same module. (Low)
- [ ] Task 8.4a: Register error code `RPC_ERROR_MPT_MODE: i64 = -32098` for the inverse case — caller invoked `eth_getBinaryProof` on a pure-MPT node. Message: `"binary trie not active on this node; use eth_getProof"`. Wire into Task 8.2's `Mpt` arm. (Low)
- [ ] Task 8.5: Register `"eth_getBinaryProof" => GetBinaryProofRequest::call(req, context)` in `rpc.rs`. (Low)
- [ ] Task 8.6: Define metrics in `binary_trie.rs`. The set below MUST match the metrics listed in `operational.md`; any addition in one doc requires the other to be updated in the same commit.
  - `binary_trie_overlay_read_hit` (Counter)
  - `binary_trie_mpt_fallback_read_hit` (Counter)
  - `binary_trie_overlay_read_miss` (Counter)
  - `binary_trie_code_chunks_written` (Counter)
  - `binary_trie_stem_count` (Gauge)
  - `binary_trie_tombstone_count` (Gauge) — number of active tombstones (SELFDESTRUCTed MPT-resident accounts)
  - `binary_trie_switch_activation_timestamp` (Gauge, seconds since epoch).
  Use the existing `ethrex_metrics::metrics!` macro pattern. (Medium)
- [ ] Task 8.7: Instrument `TransitionBackend::account/storage` and `BinaryBackend::update_accounts` with counter increments. Instrument `TransitionActivator::activate` with the timestamp gauge. (Medium)
- [ ] Task 8.8: RPC integration test: spin up in-memory store with transition metadata pre-populated, call `eth_getBinaryProof`, verify JSON shape. (Medium)
- [ ] Task 8.9: RPC integration test: call `eth_getBinaryProof` for a block number < `switch_block`; assert binary proofs are placeholders and `fallback_mpt_proof` is populated. (Medium)
- [ ] Task 8.10: RPC integration test: `eth_getProof` returns structured error in binary/transition mode. (Low)
- [ ] **Checkpoint: Verify Phase 8 complete** — RPC + metrics end-to-end. (Low)

**Tests that MUST pass**
- RPC integration tests for both methods in each of the three backend modes + pre-switch historical block.

**Known hazards**
- The JSON-RPC error code `-32099` is in the implementation-defined range `[-32099, -32000]`. Confirm no existing ethrex error handler claims it.
- Metric labels / units match existing naming (`snake_case_with_dots` vs. `_`): inspect `crates/blockchain/metrics/blocks.rs` and mirror the pattern.

**Handoff**: Operators can query binary proofs and monitor the transition via Prometheus.

---

### Phase 9 — Integration tests + documentation

**Entry criteria:** Phase 8 checkpoint passed.

**Files created**
- `test/tests/binary_transition.rs` — end-to-end scripted sequences.

**Files authored (already stub-present on this branch; fill to final form and keep synchronized with implementation)**
- `docs/binary-trie/plan.md` (this document)
- `docs/binary-trie/overview.md`
- `docs/binary-trie/design-decisions.md`
- `docs/binary-trie/rpc.md`
- `docs/binary-trie/testing.md`
- `docs/binary-trie/operational.md`

**Files modified**
- `docs/shared-trie/adding-a-backend.md` — update two claims:
  1. The FKV-tables-are-shared claim in the "shared formats" / §5 section — replace with: "Each backend may define its own FKV tables. Binary overrides this and uses `BINARY_FLATKEYVALUE`. Shared FKV is optional; the format discriminator byte in `STATE_BACKEND_FORMAT_KEY` selects the backend at open time."
  2. The `NodeUpdates::Binary` definition — update from `Binary { node_diffs: Vec<(Vec<u8>, Vec<u8>)> }` to `Binary { node_diffs, deleted_stems: Vec<[u8; 31]>, fkv_entries: Vec<([u8; 32], Option<[u8; 32]>)> }`. Comment: `deleted_stems` carries SELFDESTRUCT-originated stem tombstones separately from the trie node diff stream because they persist at a reserved `[0xFE, stem...]` key range outside the node namespace; `fkv_entries` carries inline flat-state updates because binary has no background FKV generator (unlike MPT).

**Tasks**
- [ ] Task 9.1: Integration test A — overlay read semantics: write keys through `BinaryBackend`, confirm reads on `TransitionBackend` see overlay; reads for untouched accounts see base. (Medium)
- [ ] Task 9.2: Integration test B — overlay write semantics: every `update_accounts` call on `TransitionBackend` produces a diff only in `BINARY_TRIE_NODES`, never in `ACCOUNT_TRIE_NODES`. Assert by sampling both tables. (Medium)
- [ ] Task 9.3: Integration test C — SELFDESTRUCT tombstone: selfdestruct an account that existed pre-switch; read returns `None`; re-create account; read returns new info. (Medium)
- [ ] Task 9.4: Integration test D — code resolution: deploy contract before activation → read via `AccountCodes`; deploy post-activation → dual-written to `AccountCodes` → read succeeds without reading chunks. Assert EXTCODECOPY works on post-switch-deployed contracts. (Medium)
- [ ] Task 9.5: Integration test E — layer cache + restart: apply 10 blocks post-switch with layer cache enabled, close store, reopen, confirm all 10 blocks' writes are readable (committed to disk). (Medium)
- [ ] Task 9.6: Integration test F — switch metadata persistence: persist metadata, reopen, confirm `StateBackend::Transition` and the three meta values round-trip. (Low)
- [ ] Task 9.8: Update `docs/shared-trie/adding-a-backend.md` FKV-shared claim and `NodeUpdates::Binary` definition as described above. (Low)
- [ ] Task 9.9: Author `overview.md`: architecture diagram (ASCII), dep graph, lifecycle (sync → activate → restart → transition-mode follow). (Medium)
- [ ] Task 9.10: Author `design-decisions.md`: overlay vs. migration; BLAKE3; sparse stems; no state-root validation; tombstone strategy (both trie-layer `0xFE` prefix and cache-layer `0x00/0x01` framing); activation-requires-restart; CoW stem-group invariant; dual-write code resolution. (Medium)
- [ ] Task 9.11: Author `rpc.md`: full `eth_getBinaryProof` spec with `curl` example + expected JSON; pre-switch block behavior (empty binary proofs + populated `fallback_mpt_proof`); error codes table for `eth_getProof` in non-MPT mode. (Medium)
- [ ] Task 9.12: Author `testing.md`: how to regenerate vectors (`python3 testgen/generate_vectors.py`); how to run each integration test; CI integration notes. (Low)
- [ ] Task 9.13: Author `operational.md`: step-by-step activation runbook including restart requirement; failure modes (activation aborts, reorg, disk full during force-flush); monitoring (which metrics to alert on); known limitations (no snap sync, no zkVM, no block proposal, no rollback, manual devnet smoke test). (Medium)
- [ ] **Checkpoint: Verify Phase 9 complete** — all 7 integration tests pass; 5 docs exist and are cross-linked; `adding-a-backend.md` updated. (Low)

**Tests that MUST pass**
- `cargo test -p ethrex-blockchain binary_transition` (all 7 scenarios).

**Known hazards**
- Integration test G requires process-death semantics; use a subprocess wrapper or `std::panic::catch_unwind` with a well-defined panic message.
- Docs in `docs/binary-trie/` must cross-link to `docs/shared-trie/` for readers who haven't seen the abstraction layer.

**Handoff**: A reader with only `docs/binary-trie/` and `docs/shared-trie/` can understand, operate, and extend the binary trie backend.

---

### Phase 10 — Polish pass

**Entry criteria:** Phase 9 checkpoint passed.

**Tasks**
- [ ] Task 10.1: Run `cargo fmt --all`. Ensure `make lint` passes with zero warnings on the new crate and all modified files. (Low)
- [ ] Task 10.2: Run `cargo clippy --all-features --workspace` and resolve every lint in new files. For the ported crate, add `#![allow]` only for lints that are unavoidable in the reference code; document each. (Medium)
- [ ] Task 10.3: Run `make test`; confirm no regressions. (Low)
- [ ] Task 10.4: Run `make -C tooling/ef_tests/blockchain test` (MPT default only — binary path is behind flag). Verify zero regressions. Save output to `/tmp/ef_tests_blockchain.log`. (Medium)
- [ ] Task 10.5: Run `make -C tooling/ef_tests/state test`. Verify zero regressions. (Medium)
- [ ] Task 10.6: Microbenchmark the `BinaryMerkleizer::finalize` path on a 10k-update block using the 16-shard rayon architecture (§3 mandates this; single-threaded is not a permitted fallback). Target: ≤10× MPT merkleization time. Microbenchmark the CoW first-touch cost (time to CoW 1000 accounts from a realistic MPT). If slower than target, file an issue with the measured number and a proposed optimization; do not ship without an explicit decision from the owner on whether to block on optimization or accept the number. (Medium)
- [ ] Task 10.7: Cross-link every `docs/binary-trie/` file from `docs/shared-trie/adding-a-backend.md` "Transition backend" section. (Low)
- [ ] Task 10.8: Add a `CHANGELOG.md` entry under next version: "feat(l1): add EIP-7864 binary trie backend (opt-in, research preview)". (Low)
- [ ] Task 10.9: Per §2a rule 2, **zero** `unimplemented!()`, `todo!()`, `TODO`, or `FIXME` may remain in code introduced by this PR. Grep the diff; if any are found, resolve them. "Justify and keep" is not a permitted outcome. Pre-existing occurrences in the rest of the codebase are unaffected. (Medium)
- [ ] **Final Audit** — Re-read the entire plan. For each task 1.1 through 10.9, verify the implementation exists in the codebase (use `git diff main...shared-trie-binary` and grep). List any gaps. All gaps must be resolved before reporting completion. (High)

**Tests that MUST pass**
- `make lint`
- `make test`
- `make -C tooling/ef_tests/blockchain test`
- `make -C tooling/ef_tests/state test`

**Known hazards**
- EF tests on an `InMemory` store are fast; on RocksDB slow. Use RocksDB only for the final verification, not every local iteration.

**Handoff**: PR-ready branch.

## 7. Exit Criteria (for the whole PR)

- [ ] `ethrex-binary-trie` crate exists and passes its own tests (ported + extended vectors + merkleizer round-trip).
- [ ] `EMPTY_BINARY_ROOT` constant defined in `binary-trie/src/hash.rs` and used consistently.
- [ ] `BackendKind::{Binary, Transition}` exist; byte mapping `0/1/2` persisted.
- [ ] `NodeUpdates::Binary` includes `deleted_stems: Vec<[u8; 31]>` and `fkv_entries: Vec<([u8; 32], Option<[u8; 32]>)>`.
- [ ] `StateBackend::{Binary, Transition}`, `Merkleizer::{Binary, Transition}` all dispatch correctly; every match (including `write_node_updates_direct`) is exhaustive.
- [ ] `StateCommitter::hash` returns `Result<H256, StateError>` through all backend impls and wrappers; no signature drops the `Result`.
- [ ] `TABLES: [&str; N]` array updated to include `BINARY_TRIE_NODES` and `BINARY_FLATKEYVALUE`; both tables created on open on both `InMemoryBackend` and `RocksDBBackend`.
- [ ] Overlay stem integrity invariant holds: reads never return partial `AccountInfo`; write path CoWs missing sub-leaves on first touch.
- [ ] Tombstone framing uses `[0x00, ...value]` / `[0x01]` sentinel (never empty `Vec<u8>`) in the cache layer, and `[0xFE, stem...]` in `BINARY_TRIE_NODES`.
- [ ] Post-switch code is dual-written to `AccountCodes`; all code reads go through `code_hash → AccountCodes`.
- [ ] `--binary-transition` CLI flag gates activation; without it the binary has no active binary-trie code path.
- [ ] Activation is one-way, **fully automatic** (fires when `snap_enabled==false && caught_up==true` both hold), acquires `activation_lock` to pause block execution, and **requires process restart**; restart path reconstructs `TransitionBackend` from persisted metadata.
- [ ] `eth_getBinaryProof` works (including the pre-switch-block historical case); `eth_getProof` returns `-32099` in non-MPT modes.
- [ ] Reads on `TransitionBackend` follow overlay→base; writes go to overlay only; tombstones hide base on SELFDESTRUCT.
- [ ] Metrics emit per-operation counters and an activation timestamp gauge.
- [ ] `docs/shared-trie/adding-a-backend.md` updated (FKV shared claim + `NodeUpdates::Binary` shape).
- [ ] 7 integration tests + the transition unit test (`binary_transition_restart_cycle`) pass.
- [ ] 5 docs exist and are cross-linked.
- [ ] `make lint`, `make test`, both EF suites pass with zero regressions.

## 8. Known Risks + Mitigations

| Risk | Mitigation |
|---|---|
| Reference branch state.rs semantics drift from EIP | Regenerate vectors against spec Python; lock via CI |
| Tombstone key collision with NodeId | Reserve `0xFE` prefix in `BINARY_TRIE_NODES`; documented in `node_store.rs`; unit test guards it |
| Tombstone cache-layer false negatives (empty-value collision) | Explicit framing `[0x00, ...]` / `[0x01]` sentinel; empty `Vec<u8>` is a decode error; Phase 5 round-trip test |
| Partial-stem read hazard | Overlay stem integrity invariant: atomic-group writes + CoW on first touch. Phase 6 Task 6.10 test enforces |
| Force-flush hangs under load | Add 30s timeout; on timeout, abort activation and log. Activator will retry on the next block commit (preconditions will still be true). |
| Activation during active block pipeline | `activation_lock: Mutex<()>` shared between activator and `execute_block_pipeline`; restart-required model eliminates hot-swap complexity entirely |
| Reorg deeper than 128 blocks | Already fatal on any non-archive ethrex node via the layer cache depth limit; inherited, not new. Documented in `operational.md`. No explicit switch-block-crossing detection added. |
| BLAKE3 dep version conflict | Pin to workspace version; CI check via `cargo tree` |
| RPC `-32099` code collision | Grep `rpc.rs` + `error.rs` for existing use of -32099 before claiming it |
| Layer cache double-lock (MPT + binary both present in Transition) | Acquire in fixed order (MPT-first) in all code paths; document invariant |
| Binary FKV falls out of sync with trie | Commits are atomic (`binary_commit_nodes_to_disk` writes trie nodes + tombstones + FKV entries in one DB transaction). Task 5.13 unit test verifies atomicity under injected failure. |
| Missing TABLES entry → silent write failure | Phase 5 Task 5.1 explicitly bumps the array length and appends both tables; checkpoint verifies |
| Witness disabling breaks L1 state-sync RPC | Audit callers of `init_witness` — if any are on the hot path for MPT, Transition mode must not disable them globally. Review before Phase 8 |
| Post-switch deploy returns `None` on EXTCODECOPY | Dual-write to `AccountCodes`; Phase 9 test D asserts |
| CoW cost spike on hot account | Measured in Phase 10 Task 10.6; one-time per account; acceptable for research preview |

## 9. Open Questions Carried Forward

1. **Snap sync post-switch.** Plan leaves snap unchanged (MPT-only). That means a fresh node cannot snap-sync the binary era; it must sync MPT historical + binary via full sync. Acceptable for follower-only research, but call out in `operational.md`. No blocker for implementation; user-facing note only.


## 9a. Resolved Questions (previously carried forward)

- **Streaming API (`feed_updates` / `finalize`).** Resolved: streaming at the public boundary so `execute_block_pipeline` dispatches identically to MPT. See §3 decision "BinaryMerkleizer API surface" and Phase 4 tasks.
- **Parallel merkleization shape.** Resolved after an initial 16-shard attempt was rejected: **single-tree, level-parallel via `rayon::par_iter` + sparse StemNode hashing**. Rationale: binary trie's 2-way root doesn't support clean MPT-style sharding (a 16-shard split puts each shard at depth 4 with a shared skeletal spine above), and the dominant cost is BLAKE3 on the dirty frontier which level-parallelism targets directly. See §3 and `design-decisions.md` §13.
- **Storage-layer parity with MPT.** Resolved: binary backend provides `BinaryTrieLayerCache` (bloom filter, 128-block commit threshold), FKV table, trie-update worker dispatch. Merkleization does NOT shard; that deviation is scoped to the merkleizer only. See Phase 5 and §3 "Per-backend storage primitives".
- **Activation model: hot-swap vs. restart.** Resolved: restart-required. See §3 and Phase 7 Task 7.4.
- **Activation trigger: manual vs. automatic.** Resolved: **fully automatic**. `snap_enabled=false` AND `caught_up=true` → activation fires on the next block commit. No admin RPC method, no env var, no signal. User's original spec was "snap sync done + N finalized blocks"; replacing "N finalized blocks" with the stronger "caught up to finalized head" precondition. An earlier draft introduced an "operator trigger" as a third condition; that was a drift from the spec and has been removed.
- **Reorg detection.** Resolved: **no explicit detection added.** Rationale: (1) activation fires only after `caught_up` is true, which means head has been at or beyond the CL-reported finalized head; switch_block is therefore past or at finality. (2) A reorg past finality is a Byzantine consensus failure, not a normal operational event — no mainnet post-merge reorg has exceeded 7 blocks. (3) A reorg deeper than 128 blocks is already fatal on any non-archive ethrex node via the MPT layer cache depth limit. Since "reorg crosses switch block" requires reorging past finality, which requires a reorg deeper than finality (~64 blocks), and any reorg deeper than 128 is already fatal, switch-crossing reorgs are a strict subset of an already-fatal class. Explicit detection would add code and tests for a failure mode that's already handled.
- **Metrics registry.** Resolved: binary-trie metrics register under `ethrex-blockchain`'s existing `metrics` feature flag. No new crate-level feature. Phase 8 Task 8.6 uses the existing `ethrex_metrics::metrics!` macro pattern. See Phase 8.
- **Defense-in-depth `MptBackend::into_readonly()` wrapper.** Resolved: **skipped**. Rationale: (1) `TransitionBackend` holds `base: MptBackend` by value; mutation requires a `&mut self` call on it, and nothing in `TransitionBackend`'s `StateCommitter` impl does so — all writes route through `self.overlay`. (2) Task 6.11 snapshots `base`'s root + flat-state before any TransitionBackend mutation and asserts it's bit-identical after a series of writes. (3) A runtime `panic!` wrapper adds an indirection for every read without catching any bug class the type system and the test don't already cover. Belt-and-suspenders that already work.

---

## 10. Phase Handoff Protocol

Because this plan has 10 phases plus preflight, it will likely be executed by multiple `plan-implementer` agents in sequence (per the global CLAUDE.md rule for 4+ phase plans). Each phase must produce a **handoff artifact** that lets the next agent start cold.

### Handoff artifact

At the end of each phase, the implementing agent creates `docs/binary-trie/phases/phase-<N>-handoff.md` with:

```markdown
# Phase <N> Handoff

**Completed by**: <agent description>
**Branch**: shared-trie-binary
**Commit**: <full SHA at phase completion>
**Date**: <ISO 8601>

## Checkpoint verification

- [x] <copy the phase's checkpoint criteria, each ticked>

## Tests run

| Command | Result | Notes |
|---|---|---|
| `cargo test -p ethrex-binary-trie` | pass | N tests |
| `cargo check --workspace` | pass | clean |
| (etc.) | | |

## Files created / modified

<git diff --name-status output between phase entry and completion>

## Deviations from the plan

<MUST be empty under §2a rule 1. If non-empty, the phase is not complete. If a deviation was approved by the user via escalation, link the escalation file and the user's decision.>

## Notes for next phase

<Any factual observations that would save the next agent time. NOT opinions, NOT alternative designs. E.g. "generated test_vectors.json contains 47 vectors", "BinaryTrie::insert_multi was already present in the port, no adaptation needed".>
```

The handoff file is committed as part of the phase's final commit. The next agent reads the latest handoff file first, then starts Phase 0 (preflight) — yes, even if Phase 0 was done by a prior agent. Re-verify the environment; don't trust stale state.

### Per-phase commit discipline

- One commit per task is ideal but not required.
- Phases 1–8 each end with a single **"Phase N complete"** commit that includes the handoff file and passes all phase tests.
- Commit messages use conventional commits per `CLAUDE.md`: `feat(l1): phase N — <short summary>`.
- No `--amend` of other agents' commits. Each agent owns its own commit range.

### Context pickup for the next agent

The next `plan-implementer` agent's prompt template:

```
Continue implementation of the binary-trie backend per docs/binary-trie/plan.md.
Last completed phase: <N>.
Read docs/binary-trie/phases/phase-<N>-handoff.md first, then execute Phase 0
(preflight) to verify the environment, then execute Phase <N+1>.

Hard rules (from §2a):
- No deferrals, no skipping. Escalate per §11 if blocked.
- No TODO/unimplemented!/todo! in merged code.
- Every checkpoint is a hard gate; failing tests = phase incomplete.
- Phase <N+1> exit criteria must all tick before handing off.
```

## 11. Escalation Protocol

The §2a no-deferrals rule means the implementer must escalate rather than skip. "Escalate" is not "decide yourself" — it is "stop, surface to the human, wait for explicit decision."

### When to escalate

Any of the following:

- A task's specification conflicts with observed reality (e.g., a file path doesn't exist, a function signature has changed).
- A task cannot be implemented as specified because of a hidden dependency the plan didn't anticipate.
- A test specified in the plan is impossible to write because the thing being tested cannot be constructed.
- A reviewer (`plan-reviewer`, `code-reviewer`) flags a missing task, and the agent cannot find an existing implementation that covers it.
- A decision labeled "locked" in §3 appears to be wrong in context.
- Test output is ambiguous ("passed but with a warning about X").

### How to escalate

1. **Stop all work.** Do NOT commit partial changes. Do NOT continue on to a different task.
2. **Revert uncommitted changes** that relate to the blocked task: `git checkout -- <files>`. Preserve work on unrelated, already-completed tasks.
3. **Write a blocker file** at `docs/binary-trie/blockers/phase-<N>-<short-kebab-description>.md`:

   ```markdown
   # Blocker: Phase <N> — <short description>

   **Encountered by**: <agent>
   **Task**: <exact task number from the plan, e.g., 5.7>
   **Branch**: shared-trie-binary
   **Commit**: <SHA where the blocker was found>

   ## What the plan asked for

   <Direct quote from the plan.>

   ## What was observed

   <What broke. File paths, error messages, failing test output.>

   ## Why the plan's approach doesn't work

   <Concrete reason. No hand-waving. Not "it's complicated"; explicitly what fails.>

   ## Proposed alternatives

   1. <Option A with tradeoffs>
   2. <Option B with tradeoffs>
   3. <"Scope this task out of the PR" — only if genuinely impossible>

   ## Questions for the human

   <At most 3 specific yes/no or multiple-choice questions. Not open-ended.>
   ```

4. **Commit the blocker file** with message `docs(l1): blocker on phase <N> task <T>.<s>`.
5. **Return to the invoking agent / human** with a concise message pointing at the blocker file.
6. **Wait for explicit decision.** Do NOT resume work on the blocked task or any downstream task until the human resolves the blocker.

### What is NOT escalation

- Silently marking a task "not needed" or "already covered".
- Writing a stub and moving on.
- Choosing a simpler alternative unilaterally.
- Filing a GitHub issue and continuing.
- Leaving a `TODO` in the code.

Any of the above = plan violation, §2a rule 1.

### Reviewer findings

If `plan-reviewer` or `code-reviewer` flags something:

- **If the finding is valid**: fix it in the current phase before advancing. Do NOT defer to "follow-up PR".
- **If the finding is invalid** (the reviewer is wrong): write a one-paragraph rebuttal in the phase handoff file explaining why the finding doesn't apply. Do NOT silently ignore.
- **If unclear**: escalate per §11.

---

**Relevant file paths (all absolute):**

- `/data2/edgar/work/ethrex/docs/shared-trie/spec.md`
- `/data2/edgar/work/ethrex/docs/shared-trie/adding-a-backend.md`
- `/data2/edgar/work/ethrex/crates/common/state-backend/src/lib.rs`
- `/data2/edgar/work/ethrex/crates/common/trie/backend.rs`
- `/data2/edgar/work/ethrex/crates/common/trie/merkleizer.rs`
- `/data2/edgar/work/ethrex/crates/storage/state_backend.rs`
- `/data2/edgar/work/ethrex/crates/storage/merkleizer.rs`
- `/data2/edgar/work/ethrex/crates/storage/mpt_wiring.rs`
- `/data2/edgar/work/ethrex/crates/storage/store.rs` (notably `write_node_updates_direct` at line 1147 and the `apply_trie_updates` dispatch)
- `/data2/edgar/work/ethrex/crates/storage/api/tables.rs` (notably `TABLES: [&str; 19]` at line 120)
- `/data2/edgar/work/ethrex/crates/networking/p2p/sync.rs`
- `/data2/edgar/work/ethrex/crates/networking/p2p/sync_manager.rs`
- `/data2/edgar/work/ethrex/crates/networking/rpc/eth/account.rs`
- `/data2/edgar/work/ethrex/crates/networking/rpc/rpc.rs`
- `/data2/edgar/work/ethrex/crates/blockchain/blockchain.rs`
- `/data2/edgar/work/ethrex/crates/blockchain/metrics/mod.rs`
- `/data2/edgar/work/ethrex/cmd/ethrex/cli.rs`
- `/data2/edgar/work/ethrex/cmd/ethrex/initializers.rs`
- Reference branch: `gh api repos/lambdaclass/ethrex/contents/crates/common/binary_trie/<file>?ref=eip-7864-plan`
