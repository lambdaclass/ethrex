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

## Current Implementation Status

### Core Devnet EIPs (Priority)

| EIP | Title | Code Status | Tests | devnet-bal | SFI/CFI | Owner |
|-----|-------|-------------|-------|------------|---------|-------|
| **7928** | Block-Level Access Lists | ✅ Merged ([#6020], [#6024], fix [#6149]) · Types, engine_newPayloadV5, execution tracking, hash validation, recorder fixes | Amsterdam state tests: 250/250 | ✅ | SFI | Edgar |
| **7708** | ETH Transfers Emit Logs | ✅ Merged ([#6074], fix [#6104], fix [#6149]) · [exec-specs tracking](https://github.com/ethereum/execution-specs/issues/1879) | Amsterdam state tests: 250/250 | ✅ | CFI | Edgar |
| **7778** | Block Gas Accounting without Refunds | ✅ Merged ([#5996], fix [#6128]) · [exec-specs tracking](https://github.com/ethereum/execution-specs/issues/1940) | 7 unit tests in `eip7778_tests.rs` | ✅ | CFI | Edgar |
| **7843** | SLOTNUM Opcode | ✅ Merged ([#5973]) · [exec-specs tracking](https://github.com/ethereum/execution-specs/issues/2028) | ~7 tests (skipped) | ✅ | CFI | Esteve |
| **8024** | DUPN/SWAPN/EXCHANGE | ✅ Merged ([#5970], fix [#6118]) · [exec-specs tracking](https://github.com/ethereum/execution-specs/issues/1942) | Blockchain tests passing ✅ | ✅ | CFI | Esteve |

### Gas Repricing EIPs (New - not on devnet-bal yet)

| EIP | Title | Code Status | Nethermind | Reth | SFI/CFI |
|-----|-------|-------------|------------|------|---------|
| **2780** | Reduce Intrinsic Transaction Gas | 🔴 Not implemented (21000 → 4500) · [exec-specs tracking](https://github.com/ethereum/execution-specs/issues/1940) | 🔴 | 🔴 | CFI |
| **7904** | General Repricing | 🔴 Not implemented · [exec-specs tracking](https://github.com/ethereum/execution-specs/issues/1879) | ⚠️ PR #9619 (Draft) | 🔴 | CFI |
| **7954** | Increase Max Contract Size | 🔴 Not implemented (24KiB → 32KiB) · [exec-specs tracking](https://github.com/ethereum/execution-specs/issues/2028) | ⚠️ PR #8760 (Draft) | 🔴 | CFI |
| **7976** | Increase Calldata Floor Cost | 🔴 Not implemented · [exec-specs tracking](https://github.com/ethereum/execution-specs/issues/1942) | 🔴 | 🔴 | CFI |
| **7981** | Increase Access List Cost | 🔴 Not implemented · [exec-specs tracking](https://github.com/ethereum/execution-specs/issues/1943) | 🔴 | 🔴 | CFI |
| **8037** | State Creation Gas Cost Increase | 🔴 Not implemented · [exec-specs tracking](https://github.com/ethereum/execution-specs/issues/2040) | 🔴 | 🔴 | CFI |
| **8038** | State-Access Gas Cost Update | 🔴 Not implemented · [exec-specs tracking](https://github.com/ethereum/execution-specs/issues/1941) | 🔴 | 🔴 | CFI |

> **Priority note:** All core devnet EIPs are merged, EF state tests pass, and hive tests are updated (PR [#6009] merged). bal-devnet-2 is validated (PR [#6201] merged). **Up next for devnet-3:** EIP-8037 ([#6213]) and EIP-7954 ([#6214]) now have issues filed. Gas repricing EIPs are **low priority** — no other client has started them. EIP-7904 and EIP-7954 are **medium priority** (Nethermind has draft PRs). Monitor CFI decisions at ACDE calls.

### Other Amsterdam EIPs

| EIP | Title | Code Status | Nethermind | Reth | SFI/CFI |
|-----|-------|-------------|------------|------|---------|
| **7997** | Deterministic Factory Predeploy | 🔴 Not implemented · [exec-specs tracking](https://github.com/ethereum/execution-specs/issues/1988) | 🔴 | 🔴 | CFI |
| **8070** | Sparse Blobpool | 🔴 Not implemented (ROADMAP.md: Priority —) | 🔴 | 🔴 | CFI |
| **7610** | Revert Creation on Non-empty Storage | 🔴 Not implemented | 🔴 | 🔴 | PFI |
| **7872** | Max Blob Flag for Local Builders | ✅ Merged ([#5769]) | 🔴 | 🔴 | PFI |

---

## February 10 Status Update

### All Core Devnet EIPs Merged ✅
- [x] **EIP-7928** (Block-Level Access Lists) - [#6020](https://github.com/lambdaclass/ethrex/pull/6020), [#6024](https://github.com/lambdaclass/ethrex/pull/6024), fix [#6149](https://github.com/lambdaclass/ethrex/pull/6149) → Edgar
  - Types + `engine_newPayloadV5` (Part 1)
  - Execution tracking + hash validation (Part 2, merged Feb 9)
  - BAL recorder fixes: SYSTEM_ADDRESS handling, selfdestruct cleanup, storage write-to-read reversion, gas-check gating for CALL/CREATE opcodes ([#6149](https://github.com/lambdaclass/ethrex/pull/6149))
- [x] **EIP-7708** (ETH Transfer Logs) - [#6074](https://github.com/lambdaclass/ethrex/pull/6074), fix [#6104](https://github.com/lambdaclass/ethrex/pull/6104), fix [#6149](https://github.com/lambdaclass/ethrex/pull/6149) → Edgar
  - Fix: selfdestruct-to-self and CALLCODE self-transfer log emission ([#6149](https://github.com/lambdaclass/ethrex/pull/6149))
- [x] **EIP-7778** (Gas Accounting) - [#5996](https://github.com/lambdaclass/ethrex/pull/5996), fix [#6128](https://github.com/lambdaclass/ethrex/pull/6128) → Edgar
- [x] **EIP-8024** (DUPN/SWAPN/EXCHANGE) - [#5970](https://github.com/lambdaclass/ethrex/pull/5970), fix [#6118](https://github.com/lambdaclass/ethrex/pull/6118) → Esteve
- [x] **EIP-7843** (SLOTNUM) - [#5973](https://github.com/lambdaclass/ethrex/pull/5973) → Esteve
- [x] **EIP-7872** (Max Blob Flag) - [#5769](https://github.com/lambdaclass/ethrex/pull/5769) → Edgar

### EF Tests ✅
- [x] **Amsterdam state tests: 250/250 passing**
- [x] **Prague, Cancun, Shanghai, Paris state tests: 51,728/51,728 passing**
- [x] Removed 150+ line Amsterdam skip list from `tooling/ef_tests/blockchain/tests/all.rs` ([#6149](https://github.com/lambdaclass/ethrex/pull/6149))
- [x] Added `run-ef-tests.py` script for running EF state tests across forks ([#6149](https://github.com/lambdaclass/ethrex/pull/6149))
- [x] Added Amsterdam to default forks in state test runner ([#6149](https://github.com/lambdaclass/ethrex/pull/6149))

### Remaining
- [x] Update hive tests for Amsterdam (PR [#6009] merged ✅)
- [x] bal-devnet-2 fixes (PR [#6201] merged ✅)
- [ ] Monitor EEST test changes / EIP spec changes
- [ ] Start gas repricing EIPs — EIP-8037 ([#6213]) and EIP-7954 ([#6214]) filed for devnet-3
- [ ] BAL optimizations: parallel execution ([#6209]), parallel state root ([#6210]), batched reads ([#6211])
- [ ] RPC: eth_simulateV1 ([#6212])

---

## February 16 Status Update

### bal-devnet-2 ✅
PR [#6201] merged — ethrex proposes and validates blocks post-Gloas in bal-devnet-2 kurtosis network:
- `engine_getPayloadV6` + `engine_newPayloadV5` capability
- Fix BAL hash validation (hash raw RLP bytes, not re-encoded)
- Fix EIP-7778 receipt gas tracking in block building
- Distinguish gas allowance exceeded vs block gas overflow
- Fix `engine_getClientVersionV1` commit hash
- bal-devnet-2 kurtosis fixture + ethereum-package update

### Hive Tests ✅
PR [#6009] merged — Amsterdam hive test support.

### Next Priorities Filed
- **devnet-3 EIPs:** EIP-8037 State Creation Gas Cost ([#6213]), EIP-7954 Max Contract Size ([#6214])
- **BAL optimizations:** Parallel block execution ([#6209]), parallel state root calculation ([#6210]), batched state reads ([#6211])
- **RPC:** eth_simulateV1 ([#6212])

---

## Fork Infrastructure

The codebase already has Amsterdam support in the fork system:

```rust
// crates/common/types/genesis.rs
pub enum Fork {
    // ... 25 earlier forks ...
    Amsterdam  // Fork 26
}

// Timestamp activation
pub amsterdam_time: Option<u64>
pub fn is_amsterdam_activated(&self, block_timestamp: u64) -> bool
```

**Network configs with Amsterdam timestamps:**
- `cmd/ethrex/networks/holesky/genesis.json`
- `cmd/ethrex/networks/sepolia/genesis.json`
- `cmd/ethrex/networks/hoodi/genesis.json`

---

## Ongoing: EIP Evaluation

Read and evaluate new EIPs proposed for Glamsterdam:

- [**EL PFI'd EIPs (Ansgar)**](https://notes.ethereum.org/@ansgar/glamsterdam-el-pfi-eips) - Live progress

**Key areas to watch:**
- Gas repricing changes (affects economics significantly)
- Any new opcodes beyond current set
- State growth mitigations

---

## Next Fork: Hegota (H2 2026)

Post-Glamsterdam fork, execution layer = **Bogota**

| Topic | Details |
|-------|---------|
| **FOCIL (EIP-7805)** | Inclusion lists for censorship resistance |
| **Deferred EIPs** | Whatever doesn't make Glamsterdam |
| **BPO sequence** | `bpo1_time` through `bpo5_time` already defined in ChainConfig |

> Headliner EIP to be decided February 2026

---

## BAL Optimizations (Non-EIP)

| Issue | Title | Status |
|-------|-------|--------|
| [#6209] | Parallel block execution | Not started |
| [#6210] | Parallel state root calculation | Not started |
| [#6211] | Batched state reads | Not started |
| [#6212] | eth_simulateV1 RPC | Not started |

---

## Technical Debt / Action Items

| Item | Location | Priority | Status |
|------|----------|----------|--------|
| Update `docs/eip.md` supported status | `docs/eip.md` | High | ✅ Done |
| Complete BAL execution integration | PR [#6024](https://github.com/lambdaclass/ethrex/pull/6024) | High | ✅ Merged |
| BAL recorder + EIP-7708 fixes | PR [#6149](https://github.com/lambdaclass/ethrex/pull/6149) | High | ✅ Done |
| Enable Amsterdam EIP tests | `tooling/ef_tests/blockchain/tests/all.rs` | Medium | ✅ Done (skip list removed) |
| Update hive tests for Amsterdam | PR [#6009] | Medium | ✅ Done (merged) |
| bal-devnet-2 fixes | PR [#6201] | High | ✅ Done (merged) |
| EIP-8037 State Creation Gas Cost (devnet-3) | [#6213] | High | Not started |
| EIP-7954 Max Contract Size (devnet-3) | [#6214] | Medium | Not started |
| BAL optimizations (parallel exec, state root, batched reads) | [#6209], [#6210], [#6211] | Medium | Not started |
| eth_simulateV1 RPC | [#6212] | Medium | Not started |
| Gas repricing EIPs | Various | Low | Not started (no other client has either) |

---

## Links

- [EIP-7773 Meta Glamsterdam](https://eips.ethereum.org/EIPS/eip-7773)
- [BAL Info](https://blockaccesslist.xyz)
- [ethrex docs/eip.md](../eip.md) - EIP tracking
- [ethrex ROADMAP.md](../../ROADMAP.md) - General roadmap

### Other Client References
- [Nethermind PR #9619](https://github.com/NethermindEth/nethermind/pull/9619) - EIP-7904 General Repricing (Draft)
- [Nethermind PR #8760](https://github.com/NethermindEth/nethermind/pull/8760) - EIP-7954 Contract Size (Draft)
- [Reth Issue #18783](https://github.com/paradigmxyz/reth/issues/18783) - Amsterdam Hardfork Tracking

---

## ACDE Follow-up

Meetings on **Thursdays**. Track agendas and notes at [ethereum/pm](https://github.com/ethereum/pm). Options:

1. **Attend live** - Direct participation
2. **Post-call review** - YouTube + transcript with Claude:
   - Timestamps for specific topics
   - Summary of relevant EIP discussions
   - Track CFI/SFI status changes

[#5769]: https://github.com/lambdaclass/ethrex/pull/5769
[#5970]: https://github.com/lambdaclass/ethrex/pull/5970
[#5973]: https://github.com/lambdaclass/ethrex/pull/5973
[#5996]: https://github.com/lambdaclass/ethrex/pull/5996
[#6020]: https://github.com/lambdaclass/ethrex/pull/6020
[#6024]: https://github.com/lambdaclass/ethrex/pull/6024
[#6074]: https://github.com/lambdaclass/ethrex/pull/6074
[#6104]: https://github.com/lambdaclass/ethrex/pull/6104
[#6118]: https://github.com/lambdaclass/ethrex/pull/6118
[#6128]: https://github.com/lambdaclass/ethrex/pull/6128
[#6149]: https://github.com/lambdaclass/ethrex/pull/6149
[#6201]: https://github.com/lambdaclass/ethrex/pull/6201
[#6209]: https://github.com/lambdaclass/ethrex/issues/6209
[#6210]: https://github.com/lambdaclass/ethrex/issues/6210
[#6211]: https://github.com/lambdaclass/ethrex/issues/6211
[#6212]: https://github.com/lambdaclass/ethrex/issues/6212
[#6213]: https://github.com/lambdaclass/ethrex/issues/6213
[#6214]: https://github.com/lambdaclass/ethrex/issues/6214
