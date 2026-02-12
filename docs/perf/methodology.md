# Benchmarking & Optimization Methodology

A standardized workflow for running benchmarks, testing optimizations, and tracking results. Designed for reproducibility across sessions and team members.

> **Version:** 1.1.0
> **Last Updated:** 2026-01-16

---

## Claude Instructions

**This section is for Claude (the AI assistant) to follow when commanded to do benchmarking work.**

### When to Use This Workflow

Use this workflow when the user asks to:
- "Run benchmarks on [feature]"
- "Optimize [component]"
- "Profile and improve performance"
- "Test this optimization"
- "Start a benchmarking session"

### Invocation Checklist

When starting a benchmarking session:

1. **Read this file first** - Follow the phases in order
2. **Read [architecture.md](architecture.md)** - Understand the codebase structure
3. **Read [ideas.md](ideas.md)** - Check existing ideas and their status
4. **Check for existing TRACKER.md** in `docs/perf/experiments/` - Resume if exists
5. **Ask user for sources** (Phase 1.1) before planning

### Tool Usage

| Task | Tool to Use |
|------|-------------|
| Run benchmarks | `Bash` (with `run_in_background: true` for long runs) |
| Read benchmark output | `Read` or `TaskOutput` for background tasks |
| Create experiment directories | `Bash` with `mkdir -p` |
| Write tracking files | `Write` for TRACKER.md, YAML files |
| Update ideas.md | `Edit` to modify specific rows |
| Profile code | `Bash` with samply/perf commands |
| Check system state | `Bash` with `htop`, `free -h`, `nvidia-smi` |

### File Locations

All benchmarking artifacts go in `docs/perf/`:
- `docs/perf/experiments/NNN-name/` - Per-experiment data
- `docs/perf/experiments/TRACKER.md` - Current session tracking
- `docs/perf/HISTORY.yaml` - Cross-session history
- `docs/perf/ideas.md` - Update after each experiment

### Session Continuity

**At session start:**
1. Check if `docs/perf/experiments/TRACKER.md` exists
2. If yes, read it and ask user: "Found existing session from [date]. Resume or start fresh?"
3. If resuming, continue from where TRACKER.md left off

**At session end or when interrupted:**
1. Update TRACKER.md with current state
2. Update ideas.md with any results
3. Tell user: "Session state saved to TRACKER.md. Resume anytime."

### Long-Running Benchmarks

For benchmarks that take >2 minutes:
1. Use `Bash` with `run_in_background: true`
2. Tell user the benchmark is running
3. Use `TaskOutput` to check progress periodically
4. Parse results when complete

Example:
```
# Start benchmark in background
Bash(command="hyperfine --runs 10 '...' --export-json results.json", run_in_background=true)

# Check on it later
TaskOutput(task_id="...", block=false)
```

### Reporting to User

After each experiment, report:
```
Experiment NNN: [Name]
Result: [time] (baseline: [baseline_time])
Change: [percentage]%
Verdict: KEEP/DISCARD/MAYBE
Reason: [brief explanation]
```

### Context Updates

Update the project context file (`~/Personal/contexts/lambdaclass-ethrex_4.md`) with:
- Current benchmarking focus
- Key findings
- Next steps

---

## Overview

This workflow covers:
1. Knowledge gathering and planning
2. Baseline establishment
3. Optimization exploration and testing
4. Results tracking and observability
5. Iteration and compound testing

For tracking formats, see [tracking.md](tracking.md).
For dashboard/reporting, see [observability.md](observability.md).
For zkVM-specific guidance, see [zkvm-guidelines.md](zkvm-guidelines.md).

---

## Critical Rules

These rules must NEVER be violated:

1. **Only run parallel benchmarks when they won't affect results** - GPU/CPU contention invalidates results; parallel is OK for independent machines or non-overlapping resources
2. **Never skip baseline** - All comparisons require a valid baseline
3. **Never modify code during a benchmark run** - Invalidates the run
4. **Always record before deciding** - Log results before making keep/discard decisions
5. **Always test correctness after optimization** - Faster but wrong is useless
6. **Always use multiple inputs** - Single-input optimizations may not generalize

---

## Phase 1: Knowledge Gathering

### 1.1 Request Sources

**Before starting any benchmarking work, request sources from the user:**

```
I need sources to build context before planning. Please provide:
1. Relevant documentation (internal docs, RFCs, design docs)
2. Existing benchmarks or profiling data
3. Known bottlenecks or areas of concern
4. Related academic papers or blog posts (if applicable)
5. Similar optimization efforts (past PRs, issues)
6. Constraints (time budget, acceptable regressions, etc.)
```

### 1.2 Codebase Exploration

After receiving sources:
- Read all provided materials thoroughly
- Explore the codebase for existing benchmarks (use as templates)
- Identify the critical path and hot functions
- Read [architecture.md](architecture.md) for code context
- Document initial hypotheses

### 1.3 Environment Documentation

Record the benchmark environment using the [environment template](templates/environment.yaml):

```yaml
# docs/perf/experiments/NNN-experiment-name/environment.yaml
machine:
  hostname: ethrex-office-2
  cpu: AMD Ryzen 9 5950X
  gpu: NVIDIA RTX 3090  # if applicable
  memory: 64GB
  os: Debian GNU/Linux 13

toolchain:
  rust: 1.84.0
  # Add other relevant versions

commit: abc123def  # exact commit being benchmarked
date: 2026-01-16
```

---

## Phase 2: Planning

### 2.1 Create Comprehensive Plan

After gathering knowledge, create a detailed plan using the [plan template](templates/plan.md):

```markdown
# Experiment Plan: [Name]

## Objective
[Clear statement of what we're optimizing and success criteria]

## Baseline Metrics
[Current performance numbers, to be filled after baseline runs]

## Hypotheses
[Ranked list of optimization ideas with expected impact]

1. **[HIGH] MPT Hash Caching**
   - Expected impact: 20-40% reduction in hash computation
   - Risk: Memory increase
   - Conflicts with: None

2. **[MEDIUM] Zero-copy deserialization**
   - Expected impact: 5-10% reduction in deserialization
   - Risk: API changes required
   - Conflicts with: #3

## Attack Plan
[Ordered sequence of optimizations to test]

## Success Criteria
- Minimum improvement threshold: 5%
- Statistical significance: p < 0.05
- No regressions in other metrics
```

### 2.2 Plan Review

Present the plan to the user for approval before proceeding.

---

## Phase 3: Baseline Establishment

### 3.1 Pre-Benchmark Checks

Before running benchmarks:

```bash
# Check no other heavy processes
htop  # or ps aux

# Record system state
uptime
free -h

# For GPU workloads (zkVM proving):
# Check GPU temperature (should be <50C before starting)
nvidia-smi --query-gpu=temperature.gpu --format=csv,noheader
```

### 3.2 Input Diversity

**Never benchmark with a single input.** Select diverse inputs using the [inputs template](templates/inputs.yaml):

```yaml
# docs/perf/experiments/NNN-experiment-name/inputs.yaml
inputs:
  - name: "light_block"
    path: "./inputs/block_1000_light.bin"
    description: "Few transactions, no precompiles"

  - name: "heavy_storage"
    path: "./inputs/block_2000_storage_heavy.bin"
    description: "Many SLOAD/SSTORE operations"

  - name: "precompile_heavy"
    path: "./inputs/block_3000_ecrecover.bin"
    description: "Many signature verifications"

  - name: "typical"
    path: "./inputs/block_4000_typical.bin"
    description: "Representative average block"
```

For random input generation:
- Use deterministic seeds for reproducibility
- Document the seed in results
- Generate at least 5 diverse inputs

### 3.3 Baseline Runs

Run baseline **minimum 10 times per input**:

```bash
# Using hyperfine for statistical rigor
for input in inputs/*.bin; do
  hyperfine \
    --warmup 3 \
    --runs 10 \
    --export-json "baseline/$(basename $input .bin).json" \
    "your-benchmark-command --input $input"
done

# Quick synthetic benchmark
cargo run -p ethrex-benches --bin perf_bench --release
```

### 3.4 Multi-Metric Baseline

Track multiple metrics, not just time:

```yaml
# docs/perf/experiments/NNN-experiment-name/baseline.yaml
primary:
  time_ms: 12.68
  time_stddev: 0.5
  throughput_mgas_s: 669

secondary:
  memory_peak_mb: 2048

profile:  # From profiler output
  top_functions:
    - name: "Node::memoize_hashes"
      cost_percent: 59.08
    - name: "LEVM::execute_tx"
      cost_percent: 36.58
```

### 3.5 Baseline Validation

- Check coefficient of variation (CV) < 10%
- If CV > 10%:
  1. Check for thermal throttling
  2. Check for background processes
  3. Increase cooldown between runs
  4. If still unstable, document and proceed with wider error bars
- Record baseline metrics in plan
- **Run correctness test** - Verify output is correct before proceeding

---

## Phase 4: Optimization Testing

### 4.1 Branch Strategy

For each optimization:

```bash
# Create experiment branch from baseline
git checkout -b bench/001-optimization-name

# After testing, if successful:
git checkout -b bench/001-optimization-name-KEEP

# If discarded:
git checkout -b bench/001-optimization-name-DISCARD
```

### 4.2 Experiment Structure

Each experiment gets a directory:

```
docs/perf/experiments/001-mpt-hash-cache/
├── environment.yaml  # Machine/toolchain info
├── inputs.yaml       # Inputs used
├── hypothesis.md     # What we're testing and why
├── changes.patch     # Git diff of changes
├── results.json      # Hyperfine output
├── profile.txt       # Profiler output
└── verdict.md        # Decision and reasoning
```

### 4.3 Running Experiments

```bash
# Cooldown between experiments
sleep 60

# Run with same parameters as baseline
hyperfine \
  --warmup 3 \
  --runs 10 \
  --export-json results.json \
  'your-benchmark-command'
```

### 4.4 Correctness Testing

**Before accepting any optimization, verify correctness:**

```bash
# Run existing tests
cargo test --release

# Run benchmark correctness checks
./run_correctness_tests.sh

# Compare outputs
diff baseline_output.bin optimized_output.bin
```

An optimization that produces incorrect results is **immediately rejected**, regardless of speedup.

### 4.5 Decision Criteria

| Result | Action |
|--------|--------|
| >5% improvement, p<0.05, correct | **KEEP** - merge to accumulator branch |
| 2-5% improvement, correct | **MAYBE** - keep in backlog, may compound |
| <2% improvement | **DISCARD** - overhead not worth it |
| Any regression | **DISCARD** - immediate rejection |
| Timeout (>1.10x baseline) | **KILL** - abort run, mark as failed |
| Incorrect output | **REJECT** - critical failure |

**Context-dependent thresholds:**
- For already-optimized code: accept >2% improvements
- For first-pass optimization: require >5% improvements
- Document threshold used in plan

### 4.6 Edge Cases

**Flaky results (high variance across runs):**
1. Increase run count to 20
2. Remove outliers (>2σ from mean)
3. If still flaky, mark as INCONCLUSIVE
4. Try with different inputs

**Improves some inputs, regresses others:**
1. Calculate weighted average based on input frequency
2. If net positive and no input regresses >5%: KEEP
3. If any input regresses >5%: DISCARD or make input-conditional

**Two optimizations conflict:**
1. Test each independently
2. Test both together
3. If A+B < max(A, B): they conflict
4. Keep only the better one
5. Document conflict in [tracking.md](tracking.md) conflict matrix

**Optimization breaks on edge cases:**
1. Add the edge case to test inputs
2. If fixable: fix and re-test
3. If not fixable: DISCARD

### 4.7 Parallel vs Sequential

- **Parallel exploration**: Multiple people can research and prototype independent optimizations simultaneously
- **Sequential validation**: All benchmark runs must be sequential to avoid resource contention
- **Never run parallel benchmarks on the same machine**: CPU/memory contention invalidates results

---

## Phase 5: Tracking & Memory

See [tracking.md](tracking.md) for detailed tracking formats.

### 5.1 Update Frequency

- Update TRACKER.md **after every experiment**
- Update plan when priorities change
- Commit tracking files with each experiment

### 5.2 Historical Tracking

Maintain HISTORY.yaml across sessions to:
- Detect regressions across sessions
- Build on previous learnings
- Avoid re-testing discarded ideas

---

## Phase 6: Observability & Frontend

See [observability.md](observability.md) for detailed dashboard and notification setup.

Key points:
- Generate static HTML reports for visibility
- Auto-refresh dashboard during active benchmarking
- Notify on significant findings

---

## Phase 7: Notifications

### 7.1 When to Notify

- Baseline complete
- Each experiment complete (with result summary)
- Significant finding (>10% improvement)
- Error or crash
- All experiments complete

### 7.2 Message Format

```
Experiment 001 Complete
---
Name: MPT Hash Caching
Result: 245s (baseline: 262s)
Improvement: -6.5%
Status: KEEP
---
Progress: 3/12 experiments
Dashboard: http://machine:8080/
```

---

## Phase 8: Iteration

### 8.1 After Initial Plan Complete

Once all planned experiments are done:

1. Review results and learnings
2. Identify new hypotheses based on findings
3. Test compound optimizations (combinations of KEEPs)
4. Look for second-order effects

### 8.2 Compound Testing

After individual optimizations:

```markdown
## Compound Experiments

| Combination | Expected | Actual | Verdict |
|-------------|----------|--------|---------|
| 001 + 004 | -15% | -12% | KEEP |
| 001 + 004 + 007 | -20% | -18% | KEEP |
```

### 8.3 Diminishing Returns

Stop iterating when:
- Last 3 experiments all showed <1% improvement
- No more hypotheses in backlog
- Time budget exhausted

### 8.4 Final Report

Generate a summary:

```markdown
# Final Report

## Summary
- Baseline: 12.68ms/block
- Final: 9.63ms/block
- Total improvement: -24.1%

## Optimizations Applied
1. Skip memory zero-init (-6.5%)
2. FxHashSet access lists (-8.2%)
3. Inline hot opcodes (-12.1%)

## Optimizations Discarded
[List with reasons from ideas.md]

## Recommendations for Future Work
[Ideas that showed promise but need more investigation]
```

---

## Checklists

### Before Starting

- [ ] Sources gathered and read
- [ ] Plan created and approved
- [ ] Environment documented
- [ ] Baseline established (10+ runs, CV < 10%)
- [ ] Report server running (optional)
- [ ] Notifications configured (optional)

### During Experiments

- [ ] Cooldown between runs (60s minimum)
- [ ] Results recorded immediately
- [ ] TRACKER.md updated after each experiment
- [ ] ideas.md updated with results

### After Completion

- [ ] Final report generated
- [ ] All branches properly named (KEEP/DISCARD)
- [ ] Learnings documented
- [ ] ideas.md fully updated

---

## Reference Commands

### ethrex Benchmarking

```bash
# Quick synthetic benchmark (ETH transfers, ~100 blocks)
cargo run -p ethrex-benches --bin perf_bench --release

# With hyperfine for statistical rigor
hyperfine \
  --warmup 3 \
  --runs 10 \
  --export-json results.json \
  'cargo run -p ethrex-benches --bin perf_bench --release'

# Build benchmark binary first (faster iteration)
cargo build -p ethrex-benches --bin perf_bench --release
hyperfine --warmup 3 --runs 10 './target/release/perf_bench'
```

### Profiling

```bash
# CPU profiling with samply
cargo build -p ethrex-benches --bin perf_bench --release
samply record --save-only ./target/release/perf_bench

# View profile in browser
samply load perf.json  # opens browser

# Flamegraph (requires cargo-flamegraph)
cargo flamegraph -p ethrex-benches --bin perf_bench

# perf stat for quick overview
perf stat ./target/release/perf_bench
```

### System Checks

```bash
# Check system load before benchmarking
uptime
free -h

# Check for interfering processes
ps aux --sort=-%cpu | head -10

# For GPU workloads
nvidia-smi --query-gpu=temperature.gpu,utilization.gpu,memory.used --format=csv
```

### Creating Experiment Structure

```bash
# Create new experiment directory
EXP_ID="001"
EXP_NAME="skip-zero-init"
mkdir -p docs/perf/experiments/${EXP_ID}-${EXP_NAME}

# Copy templates
cp docs/perf/templates/environment.yaml docs/perf/experiments/${EXP_ID}-${EXP_NAME}/
cp docs/perf/templates/inputs.yaml docs/perf/experiments/${EXP_ID}-${EXP_NAME}/
```

### Git Branch Strategy

```bash
# Create experiment branch
git checkout -b bench/001-skip-zero-init

# After testing - rename based on result
git branch -m bench/001-skip-zero-init bench/001-skip-zero-init-KEEP
# or
git branch -m bench/001-skip-zero-init bench/001-skip-zero-init-DISCARD
```

### Serve Report

```bash
# Simple HTTP server for dashboard
python3 -m http.server 8080 --directory docs/perf/report
```

---

## Emergency Procedures

### Benchmark Machine Becomes Unavailable

1. Check if results were auto-saved (they should be after each run)
2. Resume from last checkpoint when machine returns
3. Re-run the interrupted experiment from scratch
4. Do NOT attempt to continue a partially-completed run

### Results Look Suspicious

1. **Stop immediately** - Don't discard or keep based on suspicious data
2. Check for thermal throttling
3. Check for background processes: `htop`
4. Re-run baseline to verify environment
5. If baseline changed: restart entire session with new baseline

### Optimization Causes Crash

1. Log the crash details (backtrace, error message)
2. Mark experiment as FAILED
3. Create minimal reproduction case
4. File bug if it's a toolchain issue
5. Move to next experiment

### Running Out of Time

1. Prioritize: finish in-progress experiments
2. Skip lowest-priority remaining experiments
3. Document what was skipped and why
4. Generate partial report with available data
5. Add skipped experiments to backlog for next session
