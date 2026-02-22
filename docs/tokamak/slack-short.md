**Tokamak Ethereum Client — "Performance you can see, verify, and debug."**

Built in Rust, with ethrex (LambdaClass, Apache 2.0) as a potential starting point — whether we fork it or build from scratch is still under discussion. The core idea: three modules that form a self-reinforcing loop:

**1. JIT-Compiled EVM** — Compile hot contracts (Uniswap, Aave, ERC-20s) into native machine code at runtime. No existing client does this. Target: 3-5x on compute-heavy workloads.

**2. Continuous Benchmarking + Differential Testing** — Every commit automatically measures performance against Geth/Reth and publishes results to a public dashboard. When state roots diverge between clients, we've found a potential consensus bug → responsible disclosure → trust.

**3. Time-Travel Debugger** — Replay any historical mainnet transaction interactively, stepping through opcodes with full state inspection. Like Tenderly, but built into the node — local, free, and works on real history.

```
JIT (be fastest) → Benchmarking (prove it) → Debugger (show why) → back to JIT
```

| | Geth | Reth | **Tokamak** |
|---|:---:|:---:|:---:|
| EVM speed | Baseline | 1.5-2x | **Target: 3-5x** |
| Auto benchmark | No | No | **Every commit** |
| Differential testing | No | No | **Built-in** |
| Interactive debugger | Raw trace | Raw trace | **Time-Travel** |

No existing client combines these three. Later: `--tokamak-l2` for native L2 integration.

Whether we fork ethrex, contribute upstream, or build independently — the three-feature loop is the differentiator regardless of the base.

Full write-up: (link to slack-post.md)
