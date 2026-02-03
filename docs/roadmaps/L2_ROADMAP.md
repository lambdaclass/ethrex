# L2 Roadmap

This document outlines the short- to mid-term roadmap for ethrex L2 development.

> **Note**: This is a living document. Items are organized by category with relative priority indicated by order within each section.

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

| Item | Status | Issue/PR | Description |
|------|--------|----------|-------------|
| ZisK Backend | In Progress | [#6014](https://github.com/lambdaclass/ethrex/pull/6014) | Full L2 support with STARK→SNARK compression, CPU/GPU proving |
| ZisK Precompiles | Pending | [#4469](https://github.com/lambdaclass/ethrex/issues/4469) | Use ZisK-optimized precompiles (MODEXP, etc.) |
| SP1 Hypercube | Pending | — | Next-gen SP1 for real-time proving. Enables native rollups and synchronous composability |
| GuestProgramState Validation | Pending | — | Make witness validation stricter: error on missing data instead of silent defaults |
| RISC0 c-kzg Patch | Pending | [#4905](https://github.com/lambdaclass/ethrex/issues/4905) | Bump patch to avoid feature flag for c-kzg |
| OpenVM Toolchain | Blocked | [#5509](https://github.com/lambdaclass/ethrex/issues/5509) | Installation currently failing |
| RISC0 Precompile Test | Pending | [#3916](https://github.com/lambdaclass/ethrex/issues/3916) | Test if risc0 precompiles work without unstable feature |

**Trackers:**
- [ZisK Integration Tracker](https://github.com/lambdaclass/ethrex/issues/4466)
- [OpenVM Integration Tracker](https://github.com/lambdaclass/ethrex/issues/4461)

---

## 2. Native Rollups

Research and POC for EIP-8079 native rollups where L1 executes L2 state transitions directly.

| Item | Status | Issue/PR | Description |
|------|--------|----------|-------------|
| EXEC Precompile POC | Research | — | Implement proof-of-concept for [EIP-8079](https://eips.ethereum.org/EIPS/eip-8079) EXECUTE precompile |

**Context**: Native rollups use an L1 precompile to verify EVM state transitions, eliminating the need for separate proof systems. This depends on Ethereum protocol changes (target: post-Fusaka).

**References:**
- [EIP-8079: Native rollups](https://eips.ethereum.org/EIPS/eip-8079)
- [Scroll: Native Rollups Research](https://scroll.io/research/native-rollups-promises-and-challenges)

---

## 3. Based Rollups

Decentralized sequencing via L1-driven leader election.

| Item | Status | Issue/PR | Description |
|------|--------|----------|-------------|
| Milestone 1: MVP | Done | — | Round-robin sequencing, SequencerRegistry, State Updater, Block Fetcher |
| Dutch Auction | Pending | — | Ticket-based sequencer election |
| P2P Gossiping | Pending | — | Milestone 2: Node-to-node block/batch messaging |
| Testnet + Dashboard | Pending | — | Milestone 3: Public testnet with monitoring |
| Timelock for Based | Pending | [#5704](https://github.com/lambdaclass/ethrex/issues/5704) | Introduce Timelock to based rollup contracts |
| Health Endpoints | Pending | [#4508](https://github.com/lambdaclass/ethrex/issues/4508) | `/health` for based components |

**Documentation:** [Based Sequencing Fundamentals](./fundamentals/based.md)

---

## 4. Security & Audit

Security hardening and ongoing audit work.

| Item | Status | Issue/PR | Description |
|------|--------|----------|-------------|
| Security Audit | In Progress | — | External audit in progress; addressing comments as received |
| Normalize Contract Errors | Pending | [#6098](https://github.com/lambdaclass/ethrex/issues/6098) | Standardize error codes in OnChainProposer contracts |
| Storage Layout Checks | Pending | [#5859](https://github.com/lambdaclass/ethrex/issues/5859) | OpenZeppelin upgrade safety workflow |
| Arithmetic Overflow Checks | Pending | [#5295](https://github.com/lambdaclass/ethrex/issues/5295) | Enable clippy arithmetic side effects lint |
| Custom Contract Errors | Pending | [#4196](https://github.com/lambdaclass/ethrex/issues/4196) | Replace `require(..., string)` with custom errors |
| Gas Limit Validation | Pending | [#6053](https://github.com/lambdaclass/ethrex/issues/6053) | Validate gas_limit covers L1 DA fee (griefing prevention) |

---

## 5. Aligned Layer Integration

Proof aggregation via Aligned Layer for cost-efficient verification.

| Item | Status | Issue/PR | Description |
|------|--------|----------|-------------|
| Unify Aligned Logic | Pending | [#3883](https://github.com/lambdaclass/ethrex/issues/3883) | Refactor l1_proof_verifier as GenServer with aligned-specific logic |
| Auto-update from_block | Blocked | [#6022](https://github.com/lambdaclass/ethrex/issues/6022) | Blocked: Aligned lacks mechanism to know which block contains the proof |
| Hoodi Staging Instance | Pending | — | Long-running L2 node on Hoodi continuously sending proofs to Aligned |
| CI Integration Test | Pending | — | Automated CI test running L2 in Aligned mode |

**Documentation:** [Aligned Deployment](./deployment/aligned.md)

---

## 6. Prover Infrastructure

Improvements to the proving pipeline.

| Item | Status | Issue/PR | Description |
|------|--------|----------|-------------|
| Handle Prover Panic | Pending | [#4170](https://github.com/lambdaclass/ethrex/issues/4170) | Add panic handling and retry mechanism |
| Skip Empty Block Proving | Pending | [#4327](https://github.com/lambdaclass/ethrex/issues/4327) | Don't generate proofs for empty L2 blocks |
| Proof Coordinator Health | Pending | [#4509](https://github.com/lambdaclass/ethrex/issues/4509) | `/health` endpoint for proof_coordinator |
| RISC0 Input Serialization | Pending | [#4473](https://github.com/lambdaclass/ethrex/issues/4473) | Try `env::read_slice` for better performance |
| SP1 Build Script | Pending | [#3768](https://github.com/lambdaclass/ethrex/issues/3768) | Fix SP1 build script re-running every time |

---

## 7. Sequencer & Block Production

Core sequencer improvements.

| Item | Status | Issue/PR | Description |
|------|--------|----------|-------------|
| L1 Blob Fee Race Condition | Pending | Related to [#5059](https://github.com/lambdaclass/ethrex/issues/5059) | Fix race between L1Watcher fee update and block producer |
| Review L1 Fee Logic | Pending | [#5059](https://github.com/lambdaclass/ethrex/issues/5059) | Comprehensive review of L1 fee calculation |
| GenServer Review | Pending | [#4321](https://github.com/lambdaclass/ethrex/issues/4321) | Check GenServer patterns in L2 modules |
| Privileged Tx Ordering | Pending | [#4398](https://github.com/lambdaclass/ethrex/issues/4398) | Review privileged transaction ordering |
| Expired Privileged Tx | Pending | [#5325](https://github.com/lambdaclass/ethrex/issues/5325) | Stop new transactions when privileged txs expire |
| Empty Block Error Logging | Pending | [#5719](https://github.com/lambdaclass/ethrex/issues/5719) | Better error logs for empty L2 blocks |
| Batch Seal Error Handling | Pending | [#5154](https://github.com/lambdaclass/ethrex/issues/5154) | Handle errors between batch seal and witness generation |
| Block Timestamp Limits | Pending | [#3742](https://github.com/lambdaclass/ethrex/issues/3742) | New block timestamp must be strictly greater than previous |
| L1 Proof Verifier Health | Pending | [#4510](https://github.com/lambdaclass/ethrex/issues/4510) | `/health` endpoint for l1_proof_verifier |

---

## 8. Testing

Test coverage and infrastructure.

| Item | Status | Issue/PR | Description |
|------|--------|----------|-------------|
| L2 Hook Unit Tests | In Progress | [#5057](https://github.com/lambdaclass/ethrex/issues/5057), [#6051](https://github.com/lambdaclass/ethrex/pull/6051) | 145 unit tests for L2Hook implementation |
| Forced Inclusion Test | Pending | [#5669](https://github.com/lambdaclass/ethrex/issues/5669) | Integration test for forced inclusion in cross-L2 ERC20 |
| Refactor Integration Tests | Pending | [#4290](https://github.com/lambdaclass/ethrex/issues/4290), [#3862](https://github.com/lambdaclass/ethrex/issues/3862) | Improve test organization |
| Integration Test Docs | Pending | [#3858](https://github.com/lambdaclass/ethrex/issues/3858) | Add `///` docs to each integration test |
| Load Test | Pending | [#4243](https://github.com/lambdaclass/ethrex/issues/4243) | Add load testing to integration tests |
| Stop Docker in CI | Pending | [#5238](https://github.com/lambdaclass/ethrex/issues/5238) | Remove Docker dependency from L2 CI tests |

---

## 9. Shared Bridge

Cross-L2 communication and asset transfers.

| Item | Status | Issue/PR | Description |
|------|--------|----------|-------------|
| L2↔L2 Messages in Prover | Pending | [#5017](https://github.com/lambdaclass/ethrex/issues/5017) | Add cross-L2 message support to prover (assigned: gianbelinche) |
| ERC20 Transfers | Pending | [#5018](https://github.com/lambdaclass/ethrex/issues/5018) | Cross-L2 ERC20 transfer support |
| Refactor Model | Pending | [#5019](https://github.com/lambdaclass/ethrex/issues/5019) | Rework current shared bridge model |
| Fix Sender Address | Pending | [#5020](https://github.com/lambdaclass/ethrex/issues/5020) | Correct sender address in destination L2 |
| Error Handling Review | Pending | [#5053](https://github.com/lambdaclass/ethrex/issues/5053) | Review error handling in the flow |

**Tracker:** [Shared Bridge Tracking](https://github.com/lambdaclass/ethrex/issues/5016)

---

## 10. Monitor & Developer UX

TUI monitor and local development experience.

| Item | Status | Issue/PR | Description |
|------|--------|----------|-------------|
| Anvil-like L1 UX | Pending | — | Local dev L1 with anvil-style output and UX |
| Reduce RPC Call Volume | Pending | [#3824](https://github.com/lambdaclass/ethrex/issues/3824) | Optimize monitor RPC calls |
| Refactor Display/Fetch | Pending | [#3839](https://github.com/lambdaclass/ethrex/issues/3839) | Separate display and fetch logic |
| Show Custom Rich Accounts | Pending | [#4200](https://github.com/lambdaclass/ethrex/issues/4200) | Display custom rich accounts in monitor |
| Copy Rich Account Data | Pending | [#4198](https://github.com/lambdaclass/ethrex/issues/4198) | Enable copying from rich accounts tab |
| Add Flex to Components | Pending | [#3695](https://github.com/lambdaclass/ethrex/issues/3695) | Responsive layout for monitor |
| WebSocket RPC | Pending | [#4898](https://github.com/lambdaclass/ethrex/issues/4898) | Implement WebSocket RPC support |

---

## 11. Documentation

Documentation improvements and gaps.

| Item | Status | Issue/PR | Description |
|------|--------|----------|-------------|
| Integration Tests Docs | Pending | [#5270](https://github.com/lambdaclass/ethrex/issues/5270) | Address feedback on integration test docs |
| contracts.md Comments | Pending | [#4695](https://github.com/lambdaclass/ethrex/issues/4695) | Address review comments |
| Uniswap V3 Deployment | Pending | [#4555](https://github.com/lambdaclass/ethrex/issues/4555) | Deployment documentation (assigned: LeanSerra) |
| L2 Upgrade Guide | Pending | [#4618](https://github.com/lambdaclass/ethrex/issues/4618) | How to upgrade an L2 chain |
| Execution Witness Docs | Pending | [#4358](https://github.com/lambdaclass/ethrex/issues/4358) | Update execution witness documentation |
| WETH9 Deployment Script | Pending | [#4557](https://github.com/lambdaclass/ethrex/issues/4557) | Canonical WETH9 deployment docs |

---

## 12. Contracts & Standards

Smart contract improvements and standard compliance.

| Item | Status | Issue/PR | Description |
|------|--------|----------|-------------|
| RIP-7740 Tracking | Pending | [#4552](https://github.com/lambdaclass/ethrex/issues/4552) | CreateX and factory deployment (assigned: LeanSerra) |
| Canonical WETH9 | Pending | [#4553](https://github.com/lambdaclass/ethrex/issues/4553), [#4556](https://github.com/lambdaclass/ethrex/issues/4556) | Deploy canonical WETH9 contract |
| RIP-7875 Bridge Address | Pending | [#4527](https://github.com/lambdaclass/ethrex/issues/4527) | Change default bridge to `0x1ff` |
| Deploy create2 Factories | Pending | [#4526](https://github.com/lambdaclass/ethrex/issues/4526) | All factories from RIP-7740 |
| WETH symbol()/name() | Pending | [#5398](https://github.com/lambdaclass/ethrex/issues/5398) | Add missing ERC methods to WETH |
| Uniswap V3 Deployment | Pending | [#4551](https://github.com/lambdaclass/ethrex/issues/4551) | Full Uniswap V3 deployment |
| Uniswap V3 UI | Pending | [#4554](https://github.com/lambdaclass/ethrex/issues/4554) | Uniswap V3 interface |

---

## 13. Tech Debt & Cleanup

Code quality and maintenance items.

| Item | Status | Issue/PR | Description |
|------|--------|----------|-------------|
| Remove CARGO_MANIFEST_DIR | Pending | [#5003](https://github.com/lambdaclass/ethrex/issues/5003) | Remove hardcoded manifest paths |
| Move Batch Type | Pending | [#3755](https://github.com/lambdaclass/ethrex/issues/3755) | Move `Batch` to `crates/l2/common/` |
| Remove L2 Logic from eth_client | Pending | [#3754](https://github.com/lambdaclass/ethrex/issues/3754) | Clean separation of concerns |
| Remove L2 Broadcast | Pending | [#4318](https://github.com/lambdaclass/ethrex/issues/4318) | Remove deprecated broadcast code |
| Constants File | Pending | [#5607](https://github.com/lambdaclass/ethrex/issues/5607) | Collect sparse constants in one place |
| Remove JoinSet | Pending | [#3721](https://github.com/lambdaclass/ethrex/issues/3721) | Replace join set with spawned tasks |
| Add eth_client Docs | Pending | [#3820](https://github.com/lambdaclass/ethrex/issues/3820) | Document eth_client crate |
| Improve Docker Composes | Pending | [#5512](https://github.com/lambdaclass/ethrex/issues/5512) | Better docker-compose files |

**Tracker:** [Code Simplification Tracker](https://github.com/lambdaclass/ethrex/issues/3744)

---

## Stage Compliance

Current L2Beat stage status and path forward.

| Stage | Status | Blocker |
|-------|--------|---------|
| **Stage 0** | Met | — |
| **Stage 1** | Not Met | Forced inclusion mechanism for withdrawals |
| **Stage 2** | Not Met | Permissionless proving, 30-day exit window |

**Documentation:** [Rollup Stages](./stages.md)

---

## References

- [Main Project Roadmap](/ROADMAP.md)
- [Based Sequencing Design](./fundamentals/based.md)
- [Stage Compliance Analysis](./stages.md)
- [zkVM Comparison Benchmarks](./bench/zkvm_comparison.md)
