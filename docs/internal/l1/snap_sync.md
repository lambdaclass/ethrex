# Snap sync internals

## snap/2 — BAL-based state healing (EIP-8189)

snap/2 replaces the iterative `GetTrieNodes` / `TrieNodes` round-trips of the
healing phase with a single `BlockAccessLists` exchange. Once the bulk
download has settled at a pivot, the syncing node downloads the
`BlockAccessList` for each block between that pivot and the latest pivot,
verifies each BAL against its header commitment, and applies the diffs
locally to advance the trie.

The wire spec is documented in
[EIP-8189](https://eips.ethereum.org/EIPS/eip-8189) and depends on EIP-7928
for the `block_access_list_hash` header field.

## Capability negotiation

What a connection advertises depends on sync state, via
`advertised_snap_capabilities(is_synced)`. EIP-8189 ("Backwards Compatibility")
says a node synchronizing data should use one snap version for state sync and
serve both only once synchronization is complete. ethrex's state sync reconciles
the trie with `GetTrieNodes`, which snap/2 removes, so a node that offered
snap/2 while still syncing would negotiate away its only healing mechanism and
then find no peer able to serve it — `heal_state_trie` would re-queue its batch
and spin. So an unsynced node advertises `snap/1` alone and a synced one
advertises both.

The consequence is that BAL replay is currently reachable only on the serving
side: state sync always runs unsynced, so it always negotiates snap/1. The
client half becomes live once the download-loop catch-up (EIP-8189 step 4) lands
and healing is no longer required.

The Hello exchange picks the highest snap version common to the peer's list and
the set this connection advertised — matching against the advertised set rather
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
remains the full advertised list because `admin_peers` reports it.

## Wire format

`Snap2GetBlockAccessLists` carries `[id, [hashes...], response_bytes]`.
`response_bytes` is a soft cap; `0` means "use the default" (2 MiB).

`Snap2BlockAccessLists` carries `[id, [entries...]]` with one entry per
requested hash, in order. devp2p `caps/snap.md` types the response as
`[reqID: P, bals: [bal1: B, bal2: B, ...]]`, so every entry is an RLP **byte
string**, matching how snap carries all of its other payloads (`node1: B`,
`code1: B`, `accBody: B`):

- present  → a byte string whose content is the RLP-encoded `BlockAccessList`
- absent   → the RLP empty string `0x80`

This is deliberately not eth/71's `BlockAccessLists` (0x13) shape, which
splices the BAL in as a bare list; the two protocols differ here. Hashing is
unaffected either way, since `keccak256` covers the inner bytes and not the
string header. The codec tests `snap2_bal_entries_are_rlp_byte_strings` and
`snap2_bal_none_uses_0x80_sentinel` lock both halves against regressions.

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

## Server handler

`build_snap2_bal_response` in `rlpx/connection/server.rs` builds the response
from a batched `Store::iter_block_access_lists_by_hashes`. No per-hash header
lookup is needed: BAL storage is gated on the Amsterdam fork, so a stored BAL
implies a post-Amsterdam block and is served directly, while a pre-Amsterdam,
pruned, or unknown block has nothing stored and yields `None`.

The byte budget is tracked via `snap2_entry_encoded_len` (the zero-allocation
`RLPEncode` length plus its byte-string header) and capped at
`min(response_bytes, 2 MiB)`. `response_bytes` is the requester's soft limit
and 2 MiB is the recommendation that applies when it names none; the spec also
lets a responder impose its own QoS limits, so a request can only lower the cap
and never raise it. When
the cap is exceeded the loop breaks, preserving order up to the cutoff and
keeping at least one entry. The handler always returns a response — never
drops the request — and serves orphaned (non-canonical) blocks the same as
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

## BAL replay applier

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

## Driver

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

## Snap-sync integration

BAL replay implements EIP-8189 steps 5 and 7, and runs only in the
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

`SyncDiagnostics` carries five counters bumped by the driver:
`snap2_bal_requests_sent`, `snap2_blocks_replayed`, `snap2_bals_unavailable`,
`snap2_validation_failures`, `snap2_peer_failures`.
