# Native Rollups Integration Guide

> **Last updated:** 2026-02-04

---

## Overview

Native rollups are L2 solutions that leverage Ethereum's execution environment directly for state transition verification. Instead of building custom proof systems (fraud proofs or SNARK verifiers), native rollups use an `EXECUTE` precompile that recursively calls Ethereum's state transition function (STF).

**Key insight:** Native rollups are "programmable execution shards" that wrap the EXECUTE precompile within a derivation function for system logic like sequencing and bridging.

---

## Why Native Rollups?

### Current Rollup Challenges

Current EVM-equivalent rollups face a fundamental trade-off:

1. **Complex proof systems** — Thousands of lines of code for fraud proofs or SNARK verifiers, duplicating Ethereum's work and increasing bug surface
2. **Governance divergence** — Cannot follow Ethereum's governance automatically, requiring extended exit windows (7+ days)
3. **Maintenance burden** — Upgrades require governance interventions, preventing migration to Stage 2 decentralization
4. **Exit window limitations** — Even with exit windows, not all application types are protected

### Native Rollup Benefits

| Benefit | Description |
|---------|-------------|
| **Full L1 Security** | Trustless rollups that fully inherit L1 security properties without governance councils |
| **Governance-Free Upgrades** | Automatically synchronize with L1 hard forks; no governance interventions needed |
| **Reduced Complexity** | Minimal Solidity implementation instead of complex proof systems |
| **Real-Time Settlement** | Enables composability without requiring ultra-low-latency proving |
| **Inherent Reliability** | Any bugs in EXECUTE would also exist in Ethereum itself, ensuring fixes come from the broader community |

---

## The EXECUTE Precompile (EIP-8079)

The core of native rollups is the `EXECUTE` precompile, which exposes Ethereum's state transition function.

### Inputs

| Parameter | Description |
|-----------|-------------|
| `pre_state_root` | Starting state root |
| `post_state_root` | Expected ending state root |
| `trace` | Execution trace with transactions and state access proofs |
| `gas_used` | Computational resources consumed |

### Behavior

The precompile returns `true` if:
> "The stateless execution of `trace` starting from `pre_state_root` ends at `post_state_root` using exactly the specified gas amount."

### Key Properties

- **Rejects blob-carrying transactions** — Blob data not available for re-execution
- **Anchoring system transaction** — Enables L1→L2 messaging via predeploy contract
- **Burned fees tracking** — `burned_fees` header field allows rollups to collect (rather than burn) base fees

### Architecture Flow

```
L2 Transactions → State Access Proofs → Trace Data
                                           ↓
                              EXECUTE Precompile
                                           ↓
                    (Verify state transition)
                                           ↓
                         Update L1 State Root
```

---

## Implementation Phases

Native rollups will be implemented in two phases on Ethereum L1:

### Phase 1: Re-Execution Enforcement

Validators naively re-execute traces to verify correctness.

**Properties:**
- No state growth or bandwidth overhead for validators (beyond trace data)
- Parallelizable across CPU cores
- Requires explicit trace copies (no blob sampling via DAS)

**Bandwidth overhead:** ~127KB/s per Mgas per rollup

### Phase 2: SNARK Verification

Zero-knowledge proofs verified offchain by individual validators.

**Properties:**
- Proofs shared via P2P (not enshrined onchain)
- One-slot delayed execution provides proving window
- Validators choose preferred zkEL (zkEVM) clients independently
- No specific proof system enshrined in consensus

---

## Customization Capabilities

Native rollups can customize:

| Component | Options |
|-----------|---------|
| **Bridging** | Custom withdrawal mechanisms, deposit handling |
| **Sequencing** | Permissioned, based (L1-driven), shared |
| **Gas Token** | ETH, custom tokens, sponsored transactions |
| **Governance** | DAO, multisig, immutable |
| **Coinbase** | Custom fee collection addresses |

### Limitations

Rollups **cannot** support within the exposed STF:
- Custom opcodes
- Custom precompiles
- Custom transaction types

Migration effort is proportional to existing customizations.

---

## Native vs Based vs Ultra Sound

These concepts are orthogonal:

| Concept | Addresses | Description |
|---------|-----------|-------------|
| **Native** | Execution verification | Uses L1's EXECUTE precompile for state transitions |
| **Based** | Sequencing | L1 determines transaction ordering |
| **Ultra Sound** | Both | Combines native execution with based sequencing |

**ethrex context:** We can pursue native rollups independently of based sequencing. However, combining both (ultra sound rollups) provides maximum decentralization and security.

---

## Synchronous Composability with Preconfirmations

For rollups that want both low latency and L1 composability, a hybrid approach combines preconfirmations with based blocks.

### Three L2 Block Types

| Block Type | Description |
|------------|-------------|
| **Regular sequenced** | Low latency, requires sequencer certificate |
| **Slot-ending sequenced** | Includes validity message permitting based block construction |
| **Based blocks** | Permissionless construction, only atop slot-ending blocks |

### Trade-offs

- **Latency:** Normal delays remain minimal; longest delays during proposer absence
- **Reversion:** L2 must tolerate reversions if L1 reverts (based block failures cascade)
- **Proving:** Atomicity demands L2 state verification within single L1 slots ("streaming prover")

---

## Technical Challenges

### 1. State Transition Divergence

Current L2s use transaction types and precompiles absent from L1:

- **ethrex impact:** Our L2 uses custom transaction types for deposits. Migration path needed.
- **Mitigation:** Use anchoring system transactions instead of custom tx types.

### 2. Composability Requirements

Extension features and standard EVM functionality must integrate within single transactions.

- **ethrex impact:** Any custom precompiles would need to be upstreamed to L1 or removed.

### 3. State Tree Coordination

Future L1 state tree migrations (e.g., Verkle trees) require synchronized upgrades.

- **ethrex impact:** Native rollups automatically inherit these changes (benefit).

### 4. Trace Data Overhead

Stateless execution traces add bandwidth requirements during re-execution phase.

- **ethrex impact:** ~127KB/s per Mgas. Need to evaluate operational costs.

---

## ethrex Integration Roadmap

### Phase 1: Research & Specification (Current)

**Goal:** Understand EIP-8079 deeply and identify integration requirements.

**Tasks:**
- [ ] Analyze current L2 deviations from vanilla EVM
- [ ] Document custom transaction types and their migration paths
- [ ] Evaluate trace data generation requirements
- [ ] Track EIP-8079 progress and timeline

**Deliverables:**
- Gap analysis: ethrex L2 vs EIP-8079 requirements
- Migration plan for custom features

### Phase 2: EXECUTE Precompile PoC (ethrex L1)

**Goal:** Implement proof-of-concept EXECUTE precompile in ethrex L1 node.

**Tasks:**
- [ ] Implement EXECUTE precompile per EIP-8079 spec
- [ ] Add trace generation for L2 blocks
- [ ] Create test harness for verification
- [ ] Benchmark re-execution overhead

**Deliverables:**
- EXECUTE precompile implementation (feature-gated)
- Trace generation module
- Benchmark report

### Phase 3: L2 Adaptation

**Goal:** Adapt ethrex L2 to use EXECUTE precompile for verification.

**Tasks:**
- [ ] Replace custom tx types with anchoring transactions
- [ ] Implement L1→L2 messaging via predeploy contract
- [ ] Update bridge contracts for native verification
- [ ] Remove zkVM proof generation (optional fallback)

**Deliverables:**
- Native rollup mode for L2
- Updated bridge contracts
- Integration tests

### Phase 4: Devnet Deployment

**Goal:** Run native rollup on private devnet for testing.

**Tasks:**
- [ ] Deploy modified L1 with EXECUTE precompile
- [ ] Deploy L2 in native mode
- [ ] Run stress tests and security review
- [ ] Document operational requirements

**Deliverables:**
- Devnet deployment guide
- Operational runbook
- Security assessment

---

## Timeline Dependencies

Native rollups depend on Ethereum protocol changes:

| Milestone | Status | Description |
|-----------|--------|-------------|
| EIP-8079 | Draft | EIP proposed Nov 2025; not yet scheduled for a fork |
| Fusaka | Live (Dec 2025) | Focused on PeerDAS; does not include EIP-8079 |
| Glamsterdam | H1 2026 | Confirmed: ePBS, BALs; EIP-8079 not included |
| Hegota | Late 2026 | Still being scoped; EIP-8079 inclusion TBD |

**Recommendation:** Begin Phase 1-2 now to be ready when L1 support lands. Phase 3-4 can proceed on devnets before mainnet availability.

---

## Code Structure (Proposed)

```
crates/
├── vm/
│   └── precompiles/
│       └── execute.rs          # EXECUTE precompile implementation
├── l2/
│   └── native/
│       ├── mod.rs
│       ├── trace.rs            # Trace generation for L2 blocks
│       ├── anchor.rs           # Anchoring transaction handling
│       └── verifier.rs         # Native verification integration
└── common/
    └── types/
        └── trace.rs            # Trace data structures
```

---

## References

### EIPs and Specifications

- [EIP-8079: Native Rollups](https://eips.ethereum.org/EIPS/eip-8079)

### Research Posts

- [Native Rollups: Superpowers from L1 Execution](https://ethresear.ch/t/native-rollups-superpowers-from-l1-execution/21517) — Justin Drake's original proposal
- [Combining Preconfirmations with Based Rollups](https://ethresear.ch/t/combining-preconfirmations-with-based-rollups-for-synchronous-composability/23863) — Vitalik's synchronous composability design

### Educational Resources

- [L2Beat Native Rollups Introduction](https://native-rollups.l2beat.com/introduction.html)
- [Scroll: Native Rollups Research](https://scroll.io/research/native-rollups-promises-and-challenges)

### Related ethrex Documentation

- [Based Sequencing Fundamentals](../l2/fundamentals/based.md)
- [L2 Roadmap](../roadmaps/L2_ROADMAP.md)
