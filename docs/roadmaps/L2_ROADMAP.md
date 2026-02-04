# L2 Roadmap

This document outlines the short- to mid-term roadmap for ethrex L2 development.

> **Note**: This is a living document. Items are organized by category. The order within each section does not necessarily reflect priority.

---

## Status Legend

| Status | Meaning |
|--------|---------|
| **Done** | Completed |
| **In Progress** | Has an active PR |
| **Pending** | Planned, not yet started |
| **Blocked** | Waiting on external dependency |
| **Research** | Exploratory/investigation phase |

---

## 1. zkVM Integration

Core proving infrastructure supporting multiple zkVM backends.

### 1.1 ZisK L2

Full L2 support with STARK→SNARK compression and CPU/GPU proving.

| Item | Status | Issue/PR | Description |
|------|--------|----------|-------------|
| MVP | In Progress | [#6014](https://github.com/lambdaclass/ethrex/pull/6014) | Current STARK + SNARK system integrated with L2 stack |
| v0.16.0 Migration | Pending | — | Migrate to ZisK 0.16.0 once released; benchmark old vs new |
| SDK Migration | Pending | — | Replace compiled binaries with official ZisK SDK once available |
| Aligned Integration | Blocked | — | Enable Aligned + ZisK setup once Aligned supports ZisK |

**Tracker:** [ZisK Integration Tracker](https://github.com/lambdaclass/ethrex/issues/4466)

### 1.2 SP1 Hypercube

Upgrade to SP1 v6 and enable parallel proving for improved throughput.

| Item | Status | Issue/PR | Description |
|------|--------|----------|-------------|
| Version Upgrade (v5 → v6) | Pending | — | Upgrade SP1 SDK to Hypercube; keep existing guest program unchanged |
| Subblocks (L1) | Pending | — | Implement subblock guest program for L1 blocks; enables multi-GPU proving |
| L2 Subbatches | Pending | — | Prover-side batch decomposition: batches → blocks → subblocks |

**Documentation:** [SP1 Hypercube Integration Guide](../prover/sp1_hypercube.md)

**References:**
- [SP1 Hypercube Announcement](https://blog.succinct.xyz/sp1-hypercube/)
- [Real-Time Proving with 16 GPUs](https://blog.succinct.xyz/real-time-proving-16-gpus/)
- [rsp-subblock Reference Implementation](https://github.com/succinctlabs/rsp-subblock)

### 1.3 OpenVM L2

Axiom's modular zkVM framework integration.

| Item | Status | Issue/PR | Description |
|------|--------|----------|-------------|
| MVP (EXEC Mode) | Blocked | [#5509](https://github.com/lambdaclass/ethrex/issues/5509) | Integrate with L2 stack in exec mode (proving takes too long for now) |
| Precompiles | Pending | — | Implement precompiles to reduce proving time |

**Tracker:** [OpenVM Integration Tracker](https://github.com/lambdaclass/ethrex/issues/4461)

---

## 2. Native Rollups

EIP-8079 native rollups where L1 executes L2 state transitions directly.

| Item | Status | Issue/PR | Description |
|------|--------|----------|-------------|
| EXEC Precompile (L1) | Research | — | Implement proof-of-concept EXECUTE precompile per [EIP-8079](https://eips.ethereum.org/EIPS/eip-8079) |
| L2 Adaptation | Research | — | Adapt L2 to use the EXEC precompile for verification |

**Context:** Native rollups use an L1 precompile to verify EVM state transitions, eliminating the need for separate proof systems. This depends on Ethereum protocol changes (target: post-Fusaka).

**References:**
- [EIP-8079: Native rollups](https://eips.ethereum.org/EIPS/eip-8079)
- [Scroll: Native Rollups Research](https://scroll.io/research/native-rollups-promises-and-challenges)

---

## 3. Aligned Layer Integration

Proof aggregation via Aligned Layer for cost-efficient verification.

| Item | Status | Issue/PR | Description |
|------|--------|----------|-------------|
| Hoodi Staging Instance | Pending | — | Long-running L2 node on Hoodi continuously sending proofs to Aligned |
| CI Integration Tests | Pending | — | Automated CI test running L2 in Aligned mode |
| Enable ZisK | Blocked | — | Enable Aligned + ZisK once Aligned supports it |
| Refactors & Improvements | Pending | [#3883](https://github.com/lambdaclass/ethrex/issues/3883), [#6022](https://github.com/lambdaclass/ethrex/issues/6022) | Code cleanup and minor improvements |

**Documentation:** [Aligned Deployment](../l2/deployment/aligned.md)

---

## 4. Prover Infrastructure

Improvements to the proving pipeline.

| Item | Status | Issue/PR | Description |
|------|--------|----------|-------------|
| Distributed Proving | Pending | — | Support multiple provers proving consecutive batches; verify all proofs in one tx |
| Elastic Prover Net | Pending | — | Elastic prover network that scales based on demand |
| Refactors & Cleanup | Pending | [#4170](https://github.com/lambdaclass/ethrex/issues/4170), [#4327](https://github.com/lambdaclass/ethrex/issues/4327), [#4509](https://github.com/lambdaclass/ethrex/issues/4509), [#3768](https://github.com/lambdaclass/ethrex/issues/3768), [#4473](https://github.com/lambdaclass/ethrex/issues/4473) | Code cleanup and minor improvements |

---

## 5. Sequencer

Core sequencer improvements.

| Item | Status | Issue/PR | Description |
|------|--------|----------|-------------|
| Review Current Logic | Pending | [#5059](https://github.com/lambdaclass/ethrex/issues/5059), [#4398](https://github.com/lambdaclass/ethrex/issues/4398), [#5325](https://github.com/lambdaclass/ethrex/issues/5325), [#3742](https://github.com/lambdaclass/ethrex/issues/3742) | Review L1 fee logic, privileged tx ordering, and block timestamps |
| Refactor & Cleanup | Pending | [#4321](https://github.com/lambdaclass/ethrex/issues/4321), [#5154](https://github.com/lambdaclass/ethrex/issues/5154), [#5719](https://github.com/lambdaclass/ethrex/issues/5719), [#4510](https://github.com/lambdaclass/ethrex/issues/4510), [#3754](https://github.com/lambdaclass/ethrex/issues/3754), [#4318](https://github.com/lambdaclass/ethrex/issues/4318), [#5607](https://github.com/lambdaclass/ethrex/issues/5607), [#3755](https://github.com/lambdaclass/ethrex/issues/3755), [#5003](https://github.com/lambdaclass/ethrex/issues/5003), [#3721](https://github.com/lambdaclass/ethrex/issues/3721) | Code cleanup and minor improvements |

**Tracker:** [Code Simplification Tracker](https://github.com/lambdaclass/ethrex/issues/3744)

---

## 6. Contracts

Smart contract improvements.


| Item | Status | Issue/PR | Description |
|------|--------|----------|-------------|
| Normalize Contract Errors | Pending | [#6098](https://github.com/lambdaclass/ethrex/issues/6098) | Standardize error codes in OnChainProposer contracts |
| Custom Contract Errors | Pending | [#4196](https://github.com/lambdaclass/ethrex/issues/4196) | Replace `require(..., string)` with custom errors |
| RIP-7740 Tracking | Pending | [#4552](https://github.com/lambdaclass/ethrex/issues/4552) | CreateX and factory deployment |
| Deploy create2 Factories | Pending | [#4526](https://github.com/lambdaclass/ethrex/issues/4526) | All factories from RIP-7740 |
| RIP-7875 Bridge Address | Pending | [#4527](https://github.com/lambdaclass/ethrex/issues/4527) | Change default bridge to `0x1ff` |
| Canonical WETH9 | Pending | [#4553](https://github.com/lambdaclass/ethrex/issues/4553), [#4556](https://github.com/lambdaclass/ethrex/issues/4556) | Deploy canonical WETH9 contract |
| WETH symbol()/name() | Pending | [#5398](https://github.com/lambdaclass/ethrex/issues/5398) | Add missing ERC methods to WETH |

---

## 7. Security & Audit

Security hardening and ongoing audit work.

| Item | Status | Issue/PR | Description |
|------|--------|----------|-------------|
| External Security Audit | In Progress | — | External audit in progress; addressing comments as received |
| Storage Layout Checks | Pending | [#5859](https://github.com/lambdaclass/ethrex/issues/5859) | OpenZeppelin upgrade safety workflow |
| Arithmetic Overflow Checks | Pending | [#5295](https://github.com/lambdaclass/ethrex/issues/5295) | Enable clippy arithmetic side effects lint |
| Gas Limit Validation | Pending | [#6053](https://github.com/lambdaclass/ethrex/issues/6053) | Validate gas_limit covers L1 DA fee (griefing prevention) |

---

## 8. Based Rollups

Decentralized sequencing via L1-driven leader election.

| Item | Status | Issue/PR | Description |
|------|--------|----------|-------------|
| MVP | Done | — | Round-robin sequencing, SequencerRegistry, State Updater, Block Fetcher |
| Redesign | TBD | — | New roadmap to be defined |

**Documentation:** [Based Sequencing Fundamentals](../l2/fundamentals/based.md)

---

## Appendix: Sections Not Included

The following sections from the previous roadmap are not part of the core focus but may be relevant:

### Testing

| Item | Status | Issue/PR |
|------|--------|----------|
| L2 Hook Unit Tests | In Progress | [#5057](https://github.com/lambdaclass/ethrex/issues/5057), [#6051](https://github.com/lambdaclass/ethrex/pull/6051) |
| Forced Inclusion Test | Pending | [#5669](https://github.com/lambdaclass/ethrex/issues/5669) |
| Refactor Integration Tests | Pending | [#4290](https://github.com/lambdaclass/ethrex/issues/4290), [#3862](https://github.com/lambdaclass/ethrex/issues/3862) |
| Load Test | Pending | [#4243](https://github.com/lambdaclass/ethrex/issues/4243) |

### Monitor & Developer UX

| Item | Status | Issue/PR |
|------|--------|----------|
| Anvil-like L1 UX | Pending | — |
| Reduce RPC Call Volume | Pending | [#3824](https://github.com/lambdaclass/ethrex/issues/3824) |
| WebSocket RPC | Pending | [#4898](https://github.com/lambdaclass/ethrex/issues/4898) |

### Stage Compliance

| Stage | Status | Blocker |
|-------|--------|---------|
| **Stage 0** | Met | — |
| **Stage 1** | Not Met | Forced inclusion mechanism for withdrawals |
| **Stage 2** | Not Met | Permissionless proving, 30-day exit window |

**Documentation:** [Rollup Stages](../l2/stages.md)

---

## References

- [Main Project Roadmap](/ROADMAP.md)
- [Based Sequencing Design](../l2/fundamentals/based.md)
- [Stage Compliance Analysis](../l2/stages.md)
- [zkVM Comparison Benchmarks](../l2/bench/zkvm_comparison.md)
- [SP1 Hypercube Integration Guide](../prover/sp1_hypercube.md)
