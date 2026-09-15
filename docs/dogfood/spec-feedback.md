# Dogfooding the FOCIL enforcement spec

Two implementations of `docs/eip-focil-frametx.md`, each started from a client that has
only one half of what the spec assumes, each run as a joiner against the live Hegotá
testnet. This file collects what the spec text failed to say, said ambiguously, or said
wrongly, as found by implementing from it. Entries name the section they are about and
what an implementer needed that was not there. Another agent may review and apply them.

## Setup

The two starting points, and what each already has:

| Base | Head | Has | Lacks |
| --- | --- | --- | --- |
| `focil-devnet-0` | `c28af3278` | EIP-7805 engine surface, inclusion-list builder and Profile 1 validator, the newest glamsterdam-devnet-8 fixes | every frame EIP: the branch's head commit removed EIP-8141 outright, 49 files and 12,988 deletions |
| `frames-devnet-0` | `d587cf9ff` | EIP-8141 at `b75cbe6115`, the mempool prefix simulation and validation observer | EIP-7805 entirely, EIP-8250, EIP-8272 |

`frames-devnet-0` is an ancestor of `hegota-testnet`: the merge that produced the running
chain took it in whole. `focil-devnet-0` is not; it diverged and then deleted frames.

The spec delegates all four prerequisite EIPs by reference and specifies none of them.
Implementing those from their own texts is not a test of this spec, so each leg brings
them in from the branches that already implement them, and implements only what this spec
specifies, the Profile 2 enforcement layer, fresh from the text. That layer is removed from
the prerequisite base first, so that neither leg can read the existing implementation.

Sizing, from merge dry runs:

| Step | Result |
| --- | --- |
| `hegota-testnet` into `focil-devnet-0` at its head | 49 conflicts, 27 of them in `crates/`, most where the head commit deleted a frames file that `hegota-testnet` modified |
| `hegota-testnet` into `frames-devnet-0` | fast-forward, no merge needed |
| delta `frames-devnet-0` to `hegota-testnet` | 62 files in `crates/`, +7,806 / -1,221 |
| delta `focil-devnet-0` to `hegota-testnet` | 90 files in `crates/`, +14,188 / -2,158 |

## Leg 1: FOCIL base (`dogfood-focil`)

### Prerequisite merge

The branch `dogfood-focil` starts at `focil-devnet-0`, reverts its head commit to restore
the frames it had deleted, then merges `hegota-testnet`. Reverting first is what turns 49
conflicts into 20: without it every frames file `hegota-testnet` touched is a delete-versus-
modify conflict. Twenty conflicts remained, six in code. Each resolution follows one rule:
keep this branch's FOCIL layer, which is newer (aligned to tests-focil-devnet@v0.2.0 and the
glamsterdam-devnet-8 fixes), take the frame EIPs and fixture pins from `hegota-testnet`, and
union anything that is a list. These resolutions are recorded for review; none was
discussed before being made.

| File | Resolution | Why |
| --- | --- | --- |
| `crates/blockchain/inclusion_list_builder.rs` | ours | `get_code` provider method; the other side's `classify_code` belongs to the Profile 2 layer being rebuilt |
| `crates/blockchain/inclusion_list_validator.rs` | ours | `TrackedSender` and the EELS-mirroring includability gates; the other side's `IlSenderState`, `SenderCode` and `check_with_profile_2` are the layer being rebuilt |
| `crates/networking/rpc/engine/fork_choice.rs` | ours | `parse_v5` keeps V4's `custodyColumns` third parameter, per the Engine API |
| `crates/networking/rpc/engine/inclusion_list.rs` | ours | `parentHash` is parsed and used, not ignored |
| `crates/networking/rpc/eth/logs.rs` | theirs | `blockHash` filter support; additive, not FOCIL |
| `crates/vm/backends/levm/tracing.rs` | theirs | `#[expect]` over `#[allow]`; lint hygiene only |
| `cmd/ethrex/l2/initializers.rs` | theirs | the mempool config fields the frame mempool needs |
| `test/tests/blockchain/{inclusion_list_builder,inclusion_list_validator,focil}_tests.rs` | ours / ours / union | follow the code choices above; the frames imports in `focil_tests.rs` are kept for the Profile 2 tests that will be rewritten |
| `test/tests/rpc/{fork_choice,inclusion_list_engine}_tests.rs` | ours | custody and `parentHash` tests match the code above; the other side had moved the same custody tests to the end of the file |
| `test/tests/{common,rpc,storage}/mod.rs` | union | module lists |
| `Makefile` | theirs | the ethereum-package revision whose genesis generator deploys the EIP-8282 predeploys the chain needs |
| `tooling/ef_tests/.fixtures_url_amsterdam`, hive `amsterdam.yaml` | theirs | v8.1.4 is newer than v8.1.2; the two pins must move together |
| `tooling/ef_tests/engine/Makefile` | mixed | frames overlay from theirs, FOCIL overlay ours (v0.2.0), clean target unioned |
| `tooling/ef_tests/engine/src/fixture.rs` | merged | fork arm accepts "Bogota", "Hegota" and "Heze" |

What the merge could not express: `hegota-testnet`'s Profile 2 layer arrived through clean
auto-merges in files that did not conflict (`blockchain.rs`, `focil_eligibility.rs`,
`focil_profile2.rs`, the observer). It is removed in the next commit, so the base carries the
four prerequisite EIPs and nothing this spec specifies.

### Stripping the existing Profile 2 layer

Removed from the merged tree, so the base carries prerequisites only:

- `crates/blockchain/focil_eligibility.rs` and `focil_profile2.rs`, with their tests
- the `IlStateProvider::classify_code` method, `SenderCode`, `IlSenderState`,
  `check_with_profile_2`, `Profile2Eligibility` and `IlCheckReport`: the inclusion-list
  builder and validator are this branch's own versions, which use `get_code` and
  `TrackedSender`
- the observer's `FocilVopsSurface`, `CodeBodyBudget`, `Profile2Replay`, the two
  violations they raise, the surface branches in the `SLOAD` and `SSTORE` hooks, and the
  per-frame code charge
- the `profile_2` parameter threaded through `run_frame_validation_prefix`,
  `simulate_frame_validation_prefix` and the backend wrappers, and the `code_budget`
  field on `FrameValidationOutcome`
- the operations documents that describe the implementation
  (`hegota-testnet-{divergences,spec,verification,upgrading}.md`, `hegota-upgrade-merge.md`)

Kept, because the spec's prerequisites need them: the EIP-8141 mempool prefix simulation
and its validation observer, the canonical paymaster exemption, the EIP-8272 verifier-frame
permissions and `check_recent_root_frame`, EIP-8250 keyed nonces, and `inclusionListSatisfied`
on `engine_newPayloadV6` and `engine_forkchoiceUpdatedV5`.

Two adaptations the merge needed that are not about the spec: the inclusion-list builder's
fee arithmetic assumed `u64` fees and the frames line widened them to `U256`; and this
branch's `eth_getRawTransactionByBlock*` used an RLP helper the frames line no longer
implements on `Transaction`, replaced by `encode_canonical_to_vec`. Both are the kind of
seam a FOCIL client meets when it first takes in frame transactions, and neither is
mentioned by any of the EIPs, which is fair: they are implementation history, not protocol.

A frame transaction in an inclusion list is, on this base, simply not a Profile 1
transaction. Whether its omission is excused or judged is exactly what the spec has to make
the implementer add.

### Implementation

Written by an agent that read only `docs/eip-focil-frametx.md` and the pinned prerequisite
texts, with the base code but no access to `hegota-testnet`, its history or its documents.
Four commits on `dogfood-focil`, `d1d6e6c88` to `1ad028402`: the replay policy on the VM
(`Profile2Surface`, per-list `CodeBudget`, code charged at frame entry and at `CALL` and
`EXTCODE` targets), `focil_profile2.rs` (candidacy, `verify_budget_cost`, the two-stage fill,
the two-state check with `S_end` read through the committed root minus withdrawal credits),
one shared entry point for both profiles used by block import and the engine RPC, the
builder's second pass over skipped inclusion-list entries, and three test files (candidacy
and fill units, replay rules, real blocks for cases 1, 2, 3, 7 both ways, 21, pre-fork and the
builder retry). 1,317 tests pass in `ethrex-test`, 39 of them new.

Isolation record: the implementer reported no contact with the excluded documents or
history during the implementation. After all four commits had landed and its report was
delivered, a tooling notification showed it part of this file (the base-stripping section);
it reported the exposure and did not act on it. The implementation predates it.

### The implementation as a joiner (2026-09-15)

The leg 1 node (`dogfood-focil` at `f919aa57b`, release build) was run against the live
chain alongside the leg 2 base node, peered with it over loopback and with the network's
three bootnodes: genesis sync with `--ignore-ws-check`, 12,010 blocks, following the head
with four execution peers and three beacon peers. Over the first 95 blocks it and the
network's first beacon node both saw live, every per-block satisfaction record agrees, all
`satisfied: true`; the node logged no undecided Profile 2 verdict and no warning. Same
caveat as for the base: the live chain carries no unsatisfied block, so this shows the
implementation does not reject what the network accepts, not that it rejects what it
should. That half is the block-level tests (cases 1, 2, 3, 7, 21 and the builder retry).

### Spec feedback

The implementer's log is `leg1-spec-feedback.md` in this directory, 21 entries, verbatim.
The revision of the spec that accompanies this file applies them as follows.

| Entry | Finding | Applied |
| ---: | --- | --- |
| 1 | The Engine API delivers one flat `inclusionListTransactions` array; "per inclusion list" budgets are not computable | Terminology now defines the inclusion list as that array; the fill, the code budget and the Rationale's bound (`2**20` per payload, not `2**24`) follow; case 19 rewritten; griefing note extended |
| 2 | `FORK_TIMESTAMP` is `TBD` | Defaults to EIP-8272's activation unless the chain schedules its own |
| 3 | `S_end` is not a state the client holds after import | Reading through the committed root MUST discount withdrawal credits; an account left empty by the discount is nonexistent per EIP-161 |
| 4 | Pre-execution operations are observable at the activation block | Claim restricted to blocks after activation; the activation-block case stated |
| 5 | No chain id condition for Profile 2 | Folded into candidacy condition 1 |
| 6 | The fill's sum cannot MUST-agree with a mempool reading EIP-8141 rule 6 literally | Downgraded to SHOULD, with the reason; rule 6 SHOULD be clarified |
| 7 | "Unpriceable" undefined | `prefix_and_verifier_frame_cost` defined in the pseudocode |
| 8 | Conditions 5, 7 and the target half of 3 are implied by 1 | Said so, and why they are kept |
| 9 | `S_end`-first skip changes the shared code budget | Order made normative: first occurrence order, `S_end` then `S_start`, eligible replay ends the check |
| 10 | `gas_fits` is one-dimensional under EIP-8037 | Stated as deliberate, with the direction it errs in |
| 11 | "does not proceed" | Ineligible, no further charge, MAY abort |
| 12 | Pre-frame check order | Not consensus; SHOULD, with the reason |
| 13 | Case 9 names `DELEGATECALL` for a read it cannot perform | Storage owner rule added to the surface; case 9 rewritten |
| 14 | Where undecided verdicts are recorded | Local record; nothing reaches the Engine API |
| 15 | "MUST evaluate both states within the call that executes `B`" | Compute from `B`'s own states in whichever call imports it; `ACCEPTED` judged when executed |
| 16 | Identity by bytes, implemented by digest | MAY key by the keccak-256 digest |
| 17 | Front placement alone does not meet the builder MUST | Two-pass construction RECOMMENDED, with the failing case |
| 18 | Test cases are descriptions, not vectors | Said so; vectors still to be published |
| 19 | `AA_VOPS_SLOT_COUNT` as chain configuration | No change; it worked |
| 20 | What the text got right | No change |
| 21 | Unnecessary text | No change; the `payer` sentence stays because it defines the surface, not a check |

Not applied, for the record: the implementation reads `S_end` through a discounted view of
the committed root and judges recent roots at the block's own slot, which is what the
running chain does; the spec did not have to change to describe it.

## Leg 2: frames base (`dogfood-frames`)

### Prerequisite base

`frames-devnet-0` is an ancestor of `hegota-testnet`, so the branch starts at
`hegota-testnet` (06d98078b) and needs no merge: the FOCIL prerequisites (EIP-7805 engine
surface, inclusion-list builder and Profile 1 validator, EIP-8250 keyed nonces, EIP-8272
recent roots) are already in the tree. The only work is removing the Profile 2 layer.

### Stripping the existing Profile 2 layer

Removed, so the base carries prerequisites only (commit `e7fcab06e`):

- `crates/blockchain/focil_profile2.rs` and `test/tests/blockchain/focil_profile2_tests.rs`
- from `focil_eligibility.rs`: `VopsProfile`, `classify`, `is_profile_2_candidate`,
  `profile_2_payer`, `evaluation_index`, `FillOutcome`, `fill_il_budget`, the VERIFY budget
  constants and cost functions. Kept: `SenderCode`, `classify_sender_code`, `fee_valid`,
  which the Profile 1 validator uses for the EIP-3607 sender gate and the fee gate.
- from `inclusion_list_validator.rs`: `Profile2Eligibility`, `IlProfile2Evaluator`,
  `IlCheckReport`, `check_with_profile_2`. `check` is now the whole Profile 1 pass and
  short-circuits on the first appendable omission. A transaction outside the four Profile 1
  types (legacy, EIP-2930, EIP-1559, EIP-7702) is excused, which covers blob carriers and
  frame transactions.
- from the engine handler and `add_block_pipeline_with_il`: the evaluator construction and
  the two `focil::profile2` log loops; both call `check` directly.
- the observer's `FocilVopsSurface`, `CodeBodyBudget`, `Profile2Replay`, the two violations
  they raise, the surface branches in the `SLOAD` and `SSTORE` hooks and the per-frame code
  charge; the `profile_2` parameter and `code_budget` field on the VM and backend wrappers
- the judgment-slot test in `focil_tests.rs` (`check_recent_root_frame_at_root` judged at
  the block's own slot); the method itself stays, as the EIP-8272 seam admission uses
- the two pointers into the removed code in `docs/eip-8272.md`, and the five operations
  documents that describe the implementation

Kept and not neutral: `docs/hegota-testnet.md`, `docs/hegota-testnet-joining.md` and
`docs/hegota-testnet-prs.md` describe the chain's rule set, including the EIP-8369
parameters (`AA_VOPS_SLOT_COUNT = 4`, the budgets, the two-endpoint index rule). They are
the joiner's operations manual and stay, but the implementing agent is told not to read them.

Pre-existing on `hegota-testnet`, not touched: an unused `FrameTransaction` import in
`crates/networking/rpc/types/receipt.rs` tests, two unused helpers in
`test/tests/levm/eip8141_tests.rs`, and a `redundant_clone` clippy error in
`crates/common/types/transaction.rs` tests. CI's `lint-l1` lints only libs and bins, which
is why they survive there.

### The base as a joiner (2026-09-15)

Before any Profile 2 work, the stripped base was run against the live chain from a laptop:
ethrex release build on the host, `ethpandaops/lighthouse:focil` in docker, genesis sync. It
synced 11,246 blocks in about six minutes, crossed the Hegotá boundary and imported every
frame-transaction block, and followed the head with three peers on each layer. Over the
first 92 blocks both nodes saw live, the beacon node's per-block `Record payload inclusion
list satisfaction` lines agree with the network's first beacon node, all `satisfied: true`.
The live chain has recorded no `satisfied: false` block in its first 11,655 records, so
agreement on the live chain alone cannot distinguish an implementation that judges frame
omissions from one that excuses them; that distinction has to come from the tests, or from
a deliberately omitting builder.

### Implementation

Written by a second agent under the same isolation, from the revised text (the one that
applies leg 1's findings), on the stripped `hegota-testnet` base. Four commits on
`dogfood-frames`, `31db0ff1a` to `9c1f8c43d`: the validation surface and per-list code
budget on the LEVM observer (`Profile2Surface`, `CodeBudget`, code charged at frame dispatch,
`CALL`-family and `EXTCODE*` targets and their delegates), `inclusion_list_profile2.rs`
(candidacy, pricing, the fill, `EvaluationState`, a `Profile2Replayer` trait,
`check_frame_omissions`, `WithdrawalDiscountedDb` with the EIP-161 empty-account rule),
`Blockchain::inclusion_list_satisfaction` shared by import and the engine RPC, the retained
list now carrying its verdict, the builder's second pass, and two test files: 28 unit tests
and 19 block-level scenarios covering rows 1, 2, 3, 4, 5, 6, 7, 8, 9 (both readings), 20,
21, 22, 24, 26, the withdrawal discount and the builder retry. Five excused-direction tests
were mutation-checked. 1,312 tests pass in `ethrex-test`, 47 of them new.

Isolation record: no excluded document was opened. The base itself leaked two hints the
implementer reported: `ChainConfig::aa_vops_slot_count` already existed with a doc comment
naming Profile 2, and two comments in the LEVM backend mentioned a budget carried between
replays of a list. Neither showed a rule or a code path.

### Spec feedback

The implementer's log is `leg2-spec-feedback.md` in this directory, 16 entries plus a list
of what implemented verbatim. Against the revised text the gaps are smaller and more local
than leg 1's, and none contradicts the base. Applied as follows.

| Entry | Finding | Applied |
| ---: | --- | --- |
| 1 | `protocol_verifier_frames(tx)` used, never defined; a misplaced verifier-shaped frame's status for pricing unclear | Terminology defines the frames by position; anywhere else is a body frame for every rule, pricing included |
| 2 | Keyed-nonce surface bullet: protocol read or `SLOAD` permission? | Protocol read for condition 2; not an `SLOAD` permission; nonce-manager storage is a third account |
| 3 | Which opcodes load a code body | Load points named: frame dispatch, `CALL` family, `EXTCODESIZE/COPY/HASH`; a delegation indicator is a body of its own |
| 4 | Whether the load that trips the bound is charged | It is not |
| 5 | Charges made by a replay that ends undecided | Stay charged; a replay failing before its first frame charges nothing |
| 6 | Replay `S_start` after an undecided `S_end`? | "not found eligible at `S_end`, whether ineligible or undecided" |
| 7 | Order stated over all transactions, only matters among Profile 2 | Order normative among Profile 2 candidates; Profile 1 MAY be judged first and stop the check |
| 8 | Recomputing the verdict is not idempotent under pruning | SHOULD retain the verdict with the list, with the pruning case as the reason |
| 9 | `FORK_TIMESTAMP` default clear; "schedule explicitly" implies a config surface | No change; the sentence is a requirement on chains, not clients |
| 10 | Activation-block paragraph names only `RECENT_ROOT_CODE` | Generalised to every predeploy the replay executes or reads, with the outcome for each |
| 11 | `MAX_VALIDATION_CODE_BYTES` depends on the fork | The number under EIP-7954 added to the table |
| 12 | The base mempool undercounts the expiry frame, as predicted | No change; the SHOULD already says so |
| 13 | Conditions 5 and 3's target half unreachable, as stated | No change |
| 14 | "Statically valid" against a client with stricter local static checks | EIP-8141's constraints and only those; stricter local checks MUST NOT apply |
| 15 | `payer` resolved from the shape versus the account `APPROVE` binds | Sentence added: necessarily the same account, a pre-computation not a second check |
| 16 | Rows 27, 28 and 23 cannot be chain scenarios | Said so under Test Cases |

Where the two logs meet: leg 2's entries 6 and 7 are the direct consequences of the
replay-order rule leg 1's entry 9 asked for, so the revision created two small questions
while answering one large one. Leg 2 confirms leg 1's entries 8, 12 and 13 independently.

## Operator documentation, not spec (`docs/hegota-testnet-joining.md`)

Once the chain is older than the weak-subjectivity period (256 epochs, 8,192 slots, about
13.6 hours at 6 seconds per slot), the documented genesis sync fails at startup:
`Failed to build beacon chain: The current head state is outside the weak subjectivity
period`, and the beacon node exits. `--allow-insecure-genesis-sync` alone is no longer
enough; Lighthouse also needs `--ignore-ws-check`. The "Starting a node" section was
written while the chain was younger than that and should say so. Found on 2026-09-15 at
slot about 11,700 with `ethpandaops/lighthouse:focil` (v8.1.3).

## Cross-implementation agreement (2026-09-15)

Both implementations ran side by side against the live chain from one laptop, each as an
ethrex release build on the host with `ethpandaops/lighthouse:focil` in docker, leg 2 given
leg 1's enode as an extra bootnode. Leg 1 (`dogfood-focil` at `f919aa57b`) synced 12,010
blocks; leg 2 (`dogfood-frames` at `9c1f8c43d`) synced 12,392. Both follow the head with
three beacon peers; leg 2 reports four execution peers to leg 1's three, and ethrex exposes
no peer list, so the loopback peering is inferred from that count, not shown.

Per-block `inclusionListSatisfied` records, taken from each node's beacon client and from
the network's first beacon node over the same six-minute window once both were at the head:

| Pair | Blocks in both | Disagreements |
| --- | ---: | ---: |
| leg 1 vs production | 60 | 0 |
| leg 2 vs production | 60 | 0 |
| leg 1 vs leg 2 | 60 | 0 |

Every record on all three sides is `satisfied: true`; neither node logged an undecided
verdict or a warning. The live chain has never produced an unsatisfied block, so this
agreement is one-sided: it shows that two independent readings of the text accept
everything the network accepts, and not that they reject the same things. The rejecting
direction rests on the two test suites, which between them cover every row of the Test
Cases table that a chain can produce, in both directions for rows 1, 2, 3, 7, 9 and 21.

One divergence is visible from the code rather than the chain. Leg 1 predates the EIP-161
rule the first revision added for an account that exists in the committed post-state only
because a withdrawal in `B` created it; leg 2 implements it and reads such an account as
nonexistent at `S_end`. A listed transaction whose validation observes that account's
existence, through EIP-8037's account-creation charge in `APPROVE`, would be judged
differently by the two nodes. It is the one place the two legs are known to disagree, it is
a consequence of the spec having changed between them, and the running chain's builder never
produces the block that would expose it.

What the exercise settled about the text: the first revision removed the large gaps (the
flat Engine API list, the reconstruction of `S_end`, the unstated replay order, the missing
chain id); the second round found only local ones, all answerable in a sentence, and the
implementer working from the revised text listed most of the document as implemented
verbatim. What it did not settle: the Test Cases are still descriptions and not vectors, so
two implementers agreeing on 60 live blocks and on their own scenarios is not the same as
agreeing on a shared vector suite. That is the next thing the spec needs.
