# Experiment: [Name]

**Experiment ID:** NNN
**Date:** YYYY-MM-DD
**Author:** [Name]
**Idea:** [Link to ideas.md row]
**Branch:** `bench/NNN-name` or `bench/NNN-name-KEEP` or `bench/NNN-name-DISCARD`
**Status:** In Progress / Completed / Abandoned

---

## Hypothesis

What improvement do we expect and why?

- **Expected impact:** [e.g., 5-10% reduction in block time]
- **Evidence:** [Profile data, code analysis, or prior experiments]
- **Risk:** [What could go wrong]
- **Conflicts with:** [Other experiments/ideas that can't coexist]

---

## Changes Made

Brief description of the implementation:

| File | Change |
|------|--------|
| `crates/vm/levm/src/memory.rs` | [What changed] |
| `crates/vm/levm/src/vm.rs` | [What changed] |

```bash
# Generate patch for reference
git diff main..HEAD > changes.patch
```

---

## Methodology

### Environment

*(Copy from environment.yaml or summarize)*

- **Machine:** [hostname]
- **CPU:** [model]
- **Memory:** [size]
- **Commit:** [hash]

### Input Diversity

*(Copy from inputs.yaml or summarize)*

- [ ] Light workload
- [ ] Storage-heavy workload
- [ ] Precompile-heavy workload (if applicable)
- [ ] Typical workload

### Benchmark Configuration

- **Benchmark:** Mainnet replay / perf_bench / other
- **Blocks:** [range or count]
- **Runs:** [number, minimum 10]
- **Warmup:** [number of warmup runs]
- **Cooldown:** [seconds between runs]

### Baseline

```
[Baseline benchmark output - include raw hyperfine or benchmark output]
```

| Metric | Value | StdDev | CV |
|--------|-------|--------|-----|
| Block time (ms) | | | |
| Throughput (Mgas/s) | | | |

### After Changes

```
[New benchmark output]
```

| Metric | Value | StdDev | CV |
|--------|-------|--------|-----|
| Block time (ms) | | | |
| Throughput (Mgas/s) | | | |

---

## Results

### Summary

| Metric | Baseline | After | Change | Significant? |
|--------|----------|-------|--------|--------------|
| Block time (mean) | | | | |
| Block time (p99) | | | | |
| Throughput (Mgas/s) | | | | |
| Memory peak (MB) | | | | |

### Per-Input Results

| Input | Baseline | After | Change |
|-------|----------|-------|--------|
| light | | | |
| storage_heavy | | | |
| precompile_heavy | | | |
| typical | | | |

**Weighted average:** [if applicable]

---

## Correctness Verification

**CRITICAL: Complete this section before making keep/discard decision.**

- [ ] All existing tests pass (`cargo test --release`)
- [ ] Benchmark correctness checks pass
- [ ] Output matches baseline (diff check)
- [ ] No new warnings or errors

```bash
# Correctness test results
cargo test --release 2>&1 | tail -10
```

---

## Analysis

What do the results tell us? Any unexpected findings?

- **Profiler insights:** [What changed in the profile]
- **Unexpected behavior:** [Anything surprising]
- **Variance notes:** [If CV was high, explain why]

---

## Regressions

Did any benchmarks get worse? Which ones and by how much?

| Input | Regression | Acceptable? |
|-------|------------|-------------|
| [input] | [%] | [Yes/No - explain] |

---

## Decision Criteria Application

Based on [methodology.md decision criteria](../methodology.md#45-decision-criteria):

| Criterion | Result | Threshold | Pass? |
|-----------|--------|-----------|-------|
| Improvement | [%] | >5% | |
| Statistical significance | p=[value] | p<0.05 | |
| Correctness | | Pass | |
| Regressions | | None >2% | |

---

## Verdict

- [ ] **KEEP** - >5% improvement, statistically significant, correct, no regressions
- [ ] **MAYBE** - 2-5% improvement, keep in backlog for compound testing
- [ ] **DISCARD** - <2% improvement or not worth overhead
- [ ] **FAILED** - Crashed, incorrect output, or timeout
- [ ] **INCONCLUSIVE** - Flaky results, needs more investigation

**Reasoning:** [Explain the decision]

---

## Conflict Analysis

Does this optimization conflict with any others?

| Other Experiment | Conflict? | Notes |
|------------------|-----------|-------|
| [experiment] | [Yes/No] | [How they interact] |

---

## Follow-up

Any related ideas or next steps identified during this experiment?

- [ ] [Follow-up idea 1]
- [ ] [Follow-up idea 2]

---

## Checklist

- [ ] Environment documented (environment.yaml)
- [ ] Inputs documented (inputs.yaml)
- [ ] Baseline established (10+ runs)
- [ ] Experiment run (10+ runs)
- [ ] Correctness verified
- [ ] Results recorded
- [ ] Decision made
- [ ] ideas.md updated
- [ ] TRACKER.md updated
- [ ] Branch renamed (KEEP/DISCARD)
