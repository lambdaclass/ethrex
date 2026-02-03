# Forks Team Roadmap - ethrex

## Amsterdam / Glamsterdam → Mainnet June 2026

---

## Current Implementation Status

### Core Devnet EIPs (Priority)

| EIP | Title | Code Status | Tests | devnet-bal | SFI/CFI | Owner |
|-----|-------|-------------|-------|------------|---------|-------|
| **7928** | Block-Level Access Lists | ⚠️ Framework only (`block_access_list.rs`) - needs execution integration | Unit tests passing | ✅ | SFI | Edgar |
| **7708** | ETH Transfers Emit Logs | ✅ Fully implemented (`constants.rs`, `utils.rs`) | 100+ tests | ✅ | CFI | Edgar |
| **7778** | Block Gas Accounting without Refunds | 🔴 Not implemented | ~24 tests (skipped) | ✅ | CFI | Edgar |
| **7843** | SLOTNUM Opcode | ⚠️ Opcode 0x4b reserved, impl incomplete | ~7 tests | ✅ | CFI | Esteve |
| **8024** | DUPN/SWAPN/EXCHANGE | ✅ Fully implemented (`dup.rs`, `exchange.rs`) | ~400 tests | ✅ | CFI | Esteve |

### Gas Repricing EIPs (New - not on devnet-bal yet)

| EIP | Title | Code Status | SFI/CFI |
|-----|-------|-------------|---------|
| **2780** | Reduce Intrinsic Transaction Gas | 🔴 Not implemented (21000 → 4500) | CFI |
| **7904** | General Repricing | 🔴 Not implemented | CFI |
| **7954** | Increase Max Contract Size | 🔴 Not implemented (24KiB → 32KiB) | CFI |
| **7976** | Increase Calldata Floor Cost | 🔴 Not implemented | CFI |
| **7981** | Increase Access List Cost | 🔴 Not implemented | CFI |
| **8037** | State Creation Gas Cost Increase | 🔴 Not implemented | CFI |
| **8038** | State-Access Gas Cost Update | 🔴 Not implemented | CFI |

### Other Amsterdam EIPs

| EIP | Title | Code Status | SFI/CFI |
|-----|-------|-------------|---------|
| **7997** | Deterministic Factory Predeploy | 🔴 Not implemented | CFI |
| **8070** | Sparse Blobpool | 🔴 Not implemented (ROADMAP.md: Priority —) | CFI |
| **7610** | Revert Creation on Non-empty Storage | 🔴 Not implemented | PFI |
| **7872** | Max Blob Flag for Local Builders | 🔴 Not implemented | PFI |

---

## February 1-14

### Merge Ready
- [ ] **Merge EIP-7778** (Gas Accounting) → Edgar
- [ ] **Fix CI and merge EIP-7843** (SLOTNUM) → Esteve

### In Progress
- [ ] **Complete EIP-7928 integration** (BAL part 2) - execution hook needed → Edgar
  - Framework exists at `crates/common/types/block_access_list.rs`
  - `block_access_list_hash` field exists in `BlockHeader`
  - Missing: block execution integration to populate the list

### Documentation
- [ ] **Update `docs/eip.md`** - Mark EIP-7708 and EIP-8024 as "Supported [x]"

### Testing
- [ ] Update hive tests for Amsterdam
- [ ] Monitor EEST test changes / EIP spec changes
- [ ] Address ~31,000 skipped Amsterdam legacy tests

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

### Ready to Ship (code complete)
1. **EIP-7708** - ETH Transfer Logs
2. **EIP-8024** - DUPN/SWAPN/EXCHANGE

### Almost There (needs finishing)
1. **EIP-7778** - Gas Accounting (pending merge)
2. **EIP-7843** - SLOTNUM (CI issues)
3. **EIP-7928** - BAL (framework done, needs execution integration)

### Not Started (wave 2)
1. **Gas repricing bundle**: EIP-2780, 7904, 7976, 7981, 8037, 8038
2. **Contract changes**: EIP-7954 (max size), EIP-7997 (factory predeploy)
3. **Network**: EIP-8070 (sparse blobpool), EIP-7872 (max blob flag)
4. **Edge case**: EIP-7610 (non-empty storage revert)

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

| Item | Location | Priority |
|------|----------|----------|
| Update `docs/eip.md` supported status | `docs/eip.md:9-13` | High |
| Remove EIP-7843 from partial status | `crates/vm/levm/src/opcodes.rs` | Medium |
| Complete BAL execution integration | `crates/blockchain/` | High |
| Address 31k skipped Amsterdam tests | `tooling/ef_tests/` | Medium |

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

---

## ACDE Follow-up

Meetings on **Thursdays**. Options:

1. **Attend live** - Direct participation
2. **Post-call review** - YouTube + transcript with Claude:
   - Timestamps for specific topics
   - Summary of relevant EIP discussions
   - Track CFI/SFI status changes
