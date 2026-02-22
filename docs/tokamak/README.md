# Tokamak Ethereum Client — Proven Execution

> **"Performance you can see, verify, and debug."**
>
> The Ethereum execution client that's fastest,
> proves it automatically, and shows you exactly why.

## Base: ethrex Fork (Rust)

ethrex(LambdaClass, Apache 2.0) fork. L2 native integration via `--tokamak-l2` flag.

## Tier S Features: Self-Reinforcing Loop

```
   JIT-Compiled EVM (be the fastest)
           |
           v
   Continuous Benchmarking (prove it every commit)
           |
           v
   Time-Travel Debugger (show exactly why)
           |
           +---> feeds back into JIT optimization
```

| # | Feature | Score | Doc |
|---|---------|-------|-----|
| 1 | [Time-Travel Debugger](./features/01-time-travel-debugger.md) | 7.5 | Interactive opcode-level tx replay |
| 2 | [Continuous Benchmarking](./features/02-continuous-benchmarking.md) | 7.5 | Auto benchmark + differential testing |
| 3 | [JIT-Compiled EVM](./features/03-jit-compiled-evm.md) | 7.0 | Cranelift-based JIT, target 3-5x Geth |

## Competitive Positioning

| Capability | Geth | Reth | Nethermind | **Tokamak** |
|-----------|:----:|:----:|:---------:|:-----------:|
| EVM Performance | Baseline | 1.5-2x | ~1x | **3-5x (JIT)** |
| Auto Benchmark | No | No | No | **Every commit** |
| Differential Testing | No | No | No | **Built-in** |
| Time-Travel Debug | Raw trace | Raw trace | Raw trace | **Interactive** |
| Proves its own speed | No | No | No | **Yes** |

## Documents

### Vision
- [Combined Vision](./vision.md) — 3-feature loop + architecture + roadmap
- [Slack Post](./slack-post.md) — Full announcement draft
- [Slack Short](./slack-short.md) — Condensed version

### Context
- [Team Discussion Summary](./context/team-discussion-summary.md) — 8-person team discussion
- [Competitive Landscape](./context/competitive-landscape.md) — Market analysis + Build/Fork matrix
- [Open Questions](./context/open-questions.md) — Decision status + dual-track strategy
- [Volkov Reviews](./context/volkov-reviews.md) — 5 rounds of review (3.0 -> 5.25 -> 4.5 -> 4.0)

### Scaffold Reference
- [CLAUDE.md](./scaffold/CLAUDE.md) — Standalone monorepo setup (7 crates)
- [HANDOFF.md](./scaffold/HANDOFF.md) — Scaffold status + next steps

## Implementation Roadmap

| Phase | Duration | Deliverable |
|-------|----------|-------------|
| 1. Foundation | Month 1-2 | Mainnet sync + auto benchmark dashboard |
| 2. Debugging | Month 3-4 | Time-Travel Debugger (interactive tx replay) |
| 3. Performance | Month 5-7 | JIT EVM, Geth 2-3x+ performance |
| 4. L2 Integration | Month 8-10 | `--tokamak-l2` flag |

## Immediate Next Steps

| Priority | Action | Owner | Week |
|----------|--------|-------|------|
| 1 | ethrex fork vs contribute decision | Tech leads | W1 |
| 2 | Track A team assignment (Senior Rust 2) | Kevin | W1 |
| 3 | Continuous Benchmarking infra | 1 engineer | W2 |
| 4 | ethrex fork + first mainnet sync attempt | Rust team | W3 |
| 5 | Track B Time-Travel Debugger MVP | Python team | W3 |
