# Forks Team Roadmap - ethrex

## Amsterdam / Glamsterdam → Mainnet June 2026

---

## Current Implementation Status

### Core Devnet EIPs (Priority)

| EIP | Title | Code Status | Tests | devnet-bal | SFI/CFI | Owner |
|-----|-------|-------------|-------|------------|---------|-------|
| **7928** | Block-Level Access Lists | ⚠️ Types + engine_newPayloadV5 merged; execution integration in progress (`eip_7928_tracking` branch) | Unit tests passing | ✅ | SFI | Edgar |
| **7708** | ETH Transfers Emit Logs | ✅ Merged to main (PR #6074, #6104) | 100+ tests | ✅ | CFI | Edgar |
| **7778** | Block Gas Accounting without Refunds | ✅ Merged to main (PR #5996) | 7 unit tests in `eip7778_tests.rs` | ✅ | CFI | Edgar |
| **7843** | SLOTNUM Opcode | ⚠️ Review comments addressed, pending merge (`implement-eip7843` branch) | ~7 tests (skipped) | ✅ | CFI | Esteve |
| **8024** | DUPN/SWAPN/EXCHANGE | ✅ Merged to main (PR #5970, bugfix #6118) | ~400 tests (skipped due to gas cost deps) | ✅ | CFI | Esteve |

### Gas Repricing EIPs (New - not on devnet-bal yet)

| EIP | Title | Code Status | Nethermind | Reth | SFI/CFI |
|-----|-------|-------------|------------|------|---------|
| **2780** | Reduce Intrinsic Transaction Gas | 🔴 Not implemented (21000 → 4500) | 🔴 | 🔴 | CFI |
| **7904** | General Repricing | 🔴 Not implemented | ⚠️ PR #9619 (Draft) | 🔴 | CFI |
| **7954** | Increase Max Contract Size | 🔴 Not implemented (24KiB → 32KiB) | ⚠️ PR #8760 (Draft) | 🔴 | CFI |
| **7976** | Increase Calldata Floor Cost | 🔴 Not implemented | 🔴 | 🔴 | CFI |
| **7981** | Increase Access List Cost | 🔴 Not implemented | 🔴 | 🔴 | CFI |
| **8037** | State Creation Gas Cost Increase | 🔴 Not implemented | 🔴 | 🔴 | CFI |
| **8038** | State-Access Gas Cost Update | 🔴 Not implemented | 🔴 | 🔴 | CFI |

### Other Amsterdam EIPs

| EIP | Title | Code Status | Nethermind | Reth | SFI/CFI |
|-----|-------|-------------|------------|------|---------|
| **7997** | Deterministic Factory Predeploy | 🔴 Not implemented | 🔴 | 🔴 | CFI |
| **8070** | Sparse Blobpool | 🔴 Not implemented (ROADMAP.md: Priority —) | 🔴 | 🔴 | CFI |
| **7610** | Revert Creation on Non-empty Storage | 🔴 Not implemented | 🔴 | 🔴 | PFI |
| **7872** | Max Blob Flag for Local Builders | 🔴 Not implemented | 🔴 | 🔴 | PFI |

---

## February 5 Status Update

### Merged ✅
- [x] **EIP-7778** (Gas Accounting) - PR #5996 merged → Edgar
- [x] **EIP-7708** (ETH Transfer Logs) - PR #6074, #6104 merged → Edgar
- [x] **EIP-8024** (DUPN/SWAPN/EXCHANGE) - PR #5970 merged, bugfix #6118 merged → Esteve

### Pending Merge
- [ ] **EIP-7843** (SLOTNUM) - Review comments addressed on `implement-eip7843` branch → Esteve

### In Progress
- [ ] **Complete EIP-7928 integration** (BAL part 2) - execution hook needed → Edgar
  - Types + `engine_newPayloadV5` merged (PR #6020)
  - Work continues on `eip_7928_tracking` branch
  - Recent: removed dead code, fixed comments (ce8754cf3)
  - Missing: block execution integration to populate the access list

### Documentation
- [ ] **Update `docs/eip.md`** - Mark EIP-7708, EIP-7778, and EIP-8024 as "Supported [x]"

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

## EIP Implementation Categories

### Merged to Main ✅
1. **EIP-7708** - ETH Transfer Logs (PR #6074, #6104)
2. **EIP-7778** - Gas Accounting (PR #5996)
3. **EIP-8024** - DUPN/SWAPN/EXCHANGE (PR #5970, bugfix #6118)

### Almost There (needs finishing)
1. **EIP-7843** - SLOTNUM (review comments addressed, pending merge)
2. **EIP-7928** - BAL (types merged, execution integration in progress on `eip_7928_tracking`)

### Not Started (wave 2)
1. **Gas repricing bundle**: EIP-2780, 7904, 7976, 7981, 8037, 8038
2. **Contract changes**: EIP-7954 (max size), EIP-7997 (factory predeploy)
3. **Network**: EIP-8070 (sparse blobpool), EIP-7872 (max blob flag)
4. **Edge case**: EIP-7610 (non-empty storage revert)

---

## Client Implementation Comparison (Wave 2 CFI EIPs)

> Last updated: February 5, 2026

### Summary

The CFI EIPs are **"considered for inclusion"** but have **NOT been officially scheduled** for Glamsterdam. The officially scheduled EIPs are:
- **EIP-7732**: Enshrined Proposer-Builder Separation (ePBS)
- **EIP-7928**: Block-Level Access Lists (BALs)

### Nethermind Status

| EIP | Status | Notes |
|-----|--------|-------|
| **7904** | ⚠️ Draft PR #9619 | Opened Oct 31, 2025; implements opcode repricing |
| **7954** | ⚠️ Draft PR #8760 | Opened Jun 10, 2025; related to Berlin Interop devnet-2 |
| Others | 🔴 Not started | No PRs or issues found |

### Reth Status

| EIP | Status | Notes |
|-----|--------|-------|
| All CFI EIPs | 🔴 Not started | Focus is on EIP-7732 (ePBS) and EIP-7928 (BALs) |
| Tracking | Issue #18783 | Amsterdam hardfork tracking issue |

### Priority Recommendation

| Priority | EIPs | Rationale |
|----------|------|-----------|
| **Low** | 2780, 7976, 7981, 8037, 8038, 7997, 8070 | No client has started; not officially scheduled |
| **Medium** | 7904, 7954 | Nethermind has draft PRs; may gain traction |

### Recommended Action

1. **Complete SFI EIPs first**: Finish EIP-7928 (BAL) and merge EIP-7843 (SLOTNUM)
2. **Monitor CFI EIPs**: Wait for official inclusion decisions at ACDE calls
3. **If getting ahead**: EIP-7954 is simplest (just a constant change: 24KiB → 32KiB)

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
| Merge EIP-7843 branch | `origin/implement-eip7843` | High | Review done |
| Complete BAL execution integration | `origin/eip_7928_tracking` | High | In progress |
| Enable Amsterdam EIP tests | `tooling/ef_tests/blockchain/tests/all.rs` | Medium | Blocked by EIP impls |
| Address 31k skipped Amsterdam legacy tests | `SKIPPED_AMSTERDAM` in ef_tests | Medium | Blocked by all EIPs |

---

## Glossary

| Acronym | Meaning |
|---------|---------|
| **SFI** | Scheduled for Inclusion - Will be in the fork |
| **CFI** | Considered for Inclusion - Likely, under discussion |
| **DFI** | Declined for Inclusion - Won't be included |
| **PFI** | Proposed for Inclusion - Proposed |
| **BAL** | Block-Level Access Lists (EIP-7928) |

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

Meetings on **Thursdays**. Options:

1. **Attend live** - Direct participation
2. **Post-call review** - YouTube + transcript with Claude:
   - Timestamps for specific topics
   - Summary of relevant EIP discussions
   - Track CFI/SFI status changes
