# Hegotá devnet branch — caveats

`hegota-devnet` is the integration branch that combines the EIP-8141 frame-transaction work with its extensions for multi-client interop testing. It is **not** an upstream-clean branch: it carries deliberate divergences from the (still-draft) EIPs, listed here. Each standalone EIP PR (`eip-8250`, `eip-8272`, `eip-7906`) targets `eip-8141-1` and is upstream-faithful; the divergences below exist only to make the combined devnet build and run.

## Composition

```
hegota-devnet = main       (EIP-8141 frame transactions)
              + eip-8250   (Keyed Nonces)
              + eip-8272   (Recent Roots)
              + eip-7906   (Tx Assertions, opcodes renumbered)
              + eip-8312   (UTXO Frames, own activation timestamp)
              + eip-7805   (FOCIL inclusion lists)
              + devnet-only config, docs, scripts and the
                ethrex-only extensions listed below
```

EIP-8141 itself lives on `main`; this branch adds only the extension EIPs on top
of it, plus the devnet infrastructure and the ethrex-only extensions.

**Not yet included:**
- **EIP-8288** (PQ sig + STARK aggregation) — deferred (upstream-blocked: no Lean leanSTARK/leanSPHINCS tooling; `AGGREGATED_VK`/hash TBD).

EIP-8141/8250/8272/7906 and EIP-7805 all activate together under the single
`Fork::Hegota` / `hegota_time`, which is what the consensus layer calls `heze`.
EIP-8312 carries its own timestamp and is inert until a chain opts in.

## Opcode allocation (0xB region)

| Byte | Opcode | EIP | Note |
|------|--------|-----|------|
| `0xAA` | `APPROVE` | 8141 | |
| `0xB0`–`0xB4` | `TXPARAM`/`FRAMEDATALOAD`/`FRAMEDATACOPY`/`FRAMEPARAM`/`SIGPARAM` | 8141 | |
| `0xB5` | `RECENTROOTREFLOAD` | 8272 | spec-conformant; EIP-8272 assigns `0xB5` itself, to avoid the `SIGPARAM` collision |
| `0xB6` | `TXTRACE` | 7906 | ethrex allocation; EIP-7906 assigns no opcode bytes |
| `0xB7` | `EVENTDATACOPY` | 7906 | as above |
| `0xB8` | `TXDIFF` | 7906 | as above |
| `0xB9` | `NONCEKEYLOAD` | 8250 | **ethrex-only extension** — indexed `nonce_keys[i]`; spec defines no per-index accessor (see `docs/eip-8250.md`) |

EIP-7906's Constants table carries only `TXTRACE_GAS_COST`, `EVENTDATACOPY_GAS_COST` and `POST_TX`, so `0xB6`/`0xB7`/`0xB8` are ethrex's allocation, chosen to leave `0xB5` to EIP-8272. The standalone `eip-7906` branch uses the same three bytes, so the two agree; `test/tests/levm/eip7906_tests.rs` and `crates/vm/levm/src/opcode_handlers/tx_trace.rs` carry them. Flag upstream so the 8141-family drafts settle non-overlapping bytes.

## Per-EIP divergences

### EIP-8250 (Keyed Nonces) — see `docs/eip-8250.md`
- TXPARAM `nonce_keys[0]` at **`0x10`**: no longer a divergence, the spec assigns `0x10` and keeps `0x0B = len(signatures)`.
- `NONCE_MANAGER` predeploy at **`0x…8250`**: no longer a divergence, the spec pins this value.
- ⚠️ **Strict atomic-batch consumption durability not yet implemented** — flagged for devnet/interop validation.

### EIP-8272 (Recent Roots) — see `docs/eip-8272.md`
- `RECENT_ROOT_CODE` handled **natively** (spec `TBD`). No longer divergences: `RECENTROOTREFLOAD` at `0xB5`, TXPARAM `0x0F`, and `RECENT_ROOT_ADDRESS` at `0x…8272` all match the current spec.

### EIP-7906 (Tx Assertions)
- Opcodes renumbered as above. Behaviour otherwise unchanged.
- Frame mode stays at **3**; EIP-8312 takes 5 rather than renumbering it (see below).

### EIP-8312 (UTXO Frames) — see `docs/eip-8312.md`
- Frame mode **5** (spec `3`, already taken by EIP-7906 upstream; mode 4 stays reserved for EIP-8288 DEP_VERIFY).
- **Does not activate at `Fork::Hegota`**: its fork assignment is undecided upstream, so it gets its own `utxoFramesTime` chain-config timestamp. Absent by default, so the whole surface is inert until a chain opts in — and a future timestamp keeps the upgrade state-preserving (no new genesis).
- A UTXO frame and a POST_TX frame may not share a transaction (v1 composition rule; neither upstream draft defines it).
- `payer` is length-tested, never compared to numeric zero — closes a consensus-split ambiguity in the spec's pseudocode.

### EIP-7805 (FOCIL) — see `crates/blockchain/inclusion_list_{builder,validator}.rs`
- **Activates at `Fork::Hegota`, with no separate knob.** `hegota_time` and the consensus layer's `heze` fork are the same activation point by construction: the devnet fixture sets `heze_fork_epoch`, and `ethereum-genesis-generator` derives the execution genesis's `bogotaTime` from it. Introducing a second timestamp would create two clocks for one fork, and any gap between them is a halt window.
- The engine version guards resolve through `is_hegota_activated`: `newPayloadV5` and `forkchoiceUpdatedV4` are rejected from Hegotá on, because only V6/V5 carry `inclusionListTransactions`. This mirrors how a consensus client picks the version from its own fork — Lighthouse maps `ForkName::Gloas` to `newPayloadV5` + V4 attributes and `ForkName::Heze` to `newPayloadV6` + V5 attributes, with **no fallback** (every branch is `if capability { call } else { Err(RequiredMethodUnsupported) }`).
- **Consequence: the execution and consensus upgrades are atomic.** Hegotá is already active on the running devnet, so a node on this branch demands V6/V5 immediately. Deploying it under a client that speaks only V5/V4 halts that node, and swapping the client first halts it the other way. Plan the two as one operation; there is no inert intermediate state.
- Frame transactions are excluded from the IL satisfaction check, so frame-tx omission is always excused. That is the correct interim behaviour under EIP-8369 until Profile 2 enforcement is specified.
- **`heze` is already scheduled on the live devnet; only the client binary is behind.** `/network-configs/config.yaml` carries `HEZE_FORK_VERSION: 0x90000038` and `HEZE_FORK_EPOCH: 2`, but `ethpandaops/lighthouse:glamsterdam-devnet-7` is built from a branch whose `ForkName` ends at `Gloas`, so it parses neither and `/eth/v1/config/spec` reports no heze. That is why a stock client has been driving the frame-transaction stack. **No re-genesis is needed** — only a heze-aware image. The epoch is long past (heze is epoch 2, the chain is past epoch 3400), so such an image enters heze the moment it starts.

- **The client to move to is `sigp/lighthouse@focil`.** It is a strict superset of the `unstable` base the devnet-7 images build from: the same `ForkName` list through `Gloas`/`Heze`, `JsonPayloadAttributesV4`/`V5` both carrying `slot_number` (EIP-7843) and `target_gas_limit`, plus `getInclusionListV1`, `forkchoiceUpdatedV5` and `newPayloadV6`. Teku's `prototype/focil` matches, including `slotNumber` and `targetGasLimit` on `PayloadAttributesV4`. A newer stock build is not a substitute: `unstable` rejects a Heze payload outright with `UnsupportedForkVariant`. What remains unproven is the rest of the devnet-7 network config, since both branches sit ~100+ commits behind their own master; measure on a scratch enclave rather than inferring from branch dates.

### EIP-8369 (FOCIL Eligibility) — see `crates/blockchain/focil_eligibility.rs`
- **The Profile 2 storage surface replaces the EIP-8141 mempool rule rather than layering on it.** EIP-8141 admits `sender` at any slot; EIP-8369 admits `sender` and `payer` but only slots below `AA_VOPS_SLOT_COUNT`. Neither contains the other, and upstream never says which governs a node running both, so the validation observer switches rule wholesale when a FOCIL surface is configured.
- Not a divergence, but easy to mistake for one: **the VERIFY budget is computed twice, differently.** EIP-8369 says an expiry verifier frame "is ignored only when matching the four allowed prefix shapes; its gas limit still counts", so `verify_budget_cost` adds expiry frames back to the prefix sum, which `ValidationPrefix::frame_indices` omits for shape matching. The EIP-8141 mempool budget in `validate_prefix_structure` keeps summing the prefix alone. Both match their own spec; EIP-8141 rule 6 is silent on whether the expiry frame counts, and settling that is an upstream question rather than a reason to change admission unilaterally.
- Also conformance rather than divergence: **eligibility ignores the operator-tunable VERIFY budget.** `validate_prefix_structure` takes a `max_verify_gas` argument that `--mempool.max-verify-gas` controls; the eligibility path passes `u64::MAX` and checks `MAX_VERIFY_GAS_PER_TX` itself, so a node-local flag can never decide an attested verdict. That follows from eligibility and mempool admission being separate policies, a consequence the EIP states but does not draw out for clients sharing the code path.
- `AA_VOPS_SLOT_COUNT` = **4**, as a chain-config parameter rather than a constant. EIP-8369 leaves the value unset with a candidate range of 2 to 4 "pending benchmarks", so this fills a blank the spec left open rather than diverging from it.
- Chosen at the top of the range for two reasons. It is the worst case for attester replay, so a result that fits the attestation deadline at 4 also fits at 2 and 3; and it is a superset, so no transaction eligible at a lower value becomes unreachable. A low value would make wallets ineligible, which presents as fewer enforcement obligations and reads as success.
- The range covers the realistic validation surface: 1 slot for an address owner, 2 for a P256 pubkey, a third for a threshold or module word, with the fourth as headroom. Keyed nonces and recent roots already live in protocol state, so they cost no slots.
- **This measures replay cost only.** EIP-8369 names two costs, replay time and the growth of the globally held surface. A devnet has too few accounts for the "first `AA_VOPS_SLOT_COUNT` slots of every account" storage cost to register, so no number produced here is evidence about that side.
- The value is permissive relative to any final choice: a transaction eligible here may be ineligible once upstream settles the constant.

## Spec pins

The commit each implementation has been reconciled against, per `EIPS/eip-<n>.md`.
Bump a line only after verifying the implementation against that revision, since the
point of the pin is to make "what changed since we aligned" an exact question.
`/hegota-eips` diffs these against the head of each pinned source.

```pins
eip-8141  4a9ad32cf2  core      2026-07-30
eip-8250  81b976ac01  core      2026-08-03
eip-8272  d8636a330d  core      2026-08-03
eip-7906  ab022ace2a  core      2026-07-29
eip-8312  a5da3f608c  core      2026-08-05  nerolation/EIPs@nerolation/utxo-frame
eip-7805  9a345f96c2  focil     2026-02-20
eip-8369  8c9326fc05  focil     2026-08-06  soispoke/EIPs@codex/vops-profiles-focil
eip-7928  6c666b8d64  adjacent  2026-07-09
eip-8037  5a8c80897a  adjacent  2026-07-31
```

`core` is the frame-tx stack this branch carries. `focil` is implemented on the
`focil` branch, not here. `adjacent` EIPs are not part of the frame-tx envelope but
the devnet depends on them, so drift still matters.

The fifth column is the source, `owner/repo` or `owner/repo@ref`, and defaults to
`ethereum/EIPs` at its default branch. Two entries need it, for different reasons:
EIP-8312 exists only in its author's fork with no upstream pull request at all, and
EIP-8369 is an open pull request (ethereum/EIPs#12110) not yet merged to master. Either
way there is no `EIPS/eip-<n>.md` upstream to diff against. A fork branch can also be
rewritten under a pin in a way an upstream branch cannot, so the report distinguishes
"pin not found" from drift rather than reporting the whole history as new. Drop the
column when the EIP is upstreamed.

EIP-8369 (*VOPS Profiles for FOCIL Eligibility*) is Informational and defines no
consensus rules by itself, so its pin records the revision the FOCIL notes are
reconciled against rather than an implementation. It is the eligibility boundary any
FOCIL work on the `focil` branch must target.

## Upstream items

- **EIP-8312 vs EIP-7906 frame-mode collision** (both drafts claim mode 3). Needs a shared frame-mode registry in EIP-8141; raised with the EIP-8312 authors along with a set of spec-text findings (see the change's planning notes).

- EIP-8272 TXPARAM `0x0D → 0x0F` fix PR (drafted; from `lambdaclass/EIPs`).
- EIP-8250/8141 TXPARAM `0x0B` conflict (raise for an authoritative registry).

## Deploying

`hegota-devnet-genesis.md` covers what a fresh genesis must contain: the fork
schedule constraints, the predeploys the genesis generator does not ship, the
ethrex-only chain-config fields, and the post-deployment verification pass.

## Running a single Hegotá node locally

`--dev` needs no consensus client, which makes it the quickest way to reproduce
something on a Hegotá chain. Two things must be right or the node dies within
seconds of startup, both fatally and with a misleading error:

1. **Amsterdam must be scheduled.** Hegotá inherits Amsterdam's rules through the
   fork ordinal, but leaving `amsterdamTime` unset means the EIP-8038 repricing
   and the concurrent BAL-validation path never activate — so a local run can
   silently exercise different code from the devnet. Set both `amsterdamTime` and
   `hegotaTime`.
2. **The EIP-8282 builder deposit/exit predeploys must be preloaded**, with
   `EXCESS_INHIBITOR` (`2**256-1`) in slot 0. The genesis generator does not
   deploy them, and their empty code on an Amsterdam+ block makes the end-of-block
   system call fail: `System contract: 0x0000…8282 has no code after deployment`,
   which surfaces as `engine_getPayloadV5` failing rather than as anything about
   genesis. Copy them from the `additional_preloaded_contracts` block in
   `fixtures/networks/hegota-devnet.yaml`.

Derive the genesis from `fixtures/genesis/l1.json` (Osaka-era, so only the two
fork times need adding), then:

```bash
cargo run --release --features dev -- --dev --network <genesis.json> --datadir memory
```

A healthy node logs `Produced block` and its headers carry both `slotNumber`
(EIP-7843) and `blockAccessListHash` (EIP-7928). The producer stands in for the
slot clock, advancing one slot per block.

The dev producer is fatal by design: three consecutive failures shut the node
down, so a genesis problem looks like an immediate exit rather than a stalled
chain. Read the three `Failed to produce block` lines above the shutdown — they
name the engine-API call that rejected the payload.
