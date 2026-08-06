# Hegotá devnet branch — caveats

`hegota-devnet` is the integration branch that combines the EIP-8141 frame-transaction work with its extensions for multi-client interop testing. It is **not** an upstream-clean branch: it carries deliberate divergences from the (still-draft) EIPs, listed here. Each standalone EIP PR (`eip-8250`, `eip-8272`, `eip-7906`) targets `eip-8141-1` and is upstream-faithful; the divergences below exist only to make the combined devnet build and run.

## Composition

```
hegota-devnet = main       (EIP-8141 frame transactions)
              + eip-8250   (Keyed Nonces)
              + eip-8272   (Recent Roots)
              + eip-7906   (Tx Assertions, opcodes renumbered)
              + eip-8312   (UTXO Frames, own activation timestamp)
              + devnet-only config, docs, scripts and the
                ethrex-only extensions listed below
```

EIP-8141 itself lives on `main`; this branch adds only the three extension EIPs
on top of it, plus the devnet infrastructure and the ethrex-only extensions.

**Not yet included:**
- **FOCIL (EIP-7805)** — **deferred**, on the `focil` branch (PR #7039). The eligibility boundary is now specified: EIP-8369 (*VOPS Profiles for FOCIL Eligibility*, ethereum/EIPs#12110) puts frame transactions in Profile 2, judged at a builder-claimed transaction index, and states that FOCIL eligibility and public mempool admission are separate policies. It is Informational, so enforcement still needs a Standards Track extension to EIP-7805 that does not exist yet; `AA_VOPS_SLOT_COUNT` is unset upstream and this branch picks a value (see below). `focil` already excludes frame transactions from the IL satisfaction check, so combining the two is possible meanwhile, with frame-tx omission always excused.
  The merge surface is 18 files, not just `payload.rs`: `crates/blockchain/{blockchain,payload,mempool,error}.rs`, `crates/networking/rpc/{lib,rpc,utils}.rs` and `rpc/eth/transaction.rs`, `crates/networking/p2p/rlpx/connection/server.rs`, `crates/common/types/genesis.rs`, `cmd/ethrex/{cli,initializers}.rs`, `cmd/ethrex/l2/initializers.rs`, `docs/CLI.md`, and four test module files.
- **EIP-8288** (PQ sig + STARK aggregation) — deferred (upstream-blocked: no Lean leanSTARK/leanSPHINCS tooling; `AGGREGATED_VK`/hash TBD).

All included EIPs activate together under the existing single `Fork::Hegota` / `hegota_time`.

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

### EIP-8369 (FOCIL Eligibility) — applies to the `focil` branch, not yet merged here
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
