### Stateless (zkEVM) Amsterdam+ EF tests skipped

**Where:** `tooling/ef_tests/blockchain/test_runner.rs` — `parse_and_execute` skips
fixtures with `network >= Fork::Amsterdam` when running with a stateless backend.
Affects `make test-stateless` (the `vectors_zkevm/` run); `make test-levm` is
unaffected.

**Why:** The stateless run uses the `tests-zkevm@v0.5.0` bundle, filled against
`glamsterdam-devnet` v6.1.0, which predeploys the EIP-8282 builder deposit/exit
contracts at the OLD addresses (`0x0000884d…d9008282` / `0x000014574a…0f008282`).
This client uses the devnet-7 addresses (`0x0000bff4…300d8282` /
`0x000064d6…800e8282`, matching the live `tests-glamsterdam-devnet@v7.2.0` bundle
used by `make test-levm`). Every Amsterdam+ block runs the end-of-block EIP-8282
builder system call; with the new addresses absent from the v0.5.0 bundle, each
stateless Amsterdam+ block fails with
`SystemContractCallFailed("System contract: 0x0000…8282 has no code after deployment")`.
The skip is by fork rather than by test name, since cross-fork directories such as
`for_amsterdam/prague/...` still execute at the Amsterdam fork.

**Removal:** Delete the `skip_stateless_amsterdam` branch in `parse_and_execute`
once a `tests-zkevm` bundle filled with the devnet-7 builder predeploy addresses is
released and `.fixtures_url_zkevm` is bumped to it.

### `eth_syncing` can report a node as synced while it is behind

**Where:** `crates/networking/rpc/eth/client.rs` — the `Syncing` RPC handler and
`resolve_highest_block`.

**Why:** `highestBlock` is meant to be the chain's tip, but nothing the engine
API gives the execution client actually carries it. `forkchoiceUpdated` says
where to go *next*, not where the chain *is*: driving a lagging client forward,
a consensus client sends a head hash for an intermediate block and advances it
in chunks. That hash resolves locally, so it looks like a perfectly good target
— it is simply not the tip.

When the reported target equals the local head, the distance test in the synced
predicate compares a number against itself and is trivially true, so the
`is_synced()` latch alone decides the answer. A node that has fallen behind then
answers `false` — "fully synced".

Measured on a 3-node devnet (2026-08-07): after a restart, a node answered
`false` across 25 samples while more than 20 blocks behind, worst case **47**,
for roughly 5 seconds. In 0 of 463 samples did `highestBlock` come within 2 of
the true chain head; the shortfall ranged 69–190 blocks.

Impact is confined to reporting. Execution, state roots and consensus are
unaffected, and the node catches up normally. It matters for operators who gate
traffic on `eth_syncing` — load balancers, readiness probes, indexers — because
a restarting node can pass the check early and serve stale reads for a few
seconds.

`SyncTarget::{Known, Unknown}` fixes the narrower case where the client has no
target at all (a restart leaves `last_fcu_head` zeroed) by refusing to report
synced against a stand-in number. It does not help here: a staging head is a
genuine `Known`, just a misleading one.

**Removal:** Derive `highestBlock` from peer-advertised heads in the P2P layer,
which is the only source that observes the actual tip — the sync cycle already
sees them. Two attempts to derive it from engine-API signals failed on a devnet;
a third against the same signals would fail the same way. Note also that
recording the target from `engine_newPayload` is actively harmful and was
reverted: during catch-up the payload's number *is* the local head, so it
converts an honest `Unknown` into a misleading `Known`.

### Deep reorgs into a full-synced range are refused

**Where:** `crates/storage/journal.rs` (see its "Batch mode (full sync)" section),
`Store::store_block_updates`, and `compute_reorg_ceiling` in
`crates/blockchain/fork_choice.rs`.

**Why:** Deep-reorg recovery replays reverse diffs from the `STATE_HISTORY`
journal, which is written per block by the normal commit path. Full sync imports
in batches — one trie layer per ~1024 blocks — and the commit path **skips
journaling entirely** in that mode, because per-block reverse diffs are not
produced there. So a range imported by full sync has no journal coverage, and a
reorg whose pivot falls inside it cannot be reconstructed.

This is a bounded limitation rather than a latent fault, and the guard is
already correct: `compute_reorg_ceiling` derives its journal reach from
`Store::lowest_state_history_block_number`, so an uncovered range simply does
not extend the reach. The ceiling falls back to layer-cache retention and a
deeper forkchoice update is refused with `-38006 TooDeepReorg`. The node
declines the reorg; it does not attempt one it cannot complete, and it does not
accept incorrect state. Pinned by `journal_skipped_in_batch_mode`.

Reorg support becomes available for blocks imported after the node transitions
to normal block-by-block execution, and the covered window is
`[finalized + 1, cache_edge]` — journal entries at or below each new finalized
block are pruned on every forkchoice update that advances finality.

Not specific to any state commitment: this predates the EIP-8297 binary trie and
applies identically to the Merkle-Patricia trie. The binary-trie journal (format
v2) inherits the same boundary.

**Removal:** Journal during batch import as well, which means producing
per-block reverse diffs on a path deliberately built to avoid per-block work —
a real cost trade, not an oversight. Worth revisiting only if operators hit
refused reorgs shortly after a full sync in practice.
