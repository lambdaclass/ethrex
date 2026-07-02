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

**bal-devnet-7** — last `bal-`-prefixed devnet. Future devnets prefixed `glamsterdam-`.

- Spec baseline: [`devnets/bal/7`](https://github.com/ethereum/execution-specs/tree/devnets/bal/7)
- Fixtures: [`tests-bal@v7.2.0`](https://github.com/ethereum/execution-specs/releases/tag/tests-bal@v7.2.0) (`.github/config/hive/amsterdam.yaml`)
- EELS commit: `a3e5201a53d8c94e2283ae170a2c71bbc233f7e7`
- Status: 🟢 aligned — blockchain ef-tests + hive `eels/consume-engine` Amsterdam all passing
- Tracking: [#6583]

---

## Implementation Status

### Implemented — Amsterdam EL (per [EIP-7773])

| EIP | Title | Status | SFI/CFI | Owner |
|-----|-------|--------|---------|-------|
| **7928** | Block-Level Access Lists | ✅ Implemented (devnet-7 aligned) | SFI (EL headliner) | Edgar |
| **7708** | ETH Transfers Emit Logs | ✅ Implemented | SFI | Edgar |
| **7778** | Block Gas Accounting without Refunds | ✅ Implemented | SFI | Edgar |
| **7843** | SLOTNUM Opcode | ✅ Implemented | SFI | Esteve |
| **8024** | DUPN/SWAPN/EXCHANGE | ✅ Implemented | SFI | Esteve |
| **8037** | State Creation Gas Cost (2D gas) | ✅ Implemented (devnet-7 aligned) | SFI (CFI→SFI @ ACDE #236) | Edgar |
| **7976** | Increase Calldata Floor Cost | ✅ Implemented | SFI | |
| **7981** | Increase Access List Cost | ✅ Implemented | SFI | |
| **7954** | Increase Max Contract Size (24→32 KiB) | ✅ Implemented | SFI | |
| **8159** | eth/71 Block Access List Exchange | ✅ Implemented | SFI (protocol req for bal-7) | |
| **7872** | Max Blob Flag for Local Builders | ✅ Implemented | PFI | Edgar |
| **8025** | Optional Execution Proofs | ✅ Implemented ([#6361], #6516, #6549, #6560) | Hegotá PFI ([EIP-8081]) | |

> **8025 note:** Stagnant on eips.ethereum.org and listed as PFI for Hegotá in [EIP-8081]. ethrex code paths exist (zkboost SSZ); status may shift to Hegotá-only.

### Not Implemented — Amsterdam EL candidates

Per [EIP-7773]:

| EIP | Title | SFI/CFI | Notes |
|-----|-------|---------|-------|
| **2780** | Reduce Intrinsic Transaction Gas (21000→4500) | CFI | No other client started |
| **7904** | General Repricing | CFI | Nethermind draft #9619 only |
| **8038** | State-Access Gas Cost Update | CFI | No other client started |
| **7997** | Deterministic Factory Predeploy | CFI | No other client started |
| **8070** | Sparse Blobpool | CFI | No other client started |
| **7610** | Revert Creation on Non-empty Storage | PFI | Confirmed PFI in [EIP-7773] |
| **7979** | Call/Return Opcodes | PFI | |
| **8163** | Reserve Opcode | PFI | |

### Amsterdam EL CFI candidates (not yet evaluated)

Not in ethrex; need triage per ACDE outcomes:

| EIP | Title | SFI/CFI |
|-----|-------|---------|
| **7688** | Consensus structures | CFI |
| **7975** | Networking | CFI |
| **8045** | Core | CFI |
| **8061** | Core | CFI |
| **8080** | Core | CFI |
| **8136** | Networking | CFI |
| **8246** | Core | CFI |

### CL-side (informational)

| EIP | Title | SFI/CFI |
|-----|-------|---------|
| **7732** | Enshrined Proposer-Builder Separation (ePBS) | SFI (CL headliner) |

### Notable DFI

Declined from Glamsterdam per [EIP-7773]: 47 EIPs including **EIP-7805 (FOCIL)**, EIP-7692 (EOF), EIP-7937 (64-bit EVM). FOCIL re-targeted at Hegotá.

---

## Active Work

### `tests-bal@v7.3.0` (expected ~2026-05-29)

Stability + extra tests only; no new spec semantics. Bundled upstream PRs:

**EIP-8037:**
- [specs#2898](https://github.com/ethereum/execution-specs/pull/2898) — reject when `calldata_floor > TX_MAX_GAS_LIMIT`
- [specs#2892](https://github.com/ethereum/execution-specs/pull/2892) — strict block-gas inclusion rule (**spec gap**; audit ethrex EIP-8037 block-gas inclusion against the strict rule before bumping the fixture pin)
- [specs#2876](https://github.com/ethereum/execution-specs/pull/2876) — reject tx when `gas_limit` covers regular but not state intrinsic
- [specs#2875](https://github.com/ethereum/execution-specs/pull/2875) — CREATE-tx collision refunds state-gas reservoir

**EIP-7928:**
- [specs#2897](https://github.com/ethereum/execution-specs/pull/2897) — extend BAL coverage
- [specs#2883](https://github.com/ethereum/execution-specs/pull/2883) — BAL withdrawal predeploy balance read across txs (Edgar)
- [specs#2893](https://github.com/ethereum/execution-specs/pull/2893) — selfdestruct to system address with 0 value

**Action on drop:** bump `.github/config/hive/amsterdam.yaml` `fixtures`/`eels_commit`, re-run blockchain ef-tests + hive `eels/consume-engine` Amsterdam.

### [EIPs#11699] — EIP-7702 delegation BAL exclusion

Tightens EIP-7928 §"EIP-7702 Delegation" so the delegated address is added to the BAL only if all of:
1. Sufficient gas for delegated `access_cost`
2. For value-transferring `CALL`/`CALLCODE`, `sender_balance >= value`
3. Call stack depth not violated

ethrex currently matches the **old** spec. When EELS merges:
- Move delegation `code_address` BAL recording from `record_bal_call_touch` (`crates/vm/levm/src/opcode_handlers/system.rs:889`) to after the `sender_balance`/depth guards inside `generic_call` (~line 962).
- Update `test/tests/levm/eip7928_tests.rs` to cover: 7702 + insufficient balance, 7702 + max depth.
- EELS fixtures will rewrite `test_bal_call_revert_insufficient_funds` for the 4 `delegated-*` variants.

### `eth_simulateV1` RPC

Implemented (tracked at [#6212]): multi-block simulation with state/block
overrides, `validation`, `traceTransfers` and `returnFullTransactions`, spec
error codes (-38010..-38026). Engine in `crates/blockchain/simulate.rs`,
handler in `crates/networking/rpc/eth/simulate.rs`. Known gaps, documented in
the engine: Amsterdam EIP-8037 2D gas / EIP-7928 BAL headers are not modeled,
`maxUsedGas` reports pre-refund gas rather than geth's in-flight peak, and
`movePrecompileToAddress` is parsed but not yet enforced. The hive
`rpc-compat` pin predates the `eth_simulateV1` test corpus, so CI does not
exercise it yet; conformance is covered by ported `.io` scenarios in the RPC
crate's tests.

---

## Out of Scope / Deferred

- **`debug_getRawBlockAccessList` RPC + `-32001` error code** per [execution-apis#794](https://github.com/ethereum/execution-apis/pull/794) — required for bal-devnet-7 protocol-side; tracked separately.
- **Debug receipt fields** ([PM #2033](https://github.com/ethereum/pm/issues/2033#issuecomment-4397074196)) — qu0b polling clients on extending `debug_getBlockReceipts` with `regularGasUsed` / `stateGasCharged` / `stateGasRefunded` / `cumulative*`. Cross-client debug aid; not bal-7 scope.
- **Deferred-on-success state-gas charging** for `CREATE`/`CREATE2`/`CALL*` (misilva73 audit point #3 in [specs#2804](https://github.com/ethereum/execution-specs/issues/2804)) — not landing in bal-7 per Maria Silva on Discord 2026-05-08.
- **EIP-8025 zkboost fixtures** — 21 known-bad witness fixtures skipped; resolves once zkevm@v0.4.x bundle is regenerated against bal-7.
- **Remaining gas repricing EIPs** (2780, 7904, 8038) — no other client has started; revisit if SFI'd at ACDE.

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

Docker: `bal-devnet-7` not in [`ethpandaops/eth-client-docker-image-builder/branches.yaml`](https://github.com/ethpandaops/eth-client-docker-image-builder/blob/master/branches.yaml); `ethpandaops/ethrex:bal-devnet-7` images update via manual Discord `workflow_dispatch`.

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
- [qu0b's bal-devnet-7 spec sheet](https://gist.github.com/qu0b/f3f905cadee4464a1a941838a5a5fadb)
- [Upstream tracker — execution-specs#2804](https://github.com/ethereum/execution-specs/issues/2804)
- [ethrex docs/eip.md](../eip.md) — EIP tracking
- [ethrex ROADMAP.md](../../ROADMAP.md) — general roadmap

### Other Client References
- [Nethermind PR #9619](https://github.com/NethermindEth/nethermind/pull/9619) — EIP-7904 General Repricing (Draft)
- [Reth Issue #18783](https://github.com/paradigmxyz/reth/issues/18783) — Amsterdam Hardfork Tracking

[#6212]: https://github.com/lambdaclass/ethrex/issues/6212
[#6361]: https://github.com/lambdaclass/ethrex/pull/6361
[#6583]: https://github.com/lambdaclass/ethrex/issues/6583
[EIPs#11699]: https://github.com/ethereum/EIPs/pull/11699
[EIP-7773]: https://eips.ethereum.org/EIPS/eip-7773
[EIP-8081]: https://eips.ethereum.org/EIPS/eip-8081
