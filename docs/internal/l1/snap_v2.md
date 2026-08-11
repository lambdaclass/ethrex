# snap/2 (EIP-8189) internals

snap/2 replaces the iterative `GetTrieNodes` / `TrieNodes` round-trips of the
snap/1 healing phase with Block Access List (EIP-7928) replay. Instead of
repairing a range download by asking peers for the individual trie nodes that
disagree, a snap/2 sync downloads flat key-value state, patches it with the
access lists of the blocks that passed while it downloaded, and rebuilds every
trie locally from the result.

Implements [EIP-8189](https://eips.ethereum.org/EIPS/eip-8189) and depends on
EIP-7928 for the `block_access_list_hash` header field.

The client-side algorithm is normative and is specified in devp2p
`caps/snap.md`, "Synchronization algorithm", not in EIP-8189, which only
sketches it. Concurrent account and storage fetch, a monotonic cursor across
pivot moves, and one final root check are all requirements from that text
rather than implementation choices.

## Capability negotiation

What a connection advertises depends on whether this node's own state sync still
depends on trie nodes, via `advertised_snap_capabilities(needs_trie_nodes)`.
EIP-8189 ("Backwards Compatibility") says a node synchronizing data should use
one snap version for state sync and serve both only once synchronization is
complete. A snap/1 state sync reconciles the trie with `GetTrieNodes`, which
snap/2 removes, so a node that offered snap/2 while depending on them would
negotiate away its only healing mechanism and then find no peer able to serve
it; `heal_state_trie` would re-queue its batch and spin.

The flag is set in `SyncManager::new` from "a snap sync is running **and** the
chain has no Amsterdam", since snap/2 reconciles with access lists that do not
exist before that fork. It is cleared as soon as the sync commits to the snap/2
path.

Keying it on "a snap sync is running" alone is circular and strands the snap/2
client permanently: withholding snap/2 means never negotiating it, which means
never finding a snap/2 peer, which means always falling back to snap/1, which
never clears the flag. The fallback leaves a correctly synced node behind, so
this reads as success from the outside.

The Hello exchange picks the highest snap version common to the peer's list and
the set this connection advertised, matching against the advertised set rather
than `SUPPORTED_SNAP_CAPABILITIES`, so a version withheld by the gate above is
not negotiated back. The result lives on `Established.negotiated_snap_capability`
and is mirrored into the codec via
`RLPxCodec.snap_version: Arc<RwLock<Option<SnapCapVersion>>>` so cross-version
codes are rejected at decode time. `SnapCapVersion::V1` accepts codes
`0x00..=0x07`; `V2` accepts `0x00..=0x05` plus `0x08`, `0x09`.

`GetTrieNodes` / `TrieNodes` are absent from snap/2, so any healing code path
that sends them must restrict peer selection to snap/1 via
`SNAP1_ONLY_CAPABILITIES` in `rlpx/p2p.rs`.

That restriction only works because peer selection filters on
`PeerData.negotiated_capabilities` rather than on the advertised
`supported_capabilities`. A peer advertising both snap/1 and snap/2 negotiates
snap/2 and would reject `GetTrieNodes`, so matching the advertised list would
route trie-node healing straight into a protocol error. `supported_capabilities`
remains the full advertised list because `admin_peers` reports it, which makes
the two easy to confuse when reading a live node.

## Wire format

`Snap2GetBlockAccessLists` carries `[id, [hashes...], response_bytes]`.
`response_bytes` is a soft cap; `0` means "use the default" (2 MiB).

`Snap2BlockAccessLists` carries `[id, [entries...]]` with one entry per
requested hash, in order. An unavailable BAL is encoded as the RLP empty
string `0x80` (NOT the empty list `0xc0` — that is eth/71's `OptionalBal`
convention, a different protocol). The codec test
`snap2_bal_none_uses_0x80_sentinel` locks the sentinel byte against
regressions.

```rust
pub struct Snap2GetBlockAccessLists {
    pub id: u64,
    pub block_hashes: Vec<H256>,
    pub response_bytes: u64,
}

pub struct Snap2BlockAccessLists {
    pub id: u64,
    pub bals: Vec<Option<BlockAccessList>>,
}
```

`caps/snap.md` writes the response body as `bals: [bal1: B, …]`, which reads
like each entry is wrapped. It is not: entries are the BAL's own RLP list
spliced in raw, with `0x80` for an absent one. That is a spec bug, and ethrex
follows the wire behaviour rather than the notation.

## Server handler

`build_snap2_bal_response` in `rlpx/connection/server.rs` builds the response
from a batched `Store::iter_block_access_lists_by_hashes`. No per-hash header
lookup is needed: BAL storage is gated on the Amsterdam fork, so a stored BAL
implies a post-Amsterdam block and is served directly, while a pre-Amsterdam,
pruned, or unknown block has nothing stored and yields `None`.

The byte budget is tracked via `bal.length()` (the zero-allocation
`RLPEncode` trait method) and capped at `min(response_bytes, 2 MiB)`. When
the cap is exceeded the loop breaks, preserving order up to the cutoff and
keeping at least one entry. The handler always returns a response, never
drops the request, and serves orphaned (non-canonical) blocks the same as
canonical ones because the storage is keyed by hash.

A defensive check rejects snap/2 messages received over a snap/1 connection
by sending `DisconnectReason::ProtocolError`. The codec already rejects
cross-version codes at decode time, so this only catches misconfigurations.

## Client request

`PeerHandler::request_snap2_bals` filters on `Capability::snap(2)` so the
request only goes to a peer that can serve it. `Ok(None)` signals "no
snap/2 peer available" and the caller falls back to snap/1 healing. A peer
that returns a mismatched `id` or a non-`Snap2BlockAccessLists` reply is
recorded as a failure.

The capability list it takes is *required*, not excluded. The "excluding" in
`get_best_peer_excluding` refers to `failed_peers`, which is a separate
argument.

## The state sync pipeline

`sync/snap2/` holds the client half. `snap2_sync` in `snap_sync.rs` runs four
stages in order: download, catch-up (interleaved with the download on every
pivot move), reconstruction, then the root check.

### Range download

`download.rs` splits the account hash space into `ACCOUNT_RANGE_CHUNK_COUNT`
(800) ranges and works them concurrently. Each range owns a `next` cursor and a
`last` bound; a range retires when `next` passes `last`.

`worker.rs` issues the individual requests and verifies each response against
the root the request named before returning it. For storage that root is the
account's own, which is only correct while the account leaf that carried it came
from the same pivot; the driver guarantees that, the worker does not check it.

Accounts and storage are fetched **concurrently**, which is what keeps the
result honest. snap/1 downloads every account first and every storage
afterwards, closing the gap by healing the account trie back to the current
pivot before each storage pass. snap/2 cannot make that repair, because it
removes `GetTrieNodes`. So a range advances only once its accounts *and* their
storage are in, in batches of `STORAGE_BATCH_SIZE` (300) accounts per storage
request. Work still inside a pending range is dropped and re-requested when the
pivot moves.

Two peer-level guards, both of which exist because a peer that cannot serve
still looks perfectly healthy from the outside:

- Every response scores its peer via `record_success` / `record_failure`.
  Without this, a peer stalled behind a fork it could not follow keeps full
  score, wins selection forever, and the download retries it indefinitely.
- The stall guard is measured against **frontier movement**, not request
  dispatch. A download feeding a dead peer is busy by every local measure and
  getting nowhere. `STALL_TIMEOUT` is 300s, generous because a thin or busy
  peer set can go quiet without anything being wrong.

Two response shapes are not failures and must retire their range, or it is
rescheduled forever while every response still verifies:

- Accounts returned but all beyond the range's limit. `caps/snap.md` has a peer
  fall back to the first account past `limit_hash` when the range itself is
  empty, so this is the answer for an empty span. The range is served through to
  `last`.
- No accounts at all, **with** a proof. Past the last account of the trie there
  is no account in range and none after the limit either, so an edge proof of
  absence is the only available answer. `verify_range` checks it. Only an empty
  answer with *no* proof means the peer does not hold the root, and that is a
  genuine failure.

### The download cursor and the frontier

`cursor.rs` tracks how far the download has advanced through the key space. The
flat state it produces mixes leaves from roots `R₀ … Rₙ`, and is patched back
into consistency by access lists. A patch is only correct for a key the download
has already passed: a key still ahead of the cursor will be served later, at a
newer root, and already carries the change the access list describes. Applying
it twice, or applying it to a key that is not there yet, corrupts the flat
state, so every BAL write is gated on the cursor's predicates.

The invariant the whole design rests on:

> An account below the frontier has been served **together with its storage**.
> An account above it has not been served at all. Nothing may sit in between.

The frontier therefore commits per response batch (accounts plus their storage),
never per range. Committing per range leaves accounts served between the two
positions invisible to catch-up: the frontier has moved past them, so the access
lists that changed them are never applied again, and no later check catches it.
The frontier and the request cursor must never disagree.

### Flat state

`flat.rs` holds the download's output. Patching means reading a leaf the
download already produced, changing it, and writing it back, so the output has
to stay addressable for the whole sync rather than being consumed once at the
end. Range responses arrive as sorted chunk files (`RANGE_FILE_CHUNK_SIZE`,
64 MiB) and are absorbed in bulk; access-list diffs are applied on top as
individual writes.

Absorbing a chunk cannot clobber a diff already written, because a chunk only
ever covers keys the download had not reached, and the cursor refuses to patch
exactly those.

### Pivot moves and access-list catch-up

`catchup.rs` implements steps 3 and 4 of the algorithm: as the chain advances
from `P` to `P+K`, fetch the BALs for `P+1..P+K`, verify each against its
header's `block_access_list_hash`, and apply the diff to the partial flat state.
`P+K` is then the target for the remaining range requests. This runs on every
pivot move before the download resumes against the new root, so the flat state
is never more than one pivot behind the range requests.

`MAX_CATCH_UP_BLOCKS` is `3533 * 32`: EIP-7928 obliges the execution layer to
retain access lists for at least the weak subjectivity period, so that is the
largest gap a peer can be expected to serve. Past it a catch-up would stall
partway through, and `caps/snap.md` has the node discard its partial state and
resync instead.

The span to catch up is walked back along parent hashes from the target, not
read from the canonical index. `Store::add_block_headers` writes HEADERS and
BLOCK_NUMBERS but **not** the canonical-block-hash table, which stays empty
until the sync commits, so the canonical index reads as empty mid-sync. Walking
parent hashes is also the reorg check.

### Reconstruction and the root check

`generate.rs` rebuilds every trie from the flat state and compares the resulting
root against the pivot header. This is the only place a snap/2 sync checks its
work. snap/1 gets the check for free, because healing walks down from the
pivot's root and cannot finish unless the trie reaches it; snap/2 removes
healing, so this comparison is what stands between a corrupt download and a
corrupt chain.

Each account is written with the root its storage trie actually hashes to, not
the one served with it. Served roots come from whichever pivot answered that
account's range and disagree with the slots on disk as soon as the pivot moves.

Storage tries take one of two builders, chosen by slot count. At or above
`BULK_STORAGE_TRIE_SLOTS` a contract goes through `trie_from_sorted_accounts`,
the same bulk builder the account trie uses, which constructs bottom-up from
sorted leaves and writes through the trie's own db. Below it the contract is
built slot by slot and its node changes are buffered, because the bulk builder
spawns a writer pool per call and most contracts hold a handful of slots. The
buffered changes are flushed every `STORAGE_WRITE_BATCH_NODES` nodes; writing
each contract as it is built would open one write transaction per contract.

Both builders must agree on the root, since a real state exercises both and only
the combined state root would show a disagreement, far from the contract that
caused it. `a_bulk_built_storage_trie_matches_a_slot_by_slot_one` pins that.

On success the sync latches `snap2_reconstructed_block` in the diagnostics.
That is the terminal evidence the snap/2 path ran to completion, and it is what
test harnesses should assert on.

### Failure: discard and restart

Any error in the download or reconstruction runs `discard_partial_state`, and
the discard has to be real. `caps/snap.md` leaves one remedy for a sync that
cannot be made consistent, "discard partial state and restart synchronization".
Leaving the flat state behind would let the next attempt reuse leaves for
accounts that no longer exist, which no later check would catch: the frontier
would have moved past them, so the access lists that deleted them are never
applied again.

## BAL replay on the snap/1 path

`sync/bal_healing/` is a separate, older use of the same messages: it
accelerates the snap/1 post-download healing loop with BAL replay. It is not
part of the snap/2 pipeline above and remains in use whenever the sync falls
back to snap/1.

### Applier

`sync/bal_healing/apply.rs::apply_bal(store, parent_state_root, bal, header)`:

1. Empty-BAL short-circuit — `bal.is_empty()` returns `parent_state_root`
   directly.
2. Hash validation — `bal.compute_hash()` must equal
   `header.block_access_list_hash.unwrap_or(EMPTY_BLOCK_ACCESS_LIST_HASH)`.
3. `bal.validate_ordering()` — defense against malicious peers reordering
   entries to forge a different post-state with the same RLP encoding.
4. Apply balance, nonce, code, and storage diffs derivable from the BAL.
   Trie writes go via `write_batch(STORAGE_TRIE_NODES, …)` which bypasses
   `TrieLayerCache` cleanly: the cache reads, batch writes go to the
   backend directly, no invalidation needed.
5. Persist the BAL via `Store::store_block_access_list` so this node can
   serve it onward (the heal path never goes through `store_block`).
6. Return the post-block state root.

A wrong-state-root return triggers `SyncError::StateRootMismatch`, which is
classified as recoverable so the outer loop can retry with a different peer.

Only the post-block value of each entry matters, so the last change recorded for
a key wins and intermediate ones are ignored. Each account's `storage_root` is
left as served: it is wrong in general, and it is recomputed from the
reconstructed storage tries, so writing anything there would be discarded.

Correctness rests on the access list enumerating every storage change as an
individual slot write, a premise carried from go-ethereum's
`eth/protocols/snap/bal_apply.go`. This holds post
[EIP-6780](https://eips.ethereum.org/EIPS/eip-6780): pre-existing contracts can
no longer be destructed, so storage only changes via SSTOREs, all of which are
recorded. **Networks with the legacy SELFDESTRUCT break this premise**, since
wholesale storage wipes carry no per-slot writes and leave already-downloaded
slots stale.

### Driver

`advance_state_via_bals` in `sync/bal_healing/mod.rs` loads canonical
headers from `start_block.number + 1` to the target, then requests BALs in
batches of `BAL_REQUEST_BATCH_SIZE` — derived from the response soft cap and
EIP-7928's average BAL size so a typical response is not truncated — retrying
each block up to `BAL_MAX_RETRIES_PER_BLOCK` (3) times. Strict in-batch
ordering: a slot is only applied once all prior slots in the batch have been
applied. A parent-hash check before each apply returns
`SyncError::ChainReorgDetected` (non-recoverable) on mismatch.

A response covering no slot at all is charged to the first pending slot's retry
budget, so a peer that truncates everything cannot stall the driver. Once any
slot exhausts its retries the driver returns the partial state root reached so
far; the caller in `snap_sync.rs` compares it against the target and runs
snap/1 healing for the remainder.

### Snap-sync integration

This replay implements EIP-8189 steps 5 and 7 and runs only in the
post-bulk-download healing loop. The healing pass inside the storage-ranges
download loop stays on snap/1: a BAL carries state changes, not the state trie,
so a diff like `balance(X): a→b` needs an existing local value to apply to, and
during bulk download most accounts do not have one yet.

The loop tracks `completed_pivot` — the pivot whose state a healing pass
verified in full. That is the only valid starting point for a replay, because
the diffs are applied on top of it. Replaying from the *current* pivot instead
would apply a span of changes to a state that does not correspond to its first
block, and the resulting root would be wrong in a way the per-block check
cannot attribute.

While `completed_pivot` is `None` — the first pass, and after any reorg — the
loop heals. Once a pass sets it and the chain advances, a later pivot is reached
by replaying the span between them and checking the result against
`pivot_header.state_root` (step 7). A short root, a peer failure, or an
unavailable BAL falls back to healing for that pass; `SyncError::ChainReorgDetected`
additionally clears the anchor, since a reorg past the pivot means the state it
names belongs to an abandoned fork.

Note that a BAL cannot supply an account's `storage_root`: EIP-7928's
`AccountChanges` is `[address, storage_changes, storage_reads, balance_changes,
nonce_changes, code_changes]`, with no root. A root is only re-derivable by
re-hashing the storage trie, which is why replay is placed after the storage
download rather than before it.

## Pre-Amsterdam handling

`block_access_list_hash` is absent in pre-Amsterdam headers, so snap/2 is
functionally dormant before the fork: the server returns `None` for every
pre-Amsterdam hash, and the catch-up requires the anchoring pivot to carry a
`block_access_list_hash`, so the driver never starts. A peer returning
`Some(bal)` for a header whose `block_access_list_hash` is `None` is a protocol
violation; the hash check against `EMPTY_BLOCK_ACCESS_LIST_HASH` catches it.

Because a pre-Amsterdam pivot still needs `GetTrieNodes`, the capability gate
cannot become unconditional and snap/1 has to stay as the fallback path.

## Errors

`SyncError` gains four variants in `sync.rs`:

- `StateRootMismatch(expected, got)` — applied BAL produced a different
  state root from `header.state_root`. Recoverable.
- `MissingHeaderForBal(BlockHash)` — local header missing for a BAL we
  need to apply. Non-recoverable (DB inconsistency).
- `MissingCanonicalBlock(number)` — no canonical hash recorded at a block
  number in the replay range. Non-recoverable (DB inconsistency).
- `ChainReorgDetected { expected_parent, actual_parent }` — peer's BAL
  chain doesn't connect to our local view. Non-recoverable; the caller
  falls back to snap/1.

## Diagnostics

`SyncDiagnostics` is served by `admin_syncStatus`. Replay counters:
`snap2_bal_requests_sent`, `snap2_blocks_replayed`, `snap2_bals_unavailable`,
`snap2_validation_failures`, `snap2_peer_failures`.

Download counters: `snap2_ranges_served`, `snap2_ranges_unverified`,
`snap2_storage_requeued`, `snap2_storage_partial`, plus
`snap2_ranges_remaining` in `phase_progress`. A high served count against a
flat remaining count is the signature of a range that verifies every response
and retires nothing.

`snap2_reconstructed_block` is latched, not sampled, and carries the pivot whose
root reconstruction reproduced. Use it, not `current_phase`, to decide which
path a sync took: the phase fields describe the sync only while it runs, and a
small state downloads between two polls.

## Testing

Unit and integration coverage lives in `test/tests/p2p/`: codec round-trips,
server responses, `apply_bal`, the cursor predicates, flat state, and
reconstruction. Hive covers the serving side.

`scripts/snap2-devnet/join.py` covers the client side against real peers. It
joins a fresh ethrex node to a running kurtosis enclave, drives it with
`engine_forkchoiceUpdatedV4`, and asserts on which path the sync took.

Devnet requirements, all load-bearing:

- **Mainnet preset, not `minimal`.** Minimal reaches Amsterdam sooner but block
  production dies at the Gloas transition: `payload_attestation_data` 404s,
  builder registration 500s, every slot empty.
- **`gloas_fork_epoch >= 1`.** At 0 the beacon genesis state is a Gloas state
  and lighthouse cannot decode it.
- **EIP-8282 builder predeploys preloaded.** The genesis generator omits them
  and their empty code invalidates any Amsterdam+ block.
- **EIP-7997 deterministic factory in genesis** (nonce 1). Without it, a geth
  peer that injects the factory at the fork boundary produces a block-access
  index 0 nonce change ethrex never makes, and the two clients reject each
  other's transition block.
- Forkchoice must send **zero** safe and finalized hashes. A non-zero hash the
  node has never seen is looked up and order-checked, which errors instead of
  returning SYNCING, so the sync never starts.
- Under the `sync-test` feature, `MIN_FULL_BLOCKS`, `SNAP_LIMIT` and
  `SECONDS_PER_BLOCK` are overridable, since production values need a chain
  10k blocks deep and a 25-minute pivot lifetime. `SECONDS_PER_BLOCK` must match
  the real slot time or `update_pivot` mis-estimates and pivots arrive stale.

The trap the harness exists to avoid: **a snap/1 fallback also ends with a
correctly synced node**, so "the node synced" is not evidence that snap/2 ran.
Assert on the latched reconstruction.

## Known gaps

- **Access-list catch-up is not exercised on every run.** A state small enough
  to download inside a single pivot never makes the pivot go stale, so catch-up
  correctly has nothing to do. It has been verified on a slower run (48 blocks
  over 4 rounds, 0 validation failures); the harness reports when a run leaves
  it untested rather than passing silently. Covering it deliberately needs real
  state on the devnet, not a lower `SNAP_LIMIT`.
- **Reorg past the pivot is unimplemented.** The sync takes the
  discard-and-restart escape hatch `caps/snap.md` permits, rather than
  recovering.
- **Reconstruction does not fan out across accounts**, where snap/1's storage
  insert runs them through rayon. The per-contract costs it used to pay (a write
  transaction each, and a root-down descent per slot) are gone, but the account
  loop itself is still serial. Sharding it needs a range-bounded account
  iterator on `FlatState`; the underlying RocksDB handle is `Sync` and already
  hands out independent iterators, so nothing structural blocks it. Unmeasured
  at mainnet scale, which is the reason it has not been done.
- **`STORAGE_BATCH_SIZE` is a fixed count of 300 accounts** against a 512 KB
  response cap, where geth sizes the batch by response budget
  (`storageSets := cap / 1024`). Not currently implicated in truncation, but
  untested under real load.
- **`ChunkWriter::is_full` sizes buffers with `size_of::<AccountState>()`**,
  which under-counts the RLP. Inherited from snap/1.
- **No hive snap/2 simulator.** `test/tests/p2p/` has no multi-node RLPx
  harness, and the existing hive devp2p snap simulator calls the external Go
  `devp2p` CLI. The devnet harness covers the same ground manually.
- **Legacy-SELFDESTRUCT networks are out of scope**, per the applier's
  correctness premise above.
