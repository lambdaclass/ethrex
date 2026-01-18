# Session: 2026-01-17

## Baseline
- Metric: **NOT ESTABLISHED** - benchmark broken
- Commit: 48dbf0b81
- Machine: ivanlitteri-macbook (Apple M3 Max, 36GB)

## Experiments Tested

| # | Name | Result | Status | Notes |
|---|------|--------|--------|-------|
| 001 | Benchmark investigation | N/A | BLOCKED | build_block_benchmark fails with NotEnoughBalance |

## Blockers
1. `build_block_benchmark` panics due to `max_fee_per_gas: u64::MAX` overflow
   - Location: `benches/benches/build_block_benchmark.rs:167`
   - Fix needed before any performance testing can proceed

## Key Learnings
1. The benchmark uses unrealistic `max_fee_per_gas` values that cause balance overflow
2. Need to fix benchmark before establishing baseline

## Ideas Backlog
- [ ] Fix build_block_benchmark (BLOCKING)
- [ ] Establish baseline metrics
- [ ] FxHashSet for access lists
- [ ] Inline hot opcodes
