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

### Chain-progress logging does not replace `eth_syncing`, and does not try to

**Where:** `crates/blockchain/health.rs`, spawned from `init_l1` in
`cmd/ethrex/initializers.rs`; refusals reported from
`crates/networking/rpc/engine/fork_choice.rs`.

A node that has stopped advancing now says so in the log: `chain_stalled` at
`ERROR`, repeated once a minute, carrying the head it stopped at, how long it
has been still, how many forkchoice updates were declined, and which refusal
declined them. A frozen head with nothing declined is `chain_idle` at `INFO`,
which states the same facts and renders no verdict — a devnet between slots and
a node with no peers both land there and neither is halted.

This deliberately does **not** answer "is this node healthy". It answers "is the
head moving, and if not, is the node declining to move it". The distinction
matters because the `eth_syncing` issue above is what happens when a status
signal renders a verdict it cannot back: two fixes passed their unit tests and
were refuted by the devnet, because the tests supplied a `highestBlock` that
production almost never has.

Two consequences worth knowing before reading the lines:

- **`chain_idle` is not a health check.** It cannot distinguish a chain with
  nothing to do from a chain whose *producers* have stopped. It reports the
  facts it has and leaves the conclusion to the reader.
- **A stall on a node that never completed a sync is `WARN`, not `ERROR`**, and
  an initial sync's own `SYNCING` answer is not counted as a refusal at all
  (`countable_refusal`). Otherwise a mainnet sync would print a stall line once
  a minute for days, which is how real warnings get filtered out.

**Removal:** nothing to remove. Recorded here so the next person adding a status
signal on this branch reads the `eth_syncing` entry first.

### Snap sync cannot be exercised on a local network, and silently full-syncs instead

**Where:** `MIN_FULL_BLOCKS` in `crates/networking/p2p/snap/constants.rs`, gating
`download_headers_to_sync_head` in `crates/networking/p2p/sync/snap_sync.rs` and
`sync_cycle` in `crates/networking/p2p/sync.rs`.

**What:** A node whose sync head is below `MIN_FULL_BLOCKS` (10 000) falls back to
full sync, whatever `--syncmode` says. The threshold is right for its stated
purpose — after a state download, execute at least that many recent blocks so
recent execution history exists — and 10 000 blocks is about 33 hours of
mainnet. But a kurtosis devnet is a few hundred blocks, three orders of
magnitude short, so on any local network that branch is taken unconditionally.

**Why it matters more than it looks:** the fallback is **silent, and invisible
from the outside**. A node that full-synced ends up at the same head with the
same state root as one that snap-synced. Distinguishing them requires asking for
something only a replaying node has — `eth_getBalance` at a pre-pivot block
answers on a full-syncing node and errors on a state-downloading one. Any local
validation of snap sync that does not make that check is validating full sync
wearing a snap-sync flag.

**Scope:** this gate sits upstream of both legacy MPT snap and `pbtsnap/1`, so it
applies to snap sync generally and is not specific to the binary tree. It is not
a binary-tree regression: legacy snap sync has never been live-testable on a
devnet either.

**Workaround:** `MIN_FULL_BLOCKS` is overridable by environment variable under
the `sync-test` feature, mirroring `EXECUTE_BATCH_SIZE`. A release build has no
override and behaves exactly as before. `fixtures/networks/binary-tree-devnet.yaml`
documents the late-join recipe that uses it.

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

**Related:** a journal *format* bump reduces reach the same way and for the same
reason. The decoder refuses entries written by another version, so on the first
start after an upgrade `Store::from_backend` drains the stale ones; until
finality advances past the drained range, reorg reach is the layer cache alone
and deeper forkchoice updates are refused with `-38006` rather than attempted.
An `info!` line names the drained range and the versions involved. That drain is
what keeps the floor honest — left in place, the stale entries would advertise
reach the node cannot deliver and the reorg would fail mid-flight with
`StateNotReachable` instead of being declined up front.

### The witness generators' withdrawal replay is redundant, not untested

**Where:** `crates/blockchain/binary_witness.rs` —
`Blockchain::generate_binary_witness_for_blocks`, and
`crates/blockchain/blockchain.rs` — both `block.body.withdrawals` branches of the
MPT generator's read replay.

**History.** This entry previously read "`debug_executionWitnessV2` withdrawal
reads are untested", on the premise that withdrawal recipients are credited
outside the EVM and so the `DatabaseLogger` never records them as accessed
accounts — which would make the hand-touching the only thing putting their nodes
in a witness. The chains those suites built all carried empty withdrawal lists,
so nothing tested it either way.

**What was actually found.** Chains that pay withdrawals now exist
(`build_boundary_chains_paying_withdrawals`), and with them the mutation was
re-run: deleting the branch in the binary generator, and deleting *both*
branches in the MPT generator, still fails no test — including tests written
specifically to catch it, which check that a withdrawal-paying block's V2
witness re-executes to the committed binary root and that an MPT witness
carries each recipient's account leaf.

The premise is false. `LEVM::process_withdrawals` credits through
`GeneralizedDatabase::get_account_mut`, which calls `load_account`, which on a
cache miss falls through to `self.store.get_account_state(address)` — and during
witness generation that store *is* the `DatabaseLogger`, whose
`get_account_state` records the address in `state_accessed`
(`crates/vm/levm/src/db/gen_db.rs:345`, `crates/vm/backends/levm/db.rs:34`). A
recipient already in the cache was faulted in, and recorded, earlier. So the
recipients are in `state_accessed` by the time either generator replays it, and
the explicit branch re-reads accounts the replay above it already covered. In
the binary generator it is doubly redundant: the `pbt_state::apply_account_updates`
call immediately below walks the same paths to write the credited balances.

**Status:** the branches are kept as defensive redundancy — the same reasoning
`DatabaseLogger::has_storage` carries in its own comment, that the pairing they
rely on could be loosened later. They cost one trie read per withdrawal.

**Removal:** this is no longer a coverage gap, so there is nothing to close by
writing a test — a test that fails when the branch is deleted cannot be written
while the logger records the recipients anyway. Delete the entry if the branches
are ever removed, or if the recipients stop being faulted through the logger, in
which case they become load-bearing and
`a_v2_witness_over_a_block_paying_withdrawals_re_executes` and
`an_mpt_witness_over_a_block_paying_withdrawals_carries_the_recipients_paths`
become the tests that catch it.

### An L2 genesis may set `binaryTreeTime`, and nothing rejects it

**Where:** `cmd/ethrex/l2/initializers.rs:220-228` (`init_l2` calls the same
`Network::get_genesis()` as L1), against `crates/common/config/networks.rs:104-121`.

**What:** `ChainConfig::binary_tree_time` (`crates/common/types/genesis.rs:299`)
is an ordinary genesis field, and the L2 node loads its genesis through exactly
the same `--network <path>` mechanism as L1. So
`ChainConfig::is_binary_tree_active(ts)` returning true on an L2 chain is
reachable purely by operator configuration, and no L2 startup path rejects,
strips, or warns about it. L1 startup runs `resolve_binary_tree_time`,
`validate_sync_mode`, and `validate_genesis_binary_tree_embeddable`
(`cmd/ethrex/initializers.rs:869-896`); `init_l2` runs none of them, and
`ethrex l2 --experimental.binary-tree-delay=N` parses and is then ignored.

Meanwhile `grep -rn 'binary_tree\|binaryTree' crates/l2/` returns nothing: the
entire L2 stack — sequencer, committer, prover guest — is written under the
assumption that the commitment is never active. The assumption is asserted in
prose only (`crates/blockchain/blockchain.rs:2598-2600`, `:2362-2366`).

**Why it is not currently a wrong-answer bug:** no shipped config sets it —
`grep -rl binaryTreeTime --include='*.json'` over the repo returns zero files,
including `fixtures/genesis/l2.json` — and the two L2 witness call sites
(`crates/l2/sequencer/l1_committer.rs:1171`,
`crates/l2/networking/rpc/l2/execution_witness.rs:74`) now reach the generator
guard added in `Blockchain::ensure_mpt_witnessable`, so they refuse rather than
return an MPT witness for a binary root. On a chain that does not schedule the
commitment the guard is inert, which is why L2 behaviour is unchanged.

**Removal:** either reject `binaryTreeTime` in `init_l2` with a clear error, or
give the L2 stack real EIP-8297 support. Refusing at startup is the smaller of
the two and matches what L1 already does for other unsupported combinations.

### A same-block CREATE2 still sees a destroyed account's pre-block storage

**Where:** `GeneralizedDatabase::get_state_transitions` and its `removed`
computation (`crates/vm/levm/src/db/gen_db.rs:585`), and the
`clear_storage_of_emptied_account` call in `SELFDESTRUCT`
(`crates/vm/levm/src/opcode_handlers/system.rs:821`).

**What:** an EIP-161 clear runs inside LEVM, but the removal reaches the trie
only through the block's account updates, so a CREATE2 in a *later transaction
of the same block* still sees the account's pre-block `has_storage`. EELS
destroys within the transaction and would let that CREATE2 proceed.

**Why it is not currently a wrong-answer bug in practice:** no fixture covers
it. Both PR-#3207 destruction tests put the CREATE2 in a later block, and both
now pass (see the EIP-8038 entry below for the suite numbers).

**Removal:** thread the destruction through the in-flight transaction state
rather than only through the end-of-block account updates, and fill a fixture
that puts the CREATE2 in the same block.

### Amsterdam EIP-2780/EIP-8038 has no CI coverage at all

**Status of the drift itself: CLOSED.** ethrex had implemented EIP-8038 as
merged in `ethereum/EIPs#11802`; the EIP was later revised and ethrex was not
resynced. Seven constants and one intrinsic-cost branch were wrong. Both are
fixed — `crates/vm/levm/src/gas_cost.rs` now mirrors
`src/ethereum/forks/amsterdam/vm/gas.py::GasCosts`, with `CALL_VALUE`,
`CREATE_ACCESS`, the two access-list entries and `REFUND_STORAGE_CLEAR` written
as the spec's derivations rather than as literals, so the next repricing moves
one number. `recipient_regular_gas` no longer charges a creation transaction
for the ether it carries (EELS reaches that charge only via
`elif not is_self_transfer`, which a create never enters).

Measured against a fill of the PR #3207 tracking branch
(`ethereum/execution-specs:projects/binary-trie` @ `9dffd419`), on 2026-08-12:

| suite | before | after |
|---|---|---|
| `for_binarytree` (56 `blockchain_tests`) | 8 passed / 48 failed | **56 / 0** |
| Amsterdam `eip2780` from the same revision (59) | 27 passed / 32 failed | **59 / 0** |

The second row is the control that settles attribution: plain Amsterdam tests
from the same spec revision were failing too, so this was never a binary-trie
problem. `transactions.py` is byte-identical between `forks/amsterdam` and
`forks/binary_tree`, so the binary-trie fork changes no gas rule at all.

**What is still open: CI cannot catch the next drift.** The pinned bundle is
`tests-glamsterdam-devnet@v7.2.0`, whose `for_amsterdam/` tree holds only
`eip7928_block_level_access_lists` and `eip8025_optional_proofs` — no EIP-2780
and no EIP-8038 tests exist in it. That is why this drift went unnoticed for as
long as it did, and why nothing in `make test-levm` would notice it coming back.
Neither of the two suites in the table above is wired into CI; both were run
from a scratch fill.

**Removal:** re-pin `tooling/ef_tests/.fixtures_url_amsterdam` to a bundle that
actually carries `eip2780`/`eip8037`/`eip8038` fixtures once EEST publishes one,
or vendor the fill recipe below into CI. Until then the LEVM unit tests
(`test/tests/levm/eip8038_tests.rs`, `eip2780_tests.rs`, `eip8037_tests.rs`) are
the only guard, and they pin literals taken from the spec by hand.

**Reproducing.** execution-specs is now a monorepo with EEST vendored at
`packages/testing/src/execution_testing/`, so no second clone is needed; the
repo ships the recipe at `Justfile:163` (`binary-trie-fork`). `BinaryTree` is
`deployed=False`, so pass `--fork BinaryTree` and omit `--until`. Fixtures carry
`"network": "BinaryTree"`, which the existing `fork.rs` wiring already matches —
no ethrex change is needed to consume them. To run a filled tree, copy it under
`tooling/ef_tests/blockchain/vectors/eest/` and, from
`tooling/ef_tests/blockchain/` (it is **not** a workspace member), run
`cargo test --profile release-fast --test all -- <path-substring>`.

### `eth_getProofV2`'s response format is an ethrex dialect, not a standard

**Where:** `crates/networking/rpc/types/binary_account_proof.rs`, served by
`GetProofV2Request` in `crates/networking/rpc/eth/account.rs`.

**What:** Past the EIP-8297 activation `eth_getProof` refuses — the binary trie
has no account trie, no per-account storage trie, and therefore nothing its
MPT-shaped response can describe. `eth_getProofV2` serves the binary shape
instead, under its own name and carrying a `proofFormat` discriminator
(`ethrex-eip8297-walk-v1`). Each of the account's header-stem keys and each
requested storage slot gets its own walk proof, verifiable against the header's
`state_root` through `ethrex_binary_trie::trie::verify_walk`.

**Why it is a known issue:** there is no standard to conform to. EIP-8297 says
nothing about `eth_getProof`; EIP-8347, the companion migration EIP, names the
method only to defer it to a separate, unwritten spec; go-ethereum's
`trie/bintrie` `Prove` is `panic("not implemented")` on master; Erigon's
binary-trie branch refuses. So this format is one client's shape, and
a consumer that assumes it is portable is assuming something that is not true
today.

**What it deliberately does not do:** it does not deduplicate the shared stem
between the three account walks and the slots below 64, and it carries no
`storageHash`. Both are reversible choices — the redundancy is what makes every
entry independently checkable by the existing per-key verifier rather than by
new multiproof machinery, and there is no per-account storage root in the design
for a `storageHash` to name.

**Removal:** when execution-apis settles a shape, add it under a new
`proofFormat` string (and, if it lands in `eth_getProof` itself, replace that
method's refusal). Nothing about this method constrains that: it is separately
named precisely so the standard method's schema stayed free.
