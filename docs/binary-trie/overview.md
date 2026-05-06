# Binary Trie Backend — Overview

This directory documents the EIP-7864 binary trie backend for ethrex. The feature is research-only: ethrex can follow L1 mainnet using a binary trie layered on top of its existing MPT state, without proposing blocks or verifying the binary state root against block headers.

## Reading order

1. This file — high-level orientation.
2. `design-decisions.md` — why we chose overlay semantics, BLAKE3, sparse stems, and in-process hot-swap activation.
3. `plan.md` — full phased implementation plan.
4. `rpc.md` — `eth_getBinaryProof` wire format.
5. `testing.md` — how to regenerate EIP test vectors and run the binary-trie test suite.
6. `operational.md` — running ethrex with `--binary-transition` in practice.

## One-paragraph summary

At a chosen switch block, MPT is frozen. From the next block onward, every state write goes to a BLAKE3-based binary trie (EIP-7864). Reads check the binary overlay first and fall back to MPT for state that was never touched post-switch. MPT is never migrated and never written to again. Activation is **fully automatic**: a node launched with `--binary-transition` transitions itself as soon as snap sync completes and the follower catches up to the finalized head. Activation is one-way and in-process: it writes three metadata keys atomically, hot-swaps the backend kind to `Transition`, and the node continues running without operator intervention. No admin RPC, no manual trigger, no restart required.

## Crate layout (after implementation)

```
ethrex-common                Base types, no trie deps
      ^
      |
ethrex-state-backend         Traits: StateReader, StateCommitter
      ^        ^
      |        |
ethrex-trie  ethrex-binary-trie    MptBackend / BinaryBackend (+ merkleizers)
      ^        ^
      +---+----+
          |
      ethrex-storage         StateBackend enum: Mpt | Binary | Transition
                             Merkleizer enum:   Mpt | Binary | Transition
                             mpt_wiring.rs, binary_wiring.rs, transition_wiring.rs
```

`ethrex-trie` and `ethrex-binary-trie` do not know about each other. `ethrex-storage` is the only crate that sees both and composes them into `TransitionBackend`.

## What this feature is NOT

- It is not a hard fork activation mechanism. The binary trie root is not published to peers, not verified against block headers, and not used for consensus.
- It is not a full migration. Pre-switch state stays in MPT forever.
- It is not a block-proposal mode. Follower only.
- It is not a zkVM integration. Execution witnesses are disabled in binary/transition mode.
- It is not a snap-sync target. Snap sync stays MPT-only; transition activates after snap completes.

## Key invariants

1. **One-way switch.** Once transition is activated on a database, it cannot be undone. Reorgs deeper than 128 blocks are already fatal on any non-archive ethrex node (layer-cache depth limit); this feature inherits that property. No explicit "reorg crossed switch" detection is added — activation only fires after `caught_up` latches true, by which point the switch block is past or at finality, so switch-crossing reorgs are a strict subset of already-fatal deep reorgs.
2. **Overlay stem integrity.** The first post-switch write to an MPT-resident account atomically writes all four `BASIC_DATA` sub-leaves plus `CODE_HASH` to the overlay (copying any missing fields from MPT). After that write, the account is "in overlay"; its MPT representation is ignored for reads. This prevents partial-stem read hazards.
3. **Code reads via `code_hash`.** Post-switch-deployed code is written to both the binary trie (as chunks, for state-root correctness) and the legacy `AccountCodes` table (by `code_hash`). All code reads go through `AccountCodes`. Chunk reconstruction is never required at read time.
4. **No state-root verification post-switch.** Binary root is computed and exposed via RPC but never compared to the block header's `state_root` field. Receipts, logs bloom, transactions root, withdrawals root, and gas usage remain fully verified.
5. **In-process activation (hot-swap).** Activation writes format byte 2 + three metadata keys atomically, then hot-swaps `store.backend_kind` to `Transition` (via `AtomicU8::store(Release)`) and updates the in-memory `transition_metadata` (via `RwLock` write). The `activation_lock` serializes this write against concurrent block execution. No restart is required; the node continues in Transition mode immediately after the lock is released.
6. **Public API parity with MPT, implementation optimized for binary's structure.** `BinaryMerkleizer` matches `MptMerkleizer` at the `feed_updates` / `finalize` boundary so `execute_block_pipeline` dispatches identically. Internal implementation diverges: **single-tree serial apply + level-parallel `rayon::par_iter` merkelize + sparse StemNode hashing**. No 16-shard worker pool — binary trie's 2-way branching doesn't admit clean sharding, and the work that dominates a block is BLAKE3 on the dirty frontier, which level-parallelism targets directly. Storage-layer primitives (`BinaryTrieLayerCache` with bloom filter + 128-layer commit threshold) still mirror MPT. See `design-decisions.md` §13.

## Activation state machine

```
┌──────────────┐  --binary-transition on; after each block, check:
│  MPT (byte 0)│    snap_enabled == false  AND  caught_up == true
└──────┬───────┘                                          │
       │                                                  │
       │  neither true → run as normal MPT node           │
       │                                                  │
       └──────────────────────────────────────────────────┤  both true → activator fires
                                                          v
                                            ┌─────────────────────────────────┐
                                            │ Acquire activation_lock         │
                                            │ Force-flush MPT layer cache     │
                                            │ Drain FKV + trie_update worker  │
                                            │ Write format byte = 2           │
                                            │ Write transition_switch_block   │
                                            │       transition_mpt_root       │
                                            │       transition_binary_root    │
                                            │   (all atomic in one tx)        │
                                            │ Hot-swap backend_kind in-memory │
                                            │ Log activation message          │
                                            └─────────────┬───────────────────┘
                                                          │ (node keeps running)
                                                          v
┌──────────────────────────────────────────────────────────────────────────────────────────┐
│  Transition (byte 2):                                                                    │
│    StateBackend::Transition { base: MptBackend (read-only), overlay: BinaryBackend }     │
│    Merkleizer::Transition(BinaryMerkleizer)                                              │
│    Reads: overlay → base fallback                                                        │
│    Writes: overlay only                                                                  │
└──────────────────────────────────────────────────────────────────────────────────────────┘
```

`Binary` (byte 1) is defined in the spec for symmetry but is not reachable from this release — the only way into binary mode is through `Transition`. Attempting to start a fresh DB with `--binary-only` is an error.
