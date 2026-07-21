# Binary Trie Backend — Design Decisions

Each section here documents a decision, the alternatives, and why we chose as we did. If you come across implementation code that seems to contradict one of these, the code is probably wrong.

## 1. Pure overlay (EIP-7612), not full migration

**Chosen**: After the switch block, writes go only to the binary overlay; reads check overlay first, fall back to MPT. MPT is frozen and never written to again. Untouched accounts stay in MPT forever.

**Alternatives**:
- EIP-7748 style incremental migration (batch-convert N accounts per block until MPT is empty). Complex — needs reorg-safe migration cursors, adds per-block CPU cost, and the user's research goal doesn't require MPT to be empty.
- Background shadow migration (execute primarily on MPT; replicate writes to binary in the background; swap primary once binary catches up). Clean end state (pure binary), but tripled engineering cost and no immediate research benefit.

**Why overlay**: Matches the user's stated flow — snap sync with MPT, then switch. The binary trie grows organically from post-switch writes, which is the interesting research data set anyway. MPT lives on, but it's frozen and quiescent: no additional engineering cost.

## 2. Atomic first-write CoW on overlay stems

**Chosen**: The first post-switch write to an MPT-resident account atomically writes all four `BASIC_DATA` sub-leaves (`version`, `code_size`, `nonce`, `balance`) plus `CODE_HASH` into the overlay. If the writing transaction only modified `balance`, the other three fields are copied from MPT at write time.

**Alternative**: Lazy per-sub-leaf writes. A balance-only update writes only the balance sub-leaf; other fields come from MPT on read.

**Why atomic**: Lazy writes create a "partial stem" state where some sub-leaves are in overlay and others in MPT. The read path becomes branching and error-prone: each sub-leaf independently checks overlay-then-MPT. With atomic first-write, the invariant is simple — an account is either fully in overlay or fully in MPT. Reads become branch-free: stem present in overlay → read all sub-leaves from overlay. The extra cost (4 extra leaf writes per first-touch) is paid once per account ever written post-switch, which is amortized over every future access to that account.

**Cost of atomic first-write**: Per first-touch of an MPT-resident account:
- 3 extra MPT reads (for the unmodified BASIC_DATA fields + code_hash, if not already loaded)
- 4 extra binary-trie leaf insertions

Measured against the fact that this happens once per account ever written post-switch (not per block, not per tx), the cost is negligible.

## 3. In-process hot-swap activation

**Chosen**: Activation writes the three transition metadata keys + format byte 2 in a single DB transaction, then atomically updates the in-memory `backend_kind` field (via `Store::set_backend_kind`) and the in-memory `transition_metadata` (via the `RwLock` updated inside `persist_transition_metadata`). The node continues running in Transition mode — no restart required.

**Rationale**: `StoreVmDatabase` is constructed per-block-execution and dropped at block end (`crates/blockchain/vm.rs:39-70`). The `activation_lock` already serializes `activate()` against `execute_block_pipeline` (wired in Phase 7 at `blockchain.rs:384`). Hot-swap is therefore two writes under an already-held lock — no new concurrency surface. The store's `backend_kind` field is now an `AtomicU8` (Release-store on set, Acquire-load on get), and `transition_metadata` is a `RwLock<Option<...>>` (write-locked only by `persist_transition_metadata`, read-locked by `transition_metadata()` accessor and the `Clone` impl).

**Previously documented as restart-required; reversed during Phase 7 review** after the live hoodi run (2026-05-04) showed that the activator's `CancellationToken::cancel()` was silently ignored by the `tokio::select!` loop, leaving the node running in MPT mode indefinitely. Operator explicitly directed "it should be seamless."

**Alternative (historical)**: A restart was initially chosen because the `Store`'s `StateBackend` was believed to be referenced by long-lived objects (RPC handlers, FKV generator thread, trie update worker). Investigation showed that `StoreVmDatabase` — the only caller of `new_state_reader` — is a transient per-block object. The other references hold `Arc<Store>` (not `Arc<StateBackend>`), so their reads go through the same `backend_kind` atomic they already hold. No Arc-swap coordination is needed.

## 4. BLAKE3 locked (no hash abstraction trait)

**Chosen**: Direct `blake3::hash` calls inside `ethrex-binary-trie`. No `BinaryCrypto` trait.

**Alternative**: A `BinaryCrypto` trait parallel to the existing `Crypto` trait, gating the hash choice behind a generic parameter or dyn dispatch.

**Why locked**: EIP-7864's draft nominates BLAKE3 but notes "TBD" — the final EIP may pick Poseidon2 or keccak. Adding a trait now would couple every callsite to generics, inflate monomorphization, and pay for a flexibility we can retrofit when the EIP stabilizes. If the hash changes, it is a one-crate edit in `hash.rs` + `key_mapping.rs`.

## 5. Sparse stems via `BTreeMap<u8, [u8; 32]>`

**Chosen**: `StemNode.values: BTreeMap<u8, [u8; 32]>`, inherited from the reference branch.

**Alternative**: Dense `[[u8; 32]; 256]` (8.5 KB per stem).

**Why sparse**: Typical stems have 1-5 entries (account with `BASIC_DATA` + `CODE_HASH` + a few storage slots). Dense storage wastes >99% of the allocation. The reference branch already uses `BTreeMap`; its iteration order (by sub-index byte) is exactly what merkleization needs. A fixed-size `Option<[u8; 32]>` array would also work and avoid indirection; we can revisit if the `BTreeMap` shows up in profiles.

## 6. Separate FKV table, overriding the shared-trie spec

**Chosen**: `BINARY_FLATKEYVALUE` is a new DB table, distinct from MPT's `ACCOUNT_FLATKEYVALUE` / `STORAGE_FLATKEYVALUE`.

**Alternative**: Share the existing tables; disambiguate by key format (MPT uses keccak-nibble paths; binary would use stems).

**Why separate**: Shared tables create a coupling where every FKV consumer must know which format a given key is in, either via the stored format marker byte or by key-length heuristics. Separate tables keep each backend's FKV loop entirely contained and let future backends bolt their own tables on without changing MPT code. The earlier `docs/shared-trie/adding-a-backend.md` recommended sharing; that recommendation is being superseded here and updated in the doc phase of the plan.

## 7. Tombstones split: side-table for trie, sentinel byte for cache

**Chosen**:
- Disk-level tombstone: side-table entries keyed `[0xFE, <stem 31 bytes>]` in `BINARY_TRIE_NODES`, with an empty or single-byte marker value. Not a trie leaf; the trie has no "deleted" concept.
- Cache-level tombstone: each binary-backend entry in `TrieLayerCache` is framed — `[0x00, ...value]` for a real value, `[0x01]` (single byte) for a tombstone.

**Alternative**: Reserve a sub-index byte (e.g. 0xFF) as the tombstone marker inside the stem itself.

**Why side-table + framing**: Reserving a sub-index byte would change the trie's leaf layout and potentially clash with future EIP-7864 revisions that extend the sub-index namespace. Side-tables are purely a storage concern, invisible to trie math. Framing in the cache disambiguates from `FxHashMap::get → None` cleanly, without relying on empty-vector interpretation.

**Why tombstones at all**: When SELFDESTRUCT targets an MPT-resident account post-switch, the account is gone. Without a tombstone, a subsequent read of that account would miss the overlay and fall back to MPT, resurrecting the pre-switch state. The tombstone records "this account was deleted after switch; do not fall through."

## 8. Dual-write code: chunks for state root, hash-keyed table for reads

**Chosen**: When post-switch code is deployed, we write both:
- Binary trie: 31-byte chunks at `(stem, CODE_OFFSET + i % STEM_SUBTREE_WIDTH)`, for state-root correctness per EIP-7864.
- `AccountCodes` table: `code_hash → bytecode`, as MPT does.

All code reads go through `AccountCodes` by `code_hash`. The binary trie's chunks are never read back.

**Alternative**: Reconstruct code from chunks at read time (pure EIP-7864).

**Why dual-write**: Chunk reconstruction requires an ordered read of up to 791 leaves (max contract size 24576 bytes / 31 byte chunk payload ≈ 791), re-assembling into a contiguous byte buffer, for every EXTCODECOPY. MPT's `code_hash → bytecode` lookup is a single point read. Maintaining both keeps EVM semantics identical without sacrificing state-root correctness. The storage overhead is negligible — contract code is a tiny fraction of state size.

**The downside**: A binary-trie-native implementation can eventually drop `AccountCodes` and read chunks directly. We don't do that here; the research goal is studying the trie, not testing alternative code-storage schemes.

## 9. No binary state-root verification

**Chosen**: The `execute_block_pipeline`, when in `Transition` or `Binary` mode, skips the `state_root == expected_state_root` assertion at the end of block execution.

**Why**: Mainnet blocks commit the MPT root. Our binary trie produces a different root by construction. Comparing is meaningless. Receipts, logs bloom, transactions root, withdrawals root, gas used, and every VM-side invariant are still checked — we just don't verify the global state commitment in binary mode.

**Consequence**: If binary trie code has a bug that produces subtly wrong state, we won't catch it via block validation. We catch it via:
- Unit tests against EIP test vectors
- Integration tests that script `TransitionBackend` through sequences and compare reads against an oracle
- Cross-check RPC: `eth_getBalance` on an account should return the same value from the binary overlay as from an MPT node following the same chain

## 10. One-way transition

**Chosen**: Activation is irreversible. Reorgs crossing the switch block are fatal.

**Alternatives**:
- Maintain both trees in parallel indefinitely (deactivation just drops the binary trie)
- Keep a reversion-point snapshot at switch block

**Why one-way**: The research goal is a node that has transitioned. Reversibility doubles the engineering cost and the test surface for zero research benefit. The 128-block layer cache absorbs typical reorg depths; a reorg deeper than that is already a Byzantine event on mainnet (it has never happened post-merge), and treating it as fatal is consistent with how non-archive nodes handle deep reorgs today.

## 11. No genesis-from-binary

**Chosen**: `StateBackend::compute_genesis_block(BackendKind::Binary, _)` returns an error. The only entry into binary mode is through `Transition`.

**Why**: Genesis-from-binary would need an EIP-7864-compliant genesis state construction (alloc → stems → root) and would fork the "startup" code path. For the research goal (a mainnet follower in binary mode), genesis-binary is unnecessary. When it's time to add it, the implementation is straightforward but additive — no existing code has to change.

## 12. Witness generation disabled

**Chosen**: `ExecutionWitness` generation returns an error when `BackendKind != Mpt`.

**Why**: Witnesses are the zkVM integration point. zkVM is out of scope for this PR. Emitting MPT-format witnesses from a binary-trie node would be lying about what the state commitment is; emitting binary-format witnesses requires a binary-trie zkVM guest that doesn't exist yet. Gate and move on.

## 13. Single-tree, level-parallel merkelize + sparse stem hashing (NOT MPT shard parity)

**Chosen**: `BinaryMerkleizer` uses a single `BinaryTrieState` (no sharding). Apply is serial. Merkelize parallelizes by tree level: one serial walk collects dirty nodes (those with `cached_hash == None`) bucketed by depth, then levels are processed bottom-up with `rayon::par_iter` within each level. StemNode internal merkelization uses sparse hashing — zero subtrees are short-circuited via EIP-7864's `hash([0; 64]) = [0; 32]` rule, so a stem with K occupied sub-indices rehashes in ~K·8 BLAKE3 calls instead of ~511. The public API (`feed_updates` / `finalize`) matches `MptMerkleizer` at the enum-dispatch boundary so `execute_block_pipeline` sees no difference.

**Rejected alternatives**:
- **16-shard worker pool, mirroring `MptMerkleizer`** (an earlier version of this doc; Phase 4's first implementation attempt). MPT shards by `hashed_address[0] >> 4` because the MPT root is a 16-way `BranchNode` — shard N is exactly child N. Binary trie's root is a 2-way `InternalNode`; sharding by the top 4 bits places each shard at depth 4, with a 4-level skeletal spine of shared ancestors above it. Combining shard roots into a global root requires a new `insert_at_depth` primitive or post-merkelize subtree extraction. Either adds complexity for modest gain, because sharding parallelizes apply (cheap) while leaving merkelize (expensive) single-threaded or requiring a full tree rebuild. We tried this; it shipped with a `code_updates` double-emit bug and the fkv-rebuild fallback was single-threaded at merkelize anyway. Deleted.
- **Pure single-threaded.** Simple. For 10k-stem blocks, ~30-50 ms serial merkelize. Works for mainnet (12 s block time) but sits in the same ballpark as MPT's ~80 ms ceiling — no performance headroom for larger workloads. Rejected because level-parallel is only ~5 lines more code and scales with core count.

**Why this is the right fit for binary trie**:
- **The work IS BLAKE3.** Apply is bit-path traversal + leaf writes + `cached_hash = None` invalidation. Zero hashing. For 10k updates: ~1-5 ms. Merkelize is pure BLAKE3 on the dirty frontier: for 4000 modified stems, ~100k hashes × 500 ns = ~50 ms serial. The optimization problem is *literally* "maximize BLAKE3 throughput on a dependency DAG." Sharding doesn't help; parallel hashing does.
- **Level-parallelism is correctness-safe by construction.** Within a single tree level, no node depends on another node at the same level. `par_iter` over a level is always safe without any inter-thread coordination. Levels are processed bottom-up so by the time a level starts, its children's hashes are already computed.
- **Scales with core count.** Not fixed at 16 shards; an 8-core machine gets ~8×, a 32-core machine gets ~32× (bounded by tree shape and level sizes).
- **Sparse stem hashing is a pure algorithmic win.** Typical stems have 1-5 occupied sub-indices out of 256. Naive 256-leaf merkelize = ~511 hashes per rehash. Sparse merkelize = ~K·8 hashes. 30-60× speedup per stem. Orthogonal to outer parallelism; stacks multiplicatively.
- **No coordination complexity.** Zero workers, zero channels, zero panic-capture plumbing, zero worker lifecycle. One tree, one owner, one path to correctness. Phase 4's first attempt shipped 900 lines of merkleizer infrastructure with two coordination bugs; this shape fits in ~200 lines with no coordination bugs possible.

**Public API parity is still maintained.** The `BinaryMerkleizer { feed_updates, finalize }` shape matches `MptMerkleizer` at the trait boundary, so `execute_block_pipeline` dispatches identically. Internal implementation is free to diverge where the data structure demands it.

**Benchmarking honesty**: an earlier version of this decision argued that level-parallel "sandbags binary trie" vs MPT's 16-shard apply parallelism. That argument was flawed — it assumed apply was the bottleneck, but apply is ~1 ms while merkelize is ~50 ms. Level-parallel merkelize gives binary trie better parallelism than MPT's shard model gives MPT on the hashing phase, not worse. The benchmark comparison is honest: both backends optimize the phase that dominates their own workload. Binary's phase is merkelize; MPT's is apply + merkelize distributed across shards.

**Infrastructure parity elsewhere is unchanged**: `BinaryTrieLayerCache` mirrors `TrieLayerCache` (bloom filter, 128-layer commit threshold); the storage-layer wiring (Phase 5) preserves MPT's structural patterns. Only merkelization deviates.
