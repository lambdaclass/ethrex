# Hegotá devnet branch — caveats

`hegota-devnet` is the integration branch that combines the EIP-8141 frame-transaction work with its extensions for multi-client interop testing. It is **not** an upstream-clean branch: it carries deliberate divergences from the (still-draft) EIPs, listed here. Each standalone EIP PR (`eip-8250`, `eip-8272`) targets `eip-8141-1` and is upstream-faithful; the divergences below exist only to make the combined devnet build and run.

## Composition

```
hegota-devnet = main       (EIP-8141 frame transactions)
              + eip-8250   (Keyed Nonces)
              + eip-8272   (Recent Roots)
              + eip-8312   (UTXO Frames, own activation timestamp)
              + eip-7805   (FOCIL inclusion lists)
              + devnet-only config, docs, scripts and the
                ethrex-only extensions listed below
```

EIP-8141 itself lives on `main`; this branch adds only the extension EIPs on top
of it, plus the devnet infrastructure and the ethrex-only extensions.

**Not yet included:**
- **EIP-8288** (PQ sig + STARK aggregation) — deferred (upstream-blocked: no Lean leanSTARK/leanSPHINCS tooling; `AGGREGATED_VK`/hash TBD).

EIP-8141/8250/8272 and EIP-7805 all activate together under the single
`Fork::Hegota` / `hegota_time`, which is what the consensus layer calls `heze`.
EIP-8312 carries its own timestamp and is inert until a chain opts in.

## Opcode allocation (0xB region)

| Byte | Opcode | EIP | Note |
|------|--------|-----|------|
| `0xAA` | `APPROVE` | 8141 | |
| `0xB0`–`0xB4` | `TXPARAM`/`FRAMEDATALOAD`/`FRAMEDATACOPY`/`FRAMEPARAM`/`SIGPARAM` | 8141 | |
| `0xB5` | `RECENTROOTREFLOAD` | 8272 | spec-conformant; EIP-8272 assigns `0xB5` itself, to avoid the `SIGPARAM` collision |

## Per-EIP divergences

### EIP-8250 (Keyed Nonces) — see `docs/eip-8250.md`
- TXPARAM `nonce_keys[0]` at **`0x10`**: no longer a divergence, the spec assigns `0x10` and keeps `0x0B = len(signatures)`.
- `NONCE_MANAGER` predeploy at **`0x…8250`**: no longer a divergence, the spec pins this value.
- ⚠️ **Strict atomic-batch consumption durability not yet implemented** — flagged for devnet/interop validation.

### EIP-8272 (Recent Roots) — see `docs/eip-8272.md`
- No divergences remain. `RECENTROOTREFLOAD` at `0xB5`, TXPARAM `0x0F` and `RECENT_ROOT_ADDRESS` at `0x…8272` all match the current spec, and `RECENT_ROOT_CODE` is the 144-byte predeploy from ethereum/EIPs#12131 (`keccak256` = `0x432c8b183d17d5e9939623833203b9a5b62325246cfcd9307982bfde8f18c6fb`) rather than a native VM write. The predeploy replaced a flat `RECENT_ROOT_WRITE_GAS` that skipped the EIP-8037 state-gas charge on the created slot, so the same write cost 38 064 gas here against 127 196 on a client executing the code — a consensus split that is now closed. Its two `PUSH32` immediates are `keccak256("RECENT_ROOT_ENTRY")` and `keccak256("RECENT_ROOT_STORAGE")`, the same domains ethrex already derived, so the hash layout is unchanged and only gas and call semantics moved.
- ⚠️ **`RECENT_ROOT_CODE` is provisional**: ethereum/EIPs#12131 is open, not merged. Its bytes could still move (the `0x1fff` mask is the live question), and a change to them changes the code hash and the write's gas on a chain already running.

### EIP-8312 (UTXO Frames) — see `docs/eip-8312.md`
- Frame mode **5** (spec `3`; ethrex leaves 3 unassigned and reserves 4 for EIP-8288's deferred DEP_VERIFY).
- **Does not activate at `Fork::Hegota`**: its fork assignment is undecided upstream, so it gets its own `utxoFramesTime` chain-config timestamp. Absent by default, so the whole surface is inert until a chain opts in — and a future timestamp keeps the upgrade state-preserving (no new genesis).
- **Present but inert on `hegota-testnet`: `utxoFramesTime` unset.** Frame mode 5 is rejected by static validation, no vault account is installed and no block-end openings root is written, so the code in the tree is unreachable from the published genesis. Pinned by `a_utxo_frame_is_rejected_while_utxo_frames_time_is_unset`.
- `payer` is length-tested, never compared to numeric zero — closes a consensus-split ambiguity in the spec's pseudocode.

### EIP-7805 (FOCIL) — see `crates/blockchain/inclusion_list_{builder,validator}.rs`
- **Activates at `Fork::Hegota`, with no separate knob.** `hegota_time` and the consensus layer's `heze` fork are the same activation point by construction: the devnet fixture sets `heze_fork_epoch`, and `ethereum-genesis-generator` derives the execution genesis's `bogotaTime` from it. Introducing a second timestamp would create two clocks for one fork, and any gap between them is a halt window.
- The engine version guards resolve through `is_hegota_activated`: `newPayloadV5` and `forkchoiceUpdatedV4` are rejected from Hegotá on, because only V6/V5 carry `inclusionListTransactions`. This mirrors how a consensus client picks the version from its own fork — Lighthouse maps `ForkName::Gloas` to `newPayloadV5` + V4 attributes and `ForkName::Heze` to `newPayloadV6` + V5 attributes, with **no fallback** (every branch is `if capability { call } else { Err(RequiredMethodUnsupported) }`).
- **Consequence: the execution and consensus upgrades are atomic.** Hegotá is already active on the running devnet, so a node on this branch demands V6/V5 immediately. Deploying it under a client that speaks only V5/V4 halts that node, and swapping the client first halts it the other way. Plan the two as one operation; there is no inert intermediate state.
- **Frame transactions are enforced, not excused.** The blanket `TxType::Frame` skip is gone: both callers of the satisfaction check construct a `BlockchainProfile2Evaluator` and call `check_with_profile_2` (`crates/networking/rpc/engine/inclusion_list.rs:139`, `crates/blockchain/blockchain.rs:2558`), which replays each omitted candidate's validation prefix (`crates/blockchain/focil_profile2.rs`). EIP-8369 Profile 2 is therefore live, and it has no off switch — `ChainConfig::aa_vops_slot_count()` falls back to `DEFAULT_AA_VOPS_SLOT_COUNT = 4`, so an absent field selects the constant rather than disabling the profile. EIP-8369 specifies the profiles but explicitly defers the enforcement point to an extension EIP; the replay evaluates at the judged header's post-execution state, which is EIP-7805's own end-of-payload state. `docs/hegota-testnet.md` records the two-endpoint rule that completes it.
- **The builder side is symmetric: frame transactions are eligible for locally built inclusion lists.** Excluding them was the original plan, on the grounds that an entry every client must excuse buys no censorship resistance — false once omissions are enforced rather than excused, and it would have dropped censorship resistance for the one transaction type this network exists to exercise. The mechanical half of that reasoning was real and is handled by nonce domain instead of by exclusion: `is_linear_nonce_domain` (`crates/blockchain/inclusion_list_builder.rs`) routes keyed frame transactions to a bucket the per-sender linear-nonce walk never touches, and frame transactions are exempt from balance gating because the payer is a paymaster resolved during the validation prefix, so the sender's balance says nothing about affordability. The `apply_inclusion_list_transactions` path additionally repeats the fork and expiry gates `fill_transactions` applies, deciding both from the envelope and the payload timestamp rather than spending an execution attempt per entry.
- **Frame-tx inclusion-list eligibility is unspecified upstream.** EIP-7805 says nothing about transaction types beyond the blob exclusion, and ethereum/EIPs#12110 (*VOPS Profiles for FOCIL Eligibility*, EIP-8369) is the candidate mechanism — still an open PR. ethrex implements it ahead of the merge rather than excusing the whole type, so this is a place to re-check when it lands.
- **`engine_getInclusionListV1` tolerates one ignored parameter.** The engine API specifies `params: []`, but every consensus client implementing FOCIL today sends a `parentHash`: the engine-api FOCIL methods landed 2026-08-03 and both `sigp/lighthouse@focil` (2026-06-16) and `Consensys/teku@prototype/focil` (2026-06-25) predate them. Rejecting the call is spec-correct and deadlocks the chain at the fork boundary, because the client cannot build an inclusion list and stops driving the execution layer. Ignoring the argument is sound rather than merely permissive, since the list is built against this node's canonical head, which is the block that hash identifies. Remove once a client ships the specified signature.
- **`engine_forkchoiceUpdatedV5` accepts the `custodyColumns` third parameter.** Previously only one or two parameters were accepted, so the spec-conformant three-parameter call was rejected and payload building failed from the fork boundary on. The value is a 16-byte PeerDAS custody bitarray; ethrex advertises no custody-dependent behaviour and ignores it. Lighthouse sends `null` here by choice, which the spec permits.
- **`heze` is already scheduled on the live devnet; only the client binary is behind.** `/network-configs/config.yaml` carries `HEZE_FORK_VERSION: 0x90000038` and `HEZE_FORK_EPOCH: 2`, but `ethpandaops/lighthouse:glamsterdam-devnet-7` is built from a branch whose `ForkName` ends at `Gloas`, so it parses neither and `/eth/v1/config/spec` reports no heze. That is why a stock client has been driving the frame-transaction stack. **No re-genesis is needed** — only a heze-aware image. The epoch is long past (heze is epoch 2, the chain is past epoch 3400), so such an image enters heze the moment it starts.

- **The client to move to is `sigp/lighthouse@focil`.** It is a strict superset of the `unstable` base the devnet-7 images build from: the same `ForkName` list through `Gloas`/`Heze`, `JsonPayloadAttributesV4`/`V5` both carrying `slot_number` (EIP-7843) and `target_gas_limit`, plus `getInclusionListV1`, `forkchoiceUpdatedV5` and `newPayloadV6`. Teku's `prototype/focil` matches, including `slotNumber` and `targetGasLimit` on `PayloadAttributesV4`. A newer stock build is not a substitute: `unstable` rejects a Heze payload outright with `UnsupportedForkVariant`. What remains unproven is the rest of the devnet-7 network config, since both branches sit ~100+ commits behind their own master; measure on a scratch enclave rather than inferring from branch dates.

### EIP-8369 (FOCIL Eligibility) — see `crates/blockchain/focil_eligibility.rs`
- **The Profile 2 storage surface replaces the EIP-8141 mempool rule rather than layering on it.** EIP-8141 admits `sender` at any slot; EIP-8369 admits `sender` and `payer` but only slots below `AA_VOPS_SLOT_COUNT`. Neither contains the other, and upstream never says which governs a node running both, so the validation observer switches rule wholesale when a FOCIL surface is configured.
- Not a divergence, but easy to mistake for one: **the VERIFY budget is computed twice, differently.** EIP-8369 says an expiry verifier frame "is ignored only when matching the four allowed prefix shapes; its gas limit still counts", so `verify_budget_cost` adds expiry frames back to the prefix sum, which `ValidationPrefix::frame_indices` omits for shape matching. The EIP-8141 mempool budget in `validate_prefix_structure` keeps summing the prefix alone. Both match their own spec; EIP-8141 rule 6 is silent on whether the expiry frame counts, and settling that is an upstream question rather than a reason to change admission unilaterally.
- Also conformance rather than divergence: **eligibility ignores the operator-tunable VERIFY budget.** `validate_prefix_structure` takes a `max_verify_gas` argument that `--mempool.max-verify-gas` controls; the eligibility path passes `u64::MAX` and checks `MAX_VERIFY_GAS_PER_TX` itself, so a node-local flag can never decide an attested verdict. That follows from eligibility and mempool admission being separate policies, a consequence the EIP states but does not draw out for clients sharing the code path.
- `AA_VOPS_SLOT_COUNT` = **4**, as a chain-config parameter rather than a constant. EIP-8369 leaves the value unset with a candidate range of 2 to 4 "pending benchmarks", so this fills a blank the spec left open rather than diverging from it.
- **Live on `hegota-testnet` with `aaVopsSlotCount` unset**, which is not the same kind of absence as EIP-8312's. The profile has no off switch: an absent field selects the default 4 rather than disabling enforcement, so a joining client MUST NOT read the missing key as "Profile 2 off". Pinned by `aa_vops_slot_count_defaults_to_four`.
- **A per-inclusion-list code-body budget bounds replay reads**: 16 distinct bodies and 16 x 64 KiB per list, charged inside the replay and shared across every candidate and both evaluation endpoints. EIP-8369 does not specify one; VERIFY gas alone leaves an attester's read work to the list's author. See `docs/hegota-testnet.md` for the derivation.

### Chain-config fields that must stay absent from the published genesis

Each is a surface the tree carries and the testnet does not run. A field that
appears by accident is consensus-visible divergence from a client reading the
same genesis, so each has a test that fails if the default moves.

| Field | Effect if set | Pinned by |
| --- | --- | --- |
| `utxoFramesTime` | Activates EIP-8312: frame mode 5, the vault predeploy, block-end openings roots | `a_utxo_frame_is_rejected_while_utxo_frames_time_is_unset` |
| `payerTxparamTime` | `TXPARAM(0x11)` returns the resolved payer instead of halting; an ethrex extension with no EIP behind it | `txparam_0x11_is_inactive_while_payer_txparam_time_is_unset` |
| `derivedSlotTime` | The EL derives the EIP-7843 slot from the block timestamp instead of taking the CL's, re-keying every recent-root entry | `slotnum_comes_from_the_header_and_is_never_derived` |
| `genesisTimestamp`, `secondsPerSlot` | Inert alone; only feed the `derivedSlotTime` derivation | (as above) |

`aaVopsSlotCount` is deliberately NOT on this list: absent means 4, not off.

All four live in `test/tests/levm/hegota_active_surface_tests.rs`, against a
`ChainConfig` shaped like the published genesis.
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
eip-8312  a5da3f608c  core      2026-08-05  nerolation/EIPs@nerolation/utxo-frame
eip-7805  9a345f96c2  focil     2026-02-20
eip-8369  ad8571028a  focil     2026-08-07  soispoke/EIPs@codex/vops-profiles-focil
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

- EIP-8312 specifies `UTXO_MODE = 3`; ethrex places it at 5 and leaves 3 unassigned instead. Needs a shared frame-mode registry in EIP-8141; raised with the EIP-8312 authors along with a set of spec-text findings (see the change's planning notes).

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
