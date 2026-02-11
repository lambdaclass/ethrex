# ethrex dev node — Roadmap

> The path to becoming the default development environment for Ethereum.

## Vision

ethrex dev is not a simulator. It's a **real Ethereum execution client** running in development mode — same EVM, same storage engine, same RPC stack that runs in production. This is the foundation everything else builds on: when your contracts work on ethrex dev, they work on Ethereum. No translation layer, no behavioral gaps, no surprises at deployment.

The roadmap doubles down on this identity while adding the developer tooling that turns a correct execution client into an indispensable development environment.

---

## Phase 1 — Developer Primitives

**Goal:** Every workflow a contract developer needs, built on a real execution client.

| Feature | Description |
|---------|-------------|
| **Time manipulation** | `evm_increaseTime`, `evm_setNextBlockTimestamp`, `evm_mine` — control block timestamps for testing time-dependent logic (vesting, lockups, governance) |
| **State snapshots** | `evm_snapshot` / `evm_revert` — checkpoint and rollback state during test runs without restarting the node |
| **Account impersonation** | Send transactions as any address without its private key — essential for testing governance, multisig, and access-controlled contracts |
| **State overrides** | Override balances, nonces, code, and storage in `eth_call` and `eth_estimateGas` without modifying actual state |
| **Auto-mine controls** | Toggle between instant mining, interval mining, and manual mining (`evm_setAutomine`, `evm_setIntervalMining`) |
| **Configurable base fee** | Set and manipulate base fee for gas price testing |

**Why ethrex is different here:** These aren't bolted onto a mock — they manipulate a real client's state. The EVM behavior is identical to what validators run.

---

## Phase 2 — Live Chain Integration

**Goal:** Develop against real mainnet/testnet state without leaving ethrex.

| Feature | Description |
|---------|-------------|
| **Fork mode** | `--fork-url <RPC>` — fork any EVM chain at its latest block. Run your contracts against real mainnet state locally. |
| **Fork pinning** | `--fork-block-number <N>` — pin to a specific block for reproducible tests |
| **Lazy state loading** | Fetch storage slots on-demand from the fork source, cache locally. Minimal startup time even for mainnet forks. |
| **State diff view** | Expose what your transactions changed relative to the forked state — useful for auditing and understanding side effects |

**Why ethrex is different here:** Fork mode in a full execution client means the forked environment has the same consensus rules, the same precompile behavior, the same edge cases as the real chain. The execution path is identical.

---

## Phase 3 — Debugging & Observability

**Goal:** When something goes wrong, ethrex shows you exactly what happened and why.

| Feature | Description |
|---------|-------------|
| **Enhanced transaction traces** | Structured call trees with gas breakdown per frame, human-readable revert reasons, decoded event logs |
| **`console.log` support** | Intercept Solidity `console.log` calls and display them in the node output — zero-friction debugging |
| **Gas profiler** | Per-opcode gas breakdown for any transaction. Identify exactly which operations are expensive. |
| **Storage access heatmap** | Visualize which storage slots a transaction touches — find redundant SLOADs, optimize storage layout |
| **Built-in block explorer** | Lightweight web UI at `http://localhost:8545/explorer` — browse blocks, transactions, accounts, and logs without external tools |
| **Transaction replay** | Re-execute any historical transaction with modified parameters to test "what if" scenarios |

---

## Phase 4 — L2-Native Development

**Goal:** The only dev environment that treats L1 and L2 as a single system.

This is ethrex's structural advantage. ethrex runs both the L1 execution client and the L2 rollup stack. No other dev environment can offer integrated cross-layer development because no other dev environment _is_ both layers.

| Feature | Description |
|---------|-------------|
| **Unified L1+L2 mode** | Already ships today: `ethrex l2 --dev` starts both layers with auto-deployed bridge contracts |
| **Cross-layer message testing** | Send L1 deposits and L2 withdrawals in a single test. Verify bridge behavior end-to-end without manual setup. |
| **L2 fee simulation** | Accurate L1 data cost + L2 execution cost estimation. Test fee-sensitive logic with realistic numbers. |
| **Deposit/withdrawal acceleration** | In dev mode, process L1-to-L2 deposits and L2-to-L1 withdrawals instantly instead of waiting for challenge periods |
| **Proving dry-run** | Validate that a batch of L2 transactions can be proven without submitting an actual proof — catch proving failures during development |
| **Rollup state inspection** | Query L2 batch status, sequencer state, and bridge balances through dedicated RPC methods |

---

## Phase 5 — Testing Infrastructure

**Goal:** Ship with confidence. ethrex dev becomes the testing backend, not just the development backend.

| Feature | Description |
|---------|-------------|
| **Gas snapshot regression** | Record gas usage per test case. Fail CI when gas usage regresses beyond a threshold. Catch inefficiencies before they ship. |
| **Coverage-guided fuzzing** | Built-in transaction fuzzer that explores contract state space. Finds edge cases your unit tests miss. |
| **Invariant testing** | Define properties that must always hold (e.g., "total supply equals sum of balances"). ethrex generates random transaction sequences to try to break them. |
| **Multi-block test scenarios** | DSL or API for scripting multi-block test sequences: deploy, interact, advance time, fork, assert — all in one test file |
| **Deterministic execution** | Same seed produces identical block sequences and state transitions. Fully reproducible test runs across machines. |

---

## Phase 6 — Ecosystem Integration

**Goal:** ethrex dev works with every tool developers already use.

| Feature | Description |
|---------|-------------|
| **Foundry-compatible RPC** | Full compatibility with `forge test --fork-url`, `cast`, and `chisel` — drop-in replacement |
| **Hardhat network compatibility** | Support `hardhat_*` RPC methods so existing Hardhat projects work without configuration changes |
| **VS Code extension** | Start/stop dev node, view accounts, browse transactions, inspect storage — all from the editor |
| **GitHub Action** | `lambdaclass/ethrex-dev-action@v1` — one-line CI integration for running contract tests against ethrex |
| **SDK bindings** | TypeScript, Python, and Go libraries for programmatic node control (snapshot, time-travel, impersonate) beyond raw RPC |

---

## Principles

1. **Correctness is the feature.** A real execution client in dev mode means zero behavioral drift from production. This is the foundation of everything.
2. **L2 is first-class.** Cross-layer development isn't a plugin or afterthought — it's built into the same binary.
3. **Speed through simplicity.** The dev node is ~330 lines of block-building logic. Keep it lean. Fast startup, low memory, instant blocks.
4. **No walls.** Full Engine API, full debug namespace, full admin namespace. Developers should never hit "not supported in dev mode."
5. **Works with your tools.** Compatibility with existing frameworks is non-negotiable. Migration cost must be zero.
