# Tokamak Ethereum Client: Combined Vision

> **"Performance you can see, verify, and debug."**
> Designed to be the fastest Ethereum execution client — that proves it automatically and shows you exactly why.

## Three Features, One Loop

We're not building three separate features. We're building a **self-reinforcing feedback loop** that no other Ethereum client has:

```
   JIT-Compiled EVM (be the fastest)
           │
           ▼
   Continuous Benchmarking (prove it every commit)
           │
           ▼
   Time-Travel Debugger (show exactly why)
           │
           └──→ feeds back into JIT optimization
```

---

## Feature 1: JIT-Compiled EVM

Every existing Ethereum client interprets EVM bytecode one opcode at a time. We compile hot contracts (Uniswap Router, ERC-20s, Aave) into native machine code at runtime.

**How it works:**
- Tier 0: First call → interpreter (same as Geth/Reth, zero overhead)
- Tier 1: After 10+ calls → Cranelift baseline JIT compilation
- Tier 2: After 100+ calls → optimizing JIT with opcode fusion

**Target:** 3-5x faster than Geth on compute-heavy contracts (e.g. complex DeFi, loops). I/O-bound transactions (dominated by SLOAD/SSTORE) will see smaller gains. Overall improvement varies by workload.

**Why no one has done this:** JIT must produce results bit-identical to the interpreter — any deviation breaks consensus. This requires rigorous validation, fuzzing, and the entire Ethereum test suite passing at 100%.

---

## Feature 2: Continuous Benchmarking + Differential Testing

Every commit automatically runs the same transactions through our client, Geth, and Reth — then publishes the results to a public dashboard.

**What it does:**
- Measures sync speed, tx execution time, memory usage against Geth/Reth
- Detects performance regressions (>5% slowdown blocks the PR)
- **Differential testing:** compares state roots across clients — if they diverge, we've found a potential consensus bug

**Why this matters for trust:**
The fastest path into the Ethereum community isn't running nodes quietly for years. It's finding one bug in Geth through differential testing, disclosing it responsibly, and earning an invite to All Core Devs. This system automates that discovery process.

**Public dashboard:** Every claim about performance is verifiable at any time.

---

## Feature 3: Time-Travel Debugger

When differential testing finds a divergence, or when a developer needs to understand why a transaction reverted, they can replay any historical transaction interactively — stepping forward and backward through every opcode with full state inspection.

**What it does:**
- Replay any past transaction with the exact state at that block
- Step forward/backward through opcodes (like a code debugger, but for EVM)
- Inspect stack, memory, and storage at every step
- Set breakpoints on specific opcodes (SSTORE, CALL, DELEGATECALL)
- "What-if" mode: modify tx parameters and re-execute

**What exists today:**
- Tenderly does this as a paid SaaS — not local, not free
- Geth's `debug_traceTransaction` gives raw traces — not interactive
- Foundry's debugger is limited to local test environments

**Ours:** Built into the node. Local. Free. Interactive. Works on real mainnet history.

---

## How They Work Together

**Scenario: JIT Optimization Loop**
1. Benchmarking detects: "Aave liquidation is slower than Reth"
2. Time-Travel replays the tx → finds bottleneck at JUMPDEST pattern
3. JIT team adds optimization for that pattern
4. Next commit: Benchmarking auto-confirms improvement (2.8x → 3.4x)

**Scenario: Finding a Geth Bug**
1. Benchmarking detects: "State root mismatch at block #19,847,231"
2. Time-Travel replays the divergent tx opcode-by-opcode
3. Root cause: Geth miscalculates gas for edge-case SSTORE
4. Responsible disclosure → Geth team → positions us for ACD invitation
5. Tokamak builds reputation as a team that actively secures Ethereum

**Scenario: Convincing a Node Operator**
1. Operator: "Why should I switch from Geth?"
2. Us: "Visit clients.tokamak.network. Auto-updated benchmarks on every commit. Real numbers, not marketing. Verify it yourself."

---

## Competitive Landscape

| | Geth | Reth | Nethermind | **Tokamak** |
|---|:---:|:---:|:---:|:---:|
| EVM speed | Baseline | 1.5-2x | ~1x | **Target: 3-5x (JIT)** |
| Auto benchmark | No | No | No | **Every commit** |
| Public proof | No | No | No | **Dashboard** |
| Differential testing | No | No | No | **Built-in** |
| Interactive debugger | Raw trace | Raw trace | Raw trace | **Time-Travel** |
| Self-improving | No | No | No | **Yes (loop)** |

**No existing Ethereum client combines these three.**

---

## Implementation: Rust + Three Modules

Built in Rust, with ethrex (LambdaClass, Apache 2.0) as a potential starting point — whether we fork it, contribute upstream, or build independently is still under discussion. Regardless of the base, the architecture adds three modules:

- `jit/` — Cranelift-based JIT compiler replacing the interpreter for hot paths
- `benchmark/` — automated comparison + differential testing against Geth/Reth
- `debugger/` — transaction replay with interactive state inspection

The three-feature loop is the differentiator, not the base client.

Later: `--tokamak-l2` flag for native L2 integration (same binary, one flag).

---

**One-liner:**
Other clients claim they're fast. We prove it on every commit and show you exactly why.
