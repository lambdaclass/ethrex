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

### The last two PR-#3207 failures: causes found, both blocked behind the gas drift

**Status.** Both causes are isolated. One is fixed; the other is a *second*
Amsterdam gas drift and is filed with the EIP-8038 entry below rather than fixed
here. Neither fixture passes yet, because **both of them also depend on the
EIP-8038 drift**: the earlier reading that these two were separable from the
other 46 was wrong. With the EIP-8038 constants corrected on top of the fix that
did land, plus the one-line change described below, `for_binarytree` is
**56 passed, 0 failed** (measured 2026-08-12). Without those, the suite stays at
its previous 8 passed / 48 failed.

**What the earlier note here guessed, and what was actually wrong.** The
suspicion recorded here — that `apply_account_update`'s `removed` branch in
`crates/common/types/pbt_state.rs` fails to remove the storage leaves — was
**wrong**. That branch is correct: it removes the whole header stem (basic data,
code hash or delegation indicator, slots 0-63) and the overflow-storage zone,
which is exactly what EELS `destroy_account` does. It was simply never reached.
And the second fixture had nothing to do with storage, or with EIP-6780.

- `state_divergence/create2_after_eip161_clear_of_storage_holding_account`
  (`StateRootMismatch`) — **fixed.** The block-1 SELFDESTRUCT's beneficiary was
  never written. EELS `move_ether` writes both parties through `modify_state`
  whatever the amount, and `modify_state` destroys an account the write leaves
  existing and empty; ethrex's `vm.transfer` returns early on a zero value, and
  `get_state_transitions`' `removed` fires only for an account that *stopped*
  being non-empty. Fixed in `SELFDESTRUCT` (`clear_storage_of_emptied_account`)
  plus the `removed` computation, gated on the header having reached
  `binaryTreeTime` so the MPT keeps the orphaned storage trie that makes it
  answer the other way. Pinned by
  `an_eip161_clear_of_a_storage_only_beneficiary_drops_its_leaves_on_the_binary_trie`
  and `the_same_eip161_clear_leaves_the_mpt_exactly_as_it_was`.

  Block 1 of the fixture now produces the expected state root; block 2 still
  fails, on the EIP-8038 drift alone (it does two cold SSTOREs, so the gas — and
  through it the sender and coinbase balances — moves).

- `account_lifecycle/selfdestruct_same_transaction_leaves_no_account`
  (`ReceiptsRootMismatch`) — **not fixed; see the drift entry below.** It is not
  a state problem and not an EIP-6780 problem. `validate_gas_used` passes, and
  the receipt's logs (two EIP-7708 transfer logs) and status match the fixture
  byte for byte; only `cumulativeGasUsed` moves. See "a value-bearing CREATE
  overpays its intrinsic gas by 1756" below.

**Still open — the same-block window.** The clear runs inside LEVM, but the
removal reaches the trie only through the block's account updates, so a CREATE2
in a *later transaction of the same block* still sees the account's pre-block
`has_storage`. EELS destroys within the transaction and would let that CREATE2
proceed. No fixture covers it: both PR-#3207 tests put the CREATE2 in a later
block.

**Removal:** correct the EIP-8038 constants and the CREATE intrinsic per the
entry below, then re-run the fixtures with the recipe there; the two named tests
should join the other 54.

### Amsterdam gas schedule is behind EIP-8038 — now measured against fixtures

**Where:** `crates/vm/levm/src/gas_cost.rs:213-222` (the "EIP-8038 Amsterdam
values (merged EIPs#11802)" block), and `tooling/ef_tests/.fixtures_url_amsterdam`.

**Why:** ethrex implements EIP-8038 (State Access Gas Cost Increase) as merged in
`ethereum/EIPs#11802`, which its own comment cites. The EIP has since been
revised: cold **account** access rose to 3000, but cold **storage** access stayed
at **2100**. ethrex raised both.

| constant | ethrex | current spec |
|---|---|---|
| `COLD_STORAGE_ACCESS` | 3000 | **2100** |
| `ACCESS_LIST_STORAGE_KEY` | 3000 | **2000** (`cold_storage − warm`) |
| `ACCESS_LIST_ADDRESS` | 3000 | **2900** (`cold_account − warm`) |
| `ACCOUNT_WRITE` | 8000 | **9000** |
| `CALL_VALUE` | 10300 | **11300** (`account_write + 2300`) |
| `CREATE_ACCESS` | 11000 | **12000** (`account_write + cold_account`) |
| `STORAGE_CLEAR_REFUND` | 12480 | **11616** |

Really two root changes — `COLD_STORAGE_ACCESS` and `ACCOUNT_WRITE` — plus the
access-list costs now being *derived* as `cold − warm` rather than flattened.
Everything else agrees (`COLD_ACCOUNT_ACCESS` 3000, `STORAGE_WRITE` 10000,
`TX_BASE` 12000, `PER_AUTH_BASE_COST` 7816, the EIP-8037 constants).

The divergence is exactly **900 per distinct cold storage slot**. On a contract
touching two cold slots (`PUSH2/SLOAD/PUSH1/SSTORE/STOP`):

```
spec:   12000 + 3000 + 3 + 2100 + 3 + 2100 = 19206
ethrex: 12000 + 3000 + 3 + 3000 + 3 + 3000 = 21006
```

**Coverage is suspect, but not established.** Running the on-disk vectors
directly (`cargo test --manifest-path tooling/ef_tests/blockchain/Cargo.toml`)
gives 0/1120 on `for_amsterdam`, failing on the genesis-hash assertion. That
looked like "Amsterdam is untested", but the same run gives 196/392 on `cancun`
and 145/469 on `prague` — forks that do not use the EIP-8038 constants at all.
So the local vector tree is stale or partially downloaded, and these numbers
measure that, not CI. `make test-levm` has download/refresh prerequisites that
were not run.

What remains solid is the constant divergence itself, which is a direct
code-vs-spec comparison verified arithmetically and independent of any test
infrastructure. Whether CI currently exercises Amsterdam is **open** and worth
checking with a proper `make test-levm` run before drawing conclusions about how
this went unnoticed.

Found while running the EIP-8297 `BinaryTree` fixtures: 22 of 24 failed on
`GasUsedMismatch`/`ReceiptsRootMismatch`. Filling the *same* tests at `Amsterdam`
reproduced all 35 failing cases with byte-identical error variants and gas
numbers, and the two forks' fills are byte-identical in `gasUsed` and
`receiptTrie`. The binary-tree work is not implicated — its genesis roots match
the spec exactly.

Re-measured 2026-08-12 against a fresh fill of the PR #3207 tracking branch
(`ethereum/execution-specs:projects/binary-trie` @ `9dffd419`), which yields a
wider suite — 56 `blockchain_tests` rather than the 24 above:

| run | passed | failed |
|---|---|---|
| `BinaryTree`, ethrex as-is | 8 | 48 |
| `BinaryTree`, the seven constants aligned | **54** | **2** |
| *Amsterdam* `eip2780` from the same revision, as-is | 27 | 32 |

The third row is the control that settles attribution: plain Amsterdam tests
from the same spec revision fail too, so this is not a binary-trie problem.
`transactions.py` is byte-identical between `forks/amsterdam` and
`forks/binary_tree`, so the binary-trie fork changes no gas rule at all. The
two survivors at 54/56 are the account-destruction cases in the entry above.

Re-measured again after the EIP-161 clear landed, with the seven constants
aligned **and** the CREATE intrinsic corrected (below): **56 passed, 0 failed**.

#### A value-bearing CREATE overpays its intrinsic gas by 1756

A second, independent drift in the same file, found the same way and **not
fixed** — it is a gas-schedule change with a deliberate test pinning the current
behaviour, so it belongs to whoever settles the table above.

**Where:** `recipient_regular_gas` in `crates/vm/levm/src/gas_cost.rs`; the
behaviour is pinned by `test_intrinsic_create_nonzero_value_amsterdam` in
`test/tests/levm/eip2780_tests.rs`.

**What:** EELS `calculate_intrinsic_cost` reaches its value charge only through
`elif not is_self_transfer`, a branch a creation transaction never enters — a
create's recipient charge is `CREATE_ACCESS` and nothing else, however much
ether it carries. ethrex adds `TRANSFER_LOG_COST_AMSTERDAM` (1756, EIP-2780's
split of the spec's flat `TX_VALUE_COST` of 6000) to creates as well. The fix is
to make the value charge conditional on `!is_create`; the existing test then has
to be updated with it.

**How it was measured.** Calling the spec's own `calculate_intrinsic_cost` on
the fixture's transaction returns `execution=24422` = `TX_BASE` 12000 +
`CREATE_ACCESS` 12000 + calldata 420 + initcode 2, with no value component. Then
end to end: the spec's `t8n`, driven with the fixture's own pre-alloc, env and
transaction, reproduces the fixture's `receiptsRoot`
(`0x9cd7944b…`) and `stateRoot` exactly and settles at
`cumulativeGasUsed = 518651`. ethrex, with the seven constants above aligned so
they cannot confound it, gives **520407** — a difference of exactly **1756**.
With ethrex's own constants the two drifts partly cancel and the gap is 656
(`1756 − 1100`), which is what makes this one easy to miss.

**Why the block-level check does not catch it.** EIP-7778 makes
`header.gas_used` `max(regular_gas, state_gas)`, and on a deployment the state
dimension usually dominates — 465120 versus 53019 here — so `validate_gas_used`
passes and the error only reaches the receipt's `cumulativeGasUsed`, and through
it the receipts root. That is why
`account_lifecycle/selfdestruct_same_transaction_leaves_no_account` fails as
`ReceiptsRootMismatch` rather than `GasUsedMismatch`, and why the failure looked
like an EIP-6780 state problem when it is neither.

**Why CI never caught it:** the pinned bundle is `tests-glamsterdam-devnet@v7.2.0`,
whose `for_amsterdam/` tree holds only `eip7928_block_level_access_lists` and
`eip8025_optional_proofs` — no EIP-2780 or EIP-8038 tests exist in it.

**Not established:** which side reflects the ratified EIP-8038. Only that this
execution-specs revision and ethrex disagree. Confirm before changing the
constants — they feed EIP-8037 state-gas accounting.

**Removal:** Resync the seven constants above against
`src/ethereum/forks/amsterdam/vm/gas.py`. Separately, confirm whether CI
exercises Amsterdam at all — if `.fixtures_url_amsterdam` points at a bundle
with no `eip8037`/`eip8038` coverage, re-pin it so the next drift is caught by
CI rather than by accident.

**Reproducing.** execution-specs is now a monorepo with EEST vendored at
`packages/testing/src/execution_testing/`, so no second clone is needed; the
repo ships the recipe at `Justfile:163` (`binary-trie-fork`). `BinaryTree` is
`deployed=False`, so pass `--fork BinaryTree` and omit `--until`. Fixtures carry
`"network": "BinaryTree"`, which the existing `fork.rs` wiring already matches —
no ethrex change is needed to consume them. Note `ef_tests-blockchain` is not a
workspace member: build it from `tooling/ef_tests/blockchain/`.

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
