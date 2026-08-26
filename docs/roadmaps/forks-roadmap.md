# Forks Team Roadmap - ethrex

## Amsterdam / Glamsterdam → Mainnet June 2026

## Glossary

| Acronym | Meaning |
|---------|---------|
| **SFI** | Scheduled for Inclusion - Will be in the fork |
| **CFI** | Considered for Inclusion - Likely, under discussion |
| **DFI** | Declined for Inclusion - Won't be included |
| **PFI** | Proposed for Inclusion - Proposed |
| **BAL** | Block-Level Access Lists (EIP-7928) |

---

## Current Devnet

**glamsterdam-devnet-8**

- Spec baseline: [`devnets/glamsterdam/8`](https://github.com/ethereum/execution-specs/tree/devnets/glamsterdam/8)
- Fixtures: [`tests-glamsterdam-devnet@v8.1.2`](https://github.com/ethereum/execution-specs/releases/tag/tests-glamsterdam-devnet@v8.1.2) (`.github/config/hive/amsterdam.yaml`, `tooling/ef_tests/.fixtures_url_amsterdam` — keep both on the same release)
- EELS commit: `50a7f6ecaf4963dc0c2b46b4ac55462a2efee314` (the v8.1.2 tag, also the tip of `devnets/glamsterdam/8`)
- Status: 🟢 aligned — blockchain, state and engine ef-tests all green on the v8.1.2 bundle
- Tracking: [#6583]

Upstream expects **at least two follow-up test releases** on this devnet (v8.1.0, v8.1.1 and
v8.1.2 have shipped) carrying additional coverage rather than new semantics. v8.1.2 is
announced as spec-equal to v8.1.1 apart from the EIP-7610 removals, and the diff agrees: no
repricing, and the only behavioural change under `src/ethereum/forks/amsterdam` is EIP-7610
dropping out ([specs#3417]) — the one remaining hunk there just deletes a stale TODO comment
from `get_last_256_block_hashes`. `account_deployable` no longer rejects a target with
storage, and `generic_create` correspondingly stops refunding the `NEW_ACCOUNT` state gas on
a collision, because a collision now implies the target has code or a nonce and was therefore
never charged. ethrex still applies the EIP-7610 storage check (`LevmAccount::has_storage`) —
see the EIP table below — but its state-gas side already agreed, so the whole bundle passes
unchanged. The devnet-8 baseline also still carries v8.1.1's one behaviour change, excess blob
gas surviving a fork transition instead of resetting ([specs#3352]), which ethrex already
matched. On each drop: bump
`tooling/ef_tests/.fixtures_url_amsterdam` and `.github/config/hive/amsterdam.yaml`
(`fixtures` + `eels_commit`), then re-run the three ef-test suites and hive
`eels/consume-engine` Amsterdam.

### Next up

| Item | Why it matters |
|------|----------------|
| **EIP-8070 hive coverage** | Mandatory for all ELs on devnet-8, but tested execute-only ([hive#1365]) — it ships no fixtures, so the bundle gives it zero coverage. Needs an `execute` sim wired up. |
| **devnet-8 zkEVM bundle** | `tests-zkevm` is still filled against devnet-7, so the stateless run skips every Amsterdam+ fixture. No devnet-8 stateless coverage until a new bundle ships. |
| **EIP-8038 spec text** | The v8.0.0 access-list repricing (cold minus `WARM_ACCESS`) landed in the tests ahead of its EIPs PR. Re-check the EIP once that merges. |
| **`eth_simulateV1`** | Still unimplemented. Tracked at [#6212]. |
| **EIP-8189 (snap/2)** | BAL-based state healing, newly listed in [EIP-7773]. Not evaluated. |

---

## Implementation Status

### Implemented — Amsterdam EL (per [EIP-7773])

The devnet-8 scope is 16 EIPs; ethrex implements all of them.

| EIP | Title | Status | Owner |
|-----|-------|--------|-------|
| **7928** | Block-Level Access Lists | ✅ devnet-8 aligned | Edgar |
| **8037** | State Creation Gas Cost (2D gas) | ✅ devnet-8 aligned | Edgar |
| **8038** | State-Access Gas Cost Update | ✅ devnet-8 aligned (access-list entries priced cold minus `WARM_ACCESS`) | Edgar |
| **2780** | Resource-based Intrinsic Transaction Gas | ✅ devnet-8 aligned (transfer log cost folded into `TX_VALUE_COST`) | Edgar |
| **7708** | ETH Transfers Emit Logs | ✅ | Edgar |
| **7778** | Block Gas Accounting without Refunds | ✅ | Edgar |
| **7843** | SLOTNUM Opcode | ✅ | Esteve |
| **8024** | DUPN/SWAPN/EXCHANGE | ✅ | Esteve |
| **7976** | Increase Calldata Floor Cost | ✅ | |
| **7981** | Increase Access List Cost | ✅ | |
| **7954** | Increase Max Contract Size (24→32 KiB) | ✅ | |
| **7610** | Revert Creation on Non-empty Storage | ✅ (`LevmAccount::has_storage`) — but DFI'd for Glamsterdam; upstream dropped the check and its fixtures in v8.1.2 ([specs#3417]), leaving the storage-only collision undefined in protocol. ethrex keeps it; dropping it is a mainnet-affecting decision, not a fixture bump. | |
| **8246** | Remove SELFDESTRUCT Balance Burn | ✅ | |
| **8282** | Builder Execution Requests | ✅ | |
| **8070** | eth/72 Sparse Blobpool | ✅ — but see "Next up": no fixture coverage | |
| **7997** | Deterministic Factory Predeploy | ✅ — genesis predeploy, needs no client code; 14 fixtures pass | |

Implemented but outside the devnet-8 set:

| EIP | Title | Status | Notes |
|-----|-------|--------|-------|
| **8159** | eth/71 Block Access List Exchange | ✅ | Protocol-side BAL exchange |
| **7975** | eth/70 Partial Block Receipt Lists | ✅ | |
| **7872** | Max Blob Flag for Local Builders | ✅ | PFI |
| **8025** | Optional Execution Proofs | ✅ ([#6361], #6516, #6549, #6560) | Hegotá PFI ([EIP-8081]); may become Hegotá-only |

### Not implemented — EL candidates

| EIP | Title | SFI/CFI | Notes |
|-----|-------|---------|-------|
| **7904** | Compute Gas Cost Analysis | CFI (Informational) | Nethermind draft #9619 only |
| **7979** | Call/Return Opcodes | PFI | |
| **8163** | Reserve Opcode | PFI | |
| **8189** | snap/2 BAL-Based State Healing | Listed in [EIP-7773] | Not evaluated |

### CL-side (informational)

No EL work; listed so ACDE outcomes can be tracked.

| EIP | Title | SFI/CFI |
|-----|-------|---------|
| **7732** | Enshrined Proposer-Builder Separation (ePBS) | SFI (CL headliner) |
| **7688** | Forward compatible consensus data structures | CFI |
| **8045** | Exclude slashed validators from proposing | CFI |
| **8061** | Increase exit and consolidation churn | CFI |
| **8080** | Let exits use the consolidation queue | CFI |
| **8136** | Cell-Level Deltas for Data Column Broadcast | CFI |

### Notable DFI

Declined from Glamsterdam per [EIP-7773]: 47 EIPs including **EIP-7805 (FOCIL)**, EIP-7692 (EOF), EIP-7937 (64-bit EVM). FOCIL re-targeted at Hegotá.

---

## Watch list

Upstream changes that would land as ethrex work if they move.

- **EIP-8038 access-list repricing** — shipped in the v8.0.0 tests ahead of its EIPs PR. Confirm the EIP text matches once merged.
- **EIP-7904 General Repricing** — Informational; only a Nethermind draft ([#9619]) exists. Revisit if it reaches SFI.
- **Deferred-on-success state-gas charging** for `CREATE`/`CREATE2`/`CALL*` (misilva73 audit point #3 in [specs#2804](https://github.com/ethereum/execution-specs/issues/2804)) — declined for the BAL devnets; would reopen only if re-proposed.
- **Debug receipt fields** ([PM #2033](https://github.com/ethereum/pm/issues/2033#issuecomment-4397074196)) — extending `debug_getBlockReceipts` with `regularGasUsed` / `stateGasCharged` / `stateGasRefunded` / `cumulative*`. Cross-client debug aid, not fork scope.

---

## Fork Infrastructure

`crates/common/types/genesis.rs` — fork enum order:

```
Frontier, FrontierThawing, Homestead, DaoFork, Tangerine, SpuriousDragon,
Byzantium, Constantinople, Petersburg, Istanbul, MuirGlacier, Berlin,
London, ArrowGlacier, GrayGlacier, Paris, Shanghai, Cancun, Prague,
Osaka, BPO1, BPO2, BPO3, BPO4, BPO5, Amsterdam
```

Activation timestamps wired in `ChainConfig`: `shanghai_time`, `cancun_time`, `prague_time`, `osaka_time`, `bpo1_time`..`bpo5_time`, `amsterdam_time`, plus `verkle_time`.

Network configs with Amsterdam timestamps:
- `cmd/ethrex/networks/holesky/genesis.json`
- `cmd/ethrex/networks/sepolia/genesis.json`
- `cmd/ethrex/networks/hoodi/genesis.json`

---

## Next Fork: Hegotá (H2 2026)

Post-Glamsterdam fork. CL = **Heka**, EL = **Bogotá** (some secondary press uses "Heze/Hegota"; primary source: [EIP-8081]).

### SFI

| EIP | Title | Notes |
|-----|-------|-------|
| **7805** | FOCIL — Fork-choice enforced Inclusion Lists | **Headliner.** Promoted to SFI after DFI from Glamsterdam |

### CFI

| EIP | Title | Notes |
|-----|-------|-------|
| **8141** | Frame Transaction (Account Abstraction) | Lost headliner debate; retained as non-headliner CFI |

### PFI

| EIP | Title |
|-----|-------|
| **4758** | Deactivate `SELFDESTRUCT` |
| **7709** | Read `BLOCKHASH` from storage and update cost (presented ACDE #236) |
| **7716** | Anti-correlation attestation penalties |
| **8025** | Optional Execution Proofs (ethrex already has code paths; see Amsterdam table) |
| **8188** | State Tiering by Write Age |
| **8205** | Withdrawal credentials preregistration |
| **8253** | Bump nonce of zero-nonce storage accounts (presented ACDE #236) |

### Infrastructure

`bpo1_time`..`bpo5_time` already defined in `ChainConfig` (see Fork Infrastructure above).

---

## ACDE Follow-up

Meetings on **Thursdays**. Agendas/notes at [ethereum/pm](https://github.com/ethereum/pm). Options:

1. **Attend live** — direct participation
2. **Post-call review** — YouTube + transcript with Claude:
   - Timestamps for specific topics
   - Summary of EIP discussions
   - Track CFI/SFI status changes

## ACDT Follow-up

All Core Devs — Testing meetings on **Mondays**. Agendas/notes at [ethereum/pm](https://github.com/ethereum/pm). Followed by Edgar.

---

## Links

- [EIP-7773 Meta Glamsterdam][EIP-7773]
- [EIP-8081 Meta Hegotá (Heka/Bogotá)][EIP-8081]
- [EIP-7928 Block-Level Access Lists](https://eips.ethereum.org/EIPS/eip-7928)
- [EIP-7732 ePBS (Glamsterdam CL headliner)](https://eips.ethereum.org/EIPS/eip-7732)
- [EIP-7805 FOCIL (Hegotá SFI)](https://eips.ethereum.org/EIPS/eip-7805)
- [Ansgar — Glamsterdam EL PFI'd EIPs](https://notes.ethereum.org/@ansgar/glamsterdam-el-pfi-eips)
- [ACDE #236 — May 7 2026](https://github.com/ethereum/pm/issues/2033)
- [Upstream tracker — execution-specs#2804](https://github.com/ethereum/execution-specs/issues/2804)
- [ethrex docs/eip.md](../eip.md) — EIP tracking
- [ethrex ROADMAP.md](../../ROADMAP.md) — general roadmap

### Other Client References
- [Nethermind PR #9619][#9619] — EIP-7904 General Repricing (Draft)
- [Reth Issue #18783](https://github.com/paradigmxyz/reth/issues/18783) — Amsterdam Hardfork Tracking

[#9619]: https://github.com/NethermindEth/nethermind/pull/9619
[hive#1365]: https://github.com/ethereum/hive/pull/1365
[specs#3352]: https://github.com/ethereum/execution-specs/pull/3352
[specs#3417]: https://github.com/ethereum/execution-specs/pull/3417

[#6212]: https://github.com/lambdaclass/ethrex/issues/6212
[#6361]: https://github.com/lambdaclass/ethrex/pull/6361
[#6583]: https://github.com/lambdaclass/ethrex/issues/6583
[EIP-7773]: https://eips.ethereum.org/EIPS/eip-7773
[EIP-8081]: https://eips.ethereum.org/EIPS/eip-8081
