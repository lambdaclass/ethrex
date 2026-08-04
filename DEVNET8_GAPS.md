# glamsterdam-devnet-8 alignment gaps

Audit of branch `glamsterdam-devnet-8` (rebased on `origin/main`) against
<https://notes.ethereum.org/@ethpandaops/glamsterdam-devnet-8>.

Uncommitted working notes. Do not commit.

## Status

Upstream state as of 2026-07-31: `devnets/glamsterdam/8` HEAD `5c7a446` (2026-07-27),
2 commits ahead of / 32 behind `forks/amsterdam`.

| Gap | State |
|---|---|
| 1 — EIP-2780 repricing | **done** |
| 2 — EIP-8070 Engine half | **done**: PR #6776 merged into the branch; FCU v4 `custodyColumns` and `engine_getBlobsV3`/`V4` all served |
| 3 — callTracer 2D gas | **done** |
| 4 — BAL getter semantics | **done** |
| 5 — fixture pin | blocked on upstream `tests-glamsterdam-devnet@v8.0.0` |
| 6 — EIP-8038 access-list repricing 3000 → 2900 | **done ahead of upstream**: EELS PR #3271 approved but not yet merged |
| 7 — `eth_sendRawTransaction` error codes | open, but **general conformance debt**, not reachable from a devnet-8 run |
| 8 — `stateGasTracer` | open, devnet-8 scope but a SHOULD with no fixtures |

## Already aligned

- EIPs 7610, 7708, 7778, 7843, 7928, 7954 (64 KiB), 7976 (floor 16), 7981, 8024, 8246, 8282.
- EIP-8037: `cost_per_state_byte() = 1530`, source-based refunds.
- EIP-8038: all values match the current EIP (3000/3000/8000/10000/11000/12480,
  `CALL_VALUE = 10300`, EXT* second-read `WARM_ACCESS`). The access-list intrinsic
  surcharge is Gap 6.
- EIP-8037 flat 2D inclusion gate (pinned by execution-specs#3245): the block gate
  compares `min(TX_MAX_GAS_LIMIT, tx.gas)` against the remaining execution budget in
  full, with no credit for the top-frame state charge and no intrinsic subtraction.
  `check_2d_gas_allowance` in `crates/vm/backends/levm/mod.rs` does exactly this.
- EIP-7928 devnet-8 delta (EIPs#11902): recipient excluded when a runtime halt precedes
  the recipient load. Implemented in `crates/vm/backends/levm/mod.rs`.
- EIP-7997: no client action required. The EIP is a genesis/state requirement and
  explicitly forbids a fork-boundary existence check; ethrex performs none.
- EIP-7975 (eth/70) and EIP-8159 (eth/71) negotiated in `SUPPORTED_ETH_CAPABILITIES`.
- discv5-only: `--p2p.discv4 false` matches the spec sheet's flag table; the `eth` forkid
  ENR entry is refreshed when the chain crosses a fork. ethrex implements no DNS
  discovery, so there is no second silent fallback to disable.
- devnet-7 regression list: `debug_traceBlockByHash`, callTracer/`debug_traceCall` geth
  alignment, and `engine_getPayloadBodies*V2` are all present.

CL-only (7688, 7732, 8045, 8061) and informational (7904) items are out of scope.

## Open upstream PRs screened

Of the open PRs against `forks/amsterdam`, only #3271 (Gap 6) is a devnet-8 EL
consensus delta. The rest are out of scope or non-behavioural:

- Not in the devnet-8 EIP set: #3009 (EIP-8268), #2770 (EIP-8237), #2619 (EIP-7709),
  #3192 (EIP-8141 preparation), #3113 (bogota), #3114 (frame tx).
- Refactors and typing with no semantic change: #3121 (`ExecutionGas`/`StateGas`
  wrappers), #2700, #2714, #2384 and #2373 (py-ecc replacement).
- Tests and tooling only: #3264, #3270, #3269, #3273, #3266, #3265, #3203, #2933.
- #2898 pins the EIP-8037 `max(intrinsic, calldata_floor) > TX_MAX_GAS_LIMIT`
  rejection, which ethrex already implements in `default_hook::validate_min_gas_limit`.

Two open PRs are worth re-checking if they leave draft, since both touch validation
order or decoding strictness rather than gas numbers: #3056 (moves the priority-fee
check into `validate_transaction`, which can reclassify an exception) and #3186
(decode withdrawal amount as `uint64` per EIP-4895).

## Gap 7 — `eth_sendRawTransaction` error codes (general, not devnet-8)

Not a Glamsterdam item and not reachable from a devnet-8 release run. The codes are not
fork-gated, and they are exercised by hive `ethereum/rpc-compat` (test chain: osaka +
bpo1/bpo2) rather than `eels/consume-engine` or `eels/build-block`, which are the only
sims the devnet targets in the `Makefile` wire up. Tracked here because it was found
during the devnet-8 sweep; schedule it independently.


`src/eth/submit.yaml` binds `eth_sendRawTransaction` to the `GasErrors`,
`ExecutionErrors` and `TxPoolErrors` groups, merged in execution-apis#650. Those codes
are already normative.

`impl From<MempoolError> for RpcErr` in `crates/networking/rpc/utils.rs` collapses every
mempool rejection except a store error into `RpcErr::BadParams`, so all of them return
`-32602`:

| Rejection | ethrex | Spec |
|---|---|---|
| Nonce for account too low | -32602 | 1 |
| Transaction intrinsic gas cost above gas limit | -32602 | 800 |
| Transaction gas limit exceeded (block limit) | -32602 | 803 |
| Transaction priority fee above gas fee | -32602 | 804 |
| Transaction intrinsic gas overflow | -32602 | 805 |
| Insufficient balance for tx cost | -32602 | 809 |
| Already-known transaction | -32602 | 1000 |
| Sender is a contract account (EIP-3607) | -32602 | 1001 |

Fix: give `RpcErr` a variant carrying an explicit catalog code and map each
`MempoolError` variant to it, rather than the current catch-all arm.

execution-apis#784 turns these into hive `rpc-compat` fixtures (and adds 1002,
replacement-underpriced), so this becomes a visible test failure once it merges.

## Gap 8 — `stateGasTracer` not implemented (devnet-8, SHOULD, no fixtures)

Amsterdam-specific, but a SHOULD, and execution-apis#852 adds only `src/debug/*.yaml` and
`src/schemas/*.yaml` — no `.io` fixtures — so nothing fails on it today.


execution-apis#852 defines a `stateGasTracer` named tracer returning `gasUsed`,
`regularGasUsed`, `stateGasUsed` and `gasRefund` per transaction, and says clients
supporting Amsterdam SHOULD implement it. `TracerType` in
`crates/networking/rpc/tracing.rs` offers only `callTracer`, `prestateTracer` and
`opcodeTracer`.

All four values already exist on `ExecutionReport` — the same ones Gap 3 stamps onto the
callTracer top frame — so this is a name-dispatch plus serialization shape, not new
accounting.

## Execution-apis items verified aligned

- `-38005: Unsupported fork` gating is implemented across `engine_newPayload*`,
  `engine_getPayload*`, `engine_forkchoiceUpdated*` and `engine_getBlobs*`.
- execution-apis#856 pins the custody/cell bitarray layout as SSZ
  `BitVector[CELLS_PER_EXT_BLOB]`, column `i` at bit `i % 8` of byte `i / 8`.
  `parse_indices_bitarray` in `crates/networking/rpc/engine/blobs.rs` already uses
  exactly that layout.
- #851's `-32001` / `4444` / `null` matrix matches Gap 4.
- #852's required top-frame fields (`regularGasUsed`, `stateGasUsed`, `gasRefund`,
  Amsterdam-gated) match Gap 3.
- EIP-7843 `slotNumber` is serialized on the header and in payload types (#806, open).

## Local test coverage for tracker items

The fixture pin cannot move to v8.0.0 yet, so the tracker's coverage PRs are pinned in
Rust instead where ethrex had no equivalent test:

- EIP-8037 flat 2D inclusion gate (#3245) — `test/tests/blockchain/eip8037_block_gate_tests.rs`.
  `check_2d_gas_allowance` had no test at all. Covers exact fit, one-above, no credit
  for the top-frame state charge, the execution-dimension `TX_MAX_GAS_LIMIT` clamp, the
  uncapped state dimension, independent dimensions, and exhausted budgets.
- EIP-8037 per-tx execution-gas cap (#2898) — two tests in
  `test/tests/levm/eip8037_tests.rs`: a calldata floor above `TX_MAX_GAS_LIMIT` is
  rejected even when the gas limit funds it, with a control just under the cap.

Already covered elsewhere: EIP-8037 spill refunds (#3158) in
`eip8037_reservoir_tests.rs`, EIP-7778 pre-refund block accounting (#2932) in
`eip7778_tests.rs`, invalid-BAL content canonicality (#3170) in
`bal_content_validation_tests.rs`, and the EIP-8038 repricing itself in
`eip8038_tests.rs`.

## Gap 1 — EIP-2780 devnet-8 repricing (consensus divergence)

EIPs#11997 and execution-specs#3214 fold the transfer log cost into `TX_VALUE_COST`:

- `TX_VALUE_COST` = 6000, single constant; `TRANSFER_LOG_COST` deleted.
- A create transaction with `value > 0` gets **no** value charge; the recipient balance
  write is already covered by `CREATE_ACCESS`.

ethrex carries the devnet-7 split (4244 + 1756) and still adds `TRANSFER_LOG_COST` on
creates, so a create-with-value transaction charges 24,756 instead of 23,000. The same
error propagates into the EIP-7623/7976 calldata-floor anchor, which shares
`recipient_regular_gas`.

Done in `crates/vm/levm/src/gas_cost.rs`: `TX_VALUE_COST_AMSTERDAM = 6000`,
`TRANSFER_LOG_COST_AMSTERDAM` removed, and `recipient_regular_gas` returns
`CREATE_ACCESS` alone for creates and `cold_account_access + TX_VALUE_COST` for
non-self calls. `test/tests/levm/eip2780_tests.rs` updated: create-with-value is now
23,000 and an ETH transfer still totals 21,000.

## Gap 2 — EIP-8070 Engine API half (now required)

The spec sheet promotes the Engine half to required: `engine_getBlobsV3` *and* `V4` both
served, FCU v4 `custodyColumns`.

`parse_v4` in `crates/networking/rpc/engine/fork_choice.rs` rejects any call carrying a
third parameter, so a CL that sends `custodyColumns` — **even as `null`** — gets
`Invalid params` and forkchoice fails outright. `engine_getBlobsV4` does not exist.

Done: `parse_v4` accepts 1–3 parameters and validates `custodyColumns` as null or a
16-byte `DATA` value (`-32602` otherwise), then ignores it — the correct behaviour for a
node that replicates every blob rather than sampling by custody. Forkchoice no longer
fails against a devnet-8 CL. Covered by three tests in
`test/tests/rpc/fork_choice_tests.rs`.

PR #6776 (eth/72 sparse blobpool) is merged into the branch, so `engine_getBlobsV4` and
its `BlobCellsAndProofsV1` response live in `crates/networking/rpc/engine/blobs.rs`.
eth/72 cell exchange itself stays optional for devnet-8.

## Gap 3 — execution-apis PR-852: callTracer two-dimensional gas (required)

`CallTraceFrame` in `crates/common/tracing.rs` has no `regularGasUsed`, `stateGasUsed`,
or `gasRefund`. The spec requires them on the top-level frame for Amsterdam+ blocks and
forbids them before the fork; sub-frames should omit them.

Done: `CallTraceFrame` gained the three optional fields, and `run_call_trace` stamps them
on the top-level frame from the `ExecutionReport` when the fork is Amsterdam or later.
Pre-fork they stay `None` and are omitted from the JSON, per the spec's MUST NOT.
Sub-frames are never stamped.

## Gap 4 — execution-apis PR-851: block access list getter semantics (required)

- `debug_getRawBlockAccessList` was not implemented at all. It must accept a block number,
  tag, **or hash**, and return the RLP-encoded BAL (`0xc0` for a block with no accesses).
- `eth_getBlockAccessList` returned all six account fields with empty lists correctly, but
  returned `null` for pre-Amsterdam blocks where the spec mandates
  `-32001: Resource not found`, and never returned `4444: Pruned history unavailable`.

Done: both getters now share one resolver in
`crates/networking/rpc/eth/block_access_list.rs`. Unknown block or `pending` is `null` for
the JSON getter and `-32001` for the raw getter; a pre-Amsterdam block is `-32001`; an
Amsterdam+ block whose access list can neither be served nor reconstructed is `4444`.
`RpcErr::ResourceNotFound` (-32001) and `RpcErr::PrunedHistoryUnavailable` (4444) were
added to the error mapping, and `debug_getRawBlockAccessList` is registered in
`map_debug_requests`. Four tests in `test/tests/rpc/block_access_list_tests.rs`.

## Gap 5 — fixtures still pinned to devnet-7 (blocked upstream)

`Makefile` uses `AMSTERDAM_FIXTURES_BRANCH = devnets/glamsterdam/7` and
`tests-glamsterdam-devnet@v7.2.0`.

The devnet-8 target is `tests-glamsterdam-devnet@v8.0.0` on `devnets/glamsterdam/8`.
The release is not cut (tracker: execution-specs#3167) and is gated on the final
EIP-8037/8038 repricing numbers, so the pin cannot move yet.

The branch does exist (HEAD `5c7a446`, 2026-07-27). Diffing its
`src/ethereum/forks/amsterdam/` against `tests-glamsterdam-devnet@v7.2.1` gives exactly
one behavioural EL change:

- EIP-2780: `TX_VALUE_COST` 4244+1756 → 6000, and create-with-value loses its value
  charge. This is Gap 1, now implemented.

Everything else on the branch is the EIP-8037 "regular gas" → "execution gas" rename
(EIPs#11998 — `RegularGas` → `ExecutionGas`, `allocate_execution_gas` →
`allocate_evm_gas`, `ExecutionGasAllocation` → `EvmGasAllocation`, docstrings) plus test
additions. No semantics move with it, so ethrex's internal `regular_gas` naming needs no
change; note that execution-apis PR-852 still spells the callTracer field
`regularGasUsed`, so the wire name is unaffected by the rename.

The branch is also 5 commits *behind* v7.2.1, so it is not a strict superset yet, and it
has since fallen 32 commits behind `forks/amsterdam` — both of its own commits landed
there as squashes (#3214, #3238), so it will most likely be re-cut from
`forks/amsterdam` when the release is built.

Nothing in the 32 commits `forks/amsterdam` carries beyond the branch point changes EL
behaviour: two renames (#3238, #3263), a spec-internal state-interface refactor (#3218),
and test additions. #3245 pins the flat 2D inclusion gate, which ethrex already matches.

Status: **the only merged EL delta is covered; the pin itself is blocked on the upstream
release, which is gated on Gap 6.**

## Gap 6 — EIP-8038 access-list intrinsic surcharge 3000 → 2900

The last open scope item on the v8.0.0 tracker. `TX_ACCESS_LIST_ADDRESS` and
`TX_ACCESS_LIST_STORAGE_KEY` become `COLD_*_ACCESS - WARM_ACCESS` = 3000 - 100 = 2900, so
prepaying an access-list entry is gas neutral against the cold access it replaces instead
of costing 100 more. EIP-2930's 100 discount is deliberately not restored.

Done in `crates/vm/levm/src/gas_cost.rs`: `ACCESS_LIST_ADDRESS_COST_AMSTERDAM` and
`ACCESS_LIST_STORAGE_KEY_COST_AMSTERDAM` are now derived as
`COLD_*_ACCESS_AMSTERDAM - WARM_ADDRESS_ACCESS_COST` rather than literal 3000.
`test/tests/levm/eip8038_tests.rs` updated: the selector test and both intrinsic-delta
tests expect 2900. `BAL_ITEM_COST` stays a flat 2000 — EIP-7928's per-item charge is not
derived from the access-list constants.

Implemented ahead of the upstream merge: EELS PR #3271 is approved, mergeable and clean
but not yet merged, and no EIPs PR exists for the text yet (tracker says `#TBD`). This is
a deliberate divergence from `tests-glamsterdam-devnet@v7.2.0`, which bakes 3000 into 952
non-empty access lists across 67 fixture files. Revert these two constants if the number
moves before the release.
