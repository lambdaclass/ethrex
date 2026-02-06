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
| **7928** | Block-Level Access Lists | ⚠️ Types + engine_newPayloadV5 merged; execution integration PR [#6024] open | Unit tests passing | ✅ | SFI | Edgar |
| **7708** | ETH Transfers Emit Logs | ✅ Merged ([#6074], fix [#6104]) · [exec-specs tracking](https://github.com/ethereum/execution-specs/issues/1879) | 100+ tests | ✅ | CFI | Edgar |
| **7778** | Block Gas Accounting without Refunds | ✅ Merged ([#5996], fix [#6128]) · [exec-specs tracking](https://github.com/ethereum/execution-specs/issues/1940) | 7 unit tests in `eip7778_tests.rs` | ✅ | CFI | Edgar |
| **7843** | SLOTNUM Opcode | ✅ Merged ([#5973]) · [exec-specs tracking](https://github.com/ethereum/execution-specs/issues/2028) | ~7 tests (skipped) | ✅ | CFI | Esteve |
| **8024** | DUPN/SWAPN/EXCHANGE | ✅ Merged ([#5970], fix [#6118]) · [exec-specs tracking](https://github.com/ethereum/execution-specs/issues/1942) | ~400 tests (skipped due to gas cost deps) | ✅ | CFI | Esteve |

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

> **Priority note:** Gas repricing EIPs are **low priority** — no other client has started them. EIP-7904 and EIP-7954 are **medium priority** (Nethermind has draft PRs). Focus on finishing SFI EIPs first (EIP-7928 BAL), then monitor CFI decisions at ACDE calls. If getting ahead, EIP-7954 is simplest (just a constant change: 24KiB → 32KiB).

### Other Amsterdam EIPs

| EIP | Title | Code Status | Nethermind | Reth | SFI/CFI |
|-----|-------|-------------|------------|------|---------|
| **7997** | Deterministic Factory Predeploy | 🔴 Not implemented · [exec-specs tracking](https://github.com/ethereum/execution-specs/issues/1988) | 🔴 | 🔴 | CFI |
| **8070** | Sparse Blobpool | 🔴 Not implemented (ROADMAP.md: Priority —) | 🔴 | 🔴 | CFI |
| **7610** | Revert Creation on Non-empty Storage | 🔴 Not implemented | 🔴 | 🔴 | PFI |
| **7872** | Max Blob Flag for Local Builders | ✅ Merged ([#5769]) | 🔴 | 🔴 | PFI |

---

## February 6 Status Update

### Merged ✅
- [x] **EIP-7778** (Gas Accounting) - [#5996](https://github.com/lambdaclass/ethrex/pull/5996), fix [#6128](https://github.com/lambdaclass/ethrex/pull/6128) merged → Edgar
- [x] **EIP-7708** (ETH Transfer Logs) - [#6074](https://github.com/lambdaclass/ethrex/pull/6074), fix [#6104](https://github.com/lambdaclass/ethrex/pull/6104) merged → Edgar
- [x] **EIP-8024** (DUPN/SWAPN/EXCHANGE) - [#5970](https://github.com/lambdaclass/ethrex/pull/5970), bugfix [#6118](https://github.com/lambdaclass/ethrex/pull/6118) merged → Esteve
- [x] **EIP-7843** (SLOTNUM) - [#5973](https://github.com/lambdaclass/ethrex/pull/5973) merged → Esteve
- [x] **EIP-7872** (Max Blob Flag) - [#5769](https://github.com/lambdaclass/ethrex/pull/5769) merged → Edgar

### In Progress
- [ ] **Complete EIP-7928 integration** (BAL part 2) - execution hook needed → Edgar
  - Types + `engine_newPayloadV5` merged (PR [#6020](https://github.com/lambdaclass/ethrex/pull/6020))
  - Execution integration PR [#6024](https://github.com/lambdaclass/ethrex/pull/6024) open
  - Recent: removed dead code, fixed comments (ce8754cf3)
  - Missing: block execution integration to populate the access list

### Documentation
- [ ] **Update `docs/eip.md`** - Mark EIP-7708, EIP-7778, EIP-8024, EIP-7843, and EIP-7872 as "Supported [x]"

### Testing
- [ ] Update hive tests for Amsterdam
- [ ] Monitor EEST test changes / EIP spec changes
- [ ] Address ~31,000 skipped Amsterdam legacy tests in `tooling/ef_tests/blockchain/tests/all.rs`
- [ ] Enable EIP-specific tests as implementations complete (currently all skipped in `SKIPPED_AMSTERDAM`)

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

## Technical Debt / Action Items

| Item | Location | Priority | Status |
|------|----------|----------|--------|
| Update `docs/eip.md` supported status | `docs/eip.md` | High | Pending |
| Complete BAL execution integration | PR [#6024](https://github.com/lambdaclass/ethrex/pull/6024) | High | In progress |
| Enable Amsterdam EIP tests | `tooling/ef_tests/blockchain/tests/all.rs` | Medium | Blocked by EIP impls |
| Address 31k skipped Amsterdam legacy tests | `SKIPPED_AMSTERDAM` in ef_tests | Medium | Blocked by all EIPs |

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
