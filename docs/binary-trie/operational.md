# Binary Trie Backend — Operational Guide

Who this is for: someone running ethrex with `--binary-transition` on a real machine, potentially against mainnet.

## What the feature does

When `--binary-transition` is passed, ethrex:

1. Starts and runs as a normal MPT node.
2. Completes snap sync (MPT).
3. Catches up to the finalized head.
4. As soon as both `snap_enabled=false` and `caught_up=true` hold (checked after each committed block), the activator automatically:
   - Acquires the activation lock (block execution pauses).
   - Force-flushes the MPT layer cache and FKV generator to disk.
   - Writes three metadata keys into `MISC_VALUES` atomically along with format byte `2`.
   - Hot-swaps the in-memory `backend_kind` to `Transition` (no restart needed).
   - Logs `Binary trie transition activated at block N. Frozen MPT root: 0x... Node continues running in Transition mode.`
5. The node continues running in Transition mode. No operator action is required.

**Activation is fully automatic and in-process.** There is no admin RPC, no CLI subcommand, no signal, no env var, and no restart needed. Set the flag at startup and let the node transition itself when it's ready.

From that point on, MPT is read-only. Every new state write is binary. The binary state root is computed and logged but never compared against the block header's `state_root` field.

## How to turn it on

```bash
# first startup: snap sync as normal; node auto-transitions when ready
ethrex --network mainnet --binary-transition --datadir /var/lib/ethrex

# ... node snap-syncs, catches up, then logs:
#   "Binary trie transition activated at block 23456789. Frozen MPT root: 0x...
#    Node continues running in Transition mode."
# ... and keeps running — no restart required.
```

## What the flag does

`--binary-transition` means "this node should run in binary-trie transition mode". The effect depends on the DB state at startup:

| DB state | Flag | Behavior |
|---|---|---|
| Fresh / pure MPT (format byte `0`) | absent | Normal MPT node; no binary-trie code runs |
| Fresh / pure MPT (format byte `0`) | present | MPT node + `TransitionActivator` observer; activator fires when preconditions are met, hot-swaps to Transition mode in-process |
| Transitioned (format byte `2`) | present | Starts in Transition mode: `StateBackend::Transition { base: MptBackend, overlay: BinaryBackend }` |
| Transitioned (format byte `2`) | absent | Fatal error: refuses to start, to prevent accidentally running as pure MPT on a transitioned DB |

Same flag, same meaning; surface behavior differs because the DB already knows what it is.

## Preconditions for activation

The activator fires exactly when all of the following are true, checked after each committed block:

- `--binary-transition` was passed at startup.
- `snap_enabled` has flipped false (snap sync is complete).
- `caught_up` is true — the follower's head is at or beyond the CL-reported finalized head.

Until all three are true, the node runs as a normal MPT node. There is no way to force activation earlier; the preconditions exist so that the binary overlay is rooted on a stable, fully-flushed MPT snapshot.

## What changes after activation

| Surface | Before activation | After activation |
|---|---|---|
| `eth_getBalance`, `eth_getTransactionCount`, `eth_getCode`, `eth_getStorageAt` | Served from MPT | Served from overlay with MPT fallback |
| `eth_getProof` | Returns EIP-1186 proof | Returns `-32099` error pointing at `eth_getBinaryProof` |
| `eth_getBinaryProof` | Not available | Returns binary trie proof (see `rpc.md`) |
| Block execution | MPT reads/writes | Overlay writes, composite reads |
| Block proposal | Normal | **Refused** with a clear error message |
| `ExecutionWitness` (zkVM) | Emitted | Error: "witness disabled in binary mode" |
| State root validation against block header | Enforced | Skipped |
| Snap sync | Not applicable (already complete) | Not applicable |
| Receipts / logs bloom / tx root / gas / withdrawals root | Validated | Validated (unchanged) |

## Restart invariants

On every restart:

- If format byte is `0` (Mpt) and `--binary-transition` is **not** passed → start as pure MPT node.
- If format byte is `0` and `--binary-transition` **is** passed → start as pure MPT node preparing for future activation.
- If format byte is `2` (Transition) and `--binary-transition` is passed → start in Transition mode.
- If format byte is `2` and `--binary-transition` is **not** passed → **fatal error**. Starting without the flag on a transitioned DB is refused to prevent accidentally writing MPT-format data onto a transition DB. Operator must either re-pass the flag or point ethrex at a different datadir.

Format byte `1` (pure Binary) is never reachable through the supported paths.

## Reorgs

ethrex's layer cache absorbs reorgs up to 128 blocks. For binary mode:

- **Reorg depth ≤ 128**: handled transparently by the per-backend layer caches (MPT's `TrieLayerCache` pre-switch, binary's `BinaryTrieLayerCache` post-switch). No operator intervention.
- **Reorg depth > 128**: fatal regardless of mode (the layer cache cannot reconstruct older states from committed disk data). This is pre-existing ethrex behavior for non-archive nodes, unchanged by this feature.

No explicit "reorg crossed the switch block" check exists. Rationale:

- Activation only fires after `caught_up` has latched true, which means the node has been at or beyond the CL-reported finalized head at least once. The switch block is therefore past or at finality.
- A reorg past a finalized block is a Byzantine consensus failure (requires ≥ 1/3 stake slashed) and has never occurred on post-merge mainnet.
- For such a reorg to cross the switch block it must also exceed the layer cache depth (128 blocks), which is already fatal for other reasons.

So switch-crossing reorgs are a strict subset of the already-fatal "deep reorg" class. Adding explicit detection would be defensive redundancy with no new protection, so we don't.

## Metrics

If metrics are enabled (see `--metrics` in the main ethrex README), the following are exposed alongside existing ethrex metrics:

| Metric | Type | Meaning |
|---|---|---|
| `ethrex_binary_trie_overlay_read_hit` | counter | Reads served from the binary overlay |
| `ethrex_binary_trie_mpt_fallback_read_hit` | counter | Reads that fell through overlay and were served from MPT |
| `ethrex_binary_trie_overlay_read_miss` | counter | Reads that missed both overlay and MPT (returning `None`) |
| `ethrex_binary_trie_stem_count` | gauge | Number of distinct stems in the overlay's on-disk state |
| `ethrex_binary_trie_code_chunks_written` | counter | Code chunks written post-switch |
| `ethrex_binary_trie_switch_activation_timestamp` | gauge | Unix timestamp of activation (0 if not yet transitioned) |
| `ethrex_binary_trie_tombstone_count` | gauge | Number of active tombstones in overlay (SELFDESTRUCTed MPT-resident accounts) |

The overlay hit ratio (`overlay_read_hit / (overlay_read_hit + mpt_fallback_read_hit)`) tells you how much of the working set has migrated into the binary trie organically. On a long-running node, this should climb steadily toward 1.

## What can go wrong

- **Activation during snap sync**: rejected with an explicit error. Retry after snap sync completes.
- **Activation while disk is full**: the format-byte + metadata-key write transaction fails, ethrex logs the error and continues running in MPT mode. Activation can be retried once disk space is recovered.
- **Process killed during activation**: the metadata write is a single DB transaction; either all four keys land or none do. On restart, format byte is checked first; if it's `2`, the other three keys must be present (enforced at startup; if any are missing, fatal error suggests wipe-and-resync). This failure mode is very narrow but should be tested.
- **Running against a non-mainnet chain**: supported in principle, but test vectors and integration tests do not cover Sepolia/Holesky/devnets specifically. Expect surprises; report them.
- **Running with `--dev`**: the dev network uses genesis-from-scratch. Since genesis-binary is unsupported, `--dev --binary-transition` is rejected with an explicit error.

## When to NOT use this

- You need validator-mode block proposal. (Binary mode refuses.)
- You need archive-mode historical state (pre-128-blocks). (Not supported in binary mode.)
- You need `eth_getProof` compatibility for a client that can't speak `eth_getBinaryProof`. (Use a pure-MPT node.)
- You need zkVM integration. (Not wired in this PR.)

## When it's a good fit

- You want to measure binary trie cost and shape against mainnet workloads.
- You want to develop tooling on binary trie state (proofs, analytics).
- You want to prototype consumer software that reads binary trie state through `eth_getBinaryProof`.
