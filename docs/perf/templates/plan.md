# Experiment Plan: [Name]

**Date:** YYYY-MM-DD
**Author:** [Name]
**Status:** Draft / Approved / In Progress / Complete

---

## Objective

[Clear statement of what we're optimizing and success criteria]

Example:
> Reduce block execution time by optimizing memory allocation patterns in LEVM.
> Target: >5% improvement on typical workloads without regressions.

---

## Baseline Metrics

*Fill in after establishing baseline:*

| Metric | Value | StdDev |
|--------|-------|--------|
| Block time (ms) | | |
| Throughput (Mgas/s) | | |
| Memory peak (MB) | | |

Commit: `[commit hash]`

---

## Hypotheses

*Ranked list of optimization ideas with expected impact*

### 1. [HIGH] [Hypothesis Name]

- **Expected impact:** [e.g., 10-20% reduction in metric]
- **Risk:** [What could go wrong]
- **Conflicts with:** [Other hypotheses that can't coexist]
- **Code location:** [File:line]
- **Evidence:** [Profile data, intuition, or prior experiments supporting this]

### 2. [MEDIUM] [Hypothesis Name]

- **Expected impact:**
- **Risk:**
- **Conflicts with:**
- **Code location:**
- **Evidence:**

### 3. [LOW] [Hypothesis Name]

- **Expected impact:**
- **Risk:**
- **Conflicts with:**
- **Code location:**
- **Evidence:**

---

## Attack Plan

*Ordered sequence of optimizations to test*

1. **[Hypothesis 1]** - Highest expected impact, test first
2. **[Hypothesis 2]** - If #1 succeeds, test compound effect
3. **[Hypothesis 3]** - Independent, can test in parallel with #2

---

## Success Criteria

| Criterion | Threshold |
|-----------|-----------|
| Minimum improvement | >5% (or >2% for already-optimized code) |
| Statistical significance | p < 0.05 (10 runs minimum) |
| Regressions | None >2% on any input |
| Correctness | All tests pass, outputs match baseline |

---

## Input Selection

*Document inputs to test (see inputs.yaml for details)*

- [ ] Light workload
- [ ] Storage-heavy workload
- [ ] Precompile-heavy workload (if applicable)
- [ ] Typical workload
- [ ] Edge cases from prior experiments

---

## Risks and Mitigations

| Risk | Likelihood | Mitigation |
|------|------------|------------|
| [Risk 1] | [High/Med/Low] | [How to address] |
| [Risk 2] | | |

---

## Timeline

*Optional: rough timeline for the benchmarking session*

- [ ] Knowledge gathering: [date]
- [ ] Plan approval: [date]
- [ ] Baseline establishment: [date]
- [ ] Experiments: [date range]
- [ ] Final report: [date]

---

## Approval

- [ ] Plan reviewed and approved by: [Name] on [Date]

---

## Notes

*Additional context, links to related issues/PRs, etc.*
