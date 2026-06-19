# Performance Tracking Formats

This document defines the formats for tracking benchmarking progress within a session and across sessions.

---

## TRACKER.md - Per-Session Tracking

Maintain a living document for each benchmarking session:

```markdown
# Session: YYYY-MM-DD

## Baseline
- Metric: 12.68 ms/block, 669 Mgas/s
- Commit: abc123
- Machine: ethrex-office-2

## Experiments Tested

| # | Name | Result | Δ% | Status | Notes |
|---|------|--------|-----|--------|-------|
| 001 | Skip memory zero-init | 11.85ms | -6.5% | KEEP | Merged to main |
| 002 | Zero-copy deserialization | 12.58ms | -0.8% | DISCARD | Below threshold |
| 003 | Batch signatures | crash | N/A | FAILED | Stack overflow |

## Key Learnings
1. Memory zero-init is ~6% of cost - confirmed by profiling
2. Zero-copy already well-optimized, minimal gains possible
3. [Add learnings as you discover them]

## Ideas Backlog
- [ ] Try arena allocator for trie nodes
- [ ] Investigate parallel hash computation
- [x] ~~Zero-copy deserialization~~ (tested, discarded)

## Conflicts Matrix
| Opt A | Opt B | Conflict? | Notes |
|-------|-------|-----------|-------|
| 001 | 004 | Yes | Both modify Memory |
| 002 | 003 | No | |
```

---

## HISTORY.yaml - Cross-Session History

Maintain history across sessions in a structured format:

```yaml
# docs/perf/HISTORY.yaml
sessions:
  - date: "2026-01-16"
    commit: "abc123"
    baseline:
      time_ms: 12.68
      throughput_mgas_s: 669
    final:
      time_ms: 9.63
      throughput_mgas_s: 881
    improvement: "-24.1%"
    optimizations_kept:
      - id: "001"
        name: "Skip memory zero-init"
        improvement: "-6.5%"
      - id: "004"
        name: "FxHashSet access lists"
        improvement: "-8.2%"
    key_learnings:
      - "Memory zero-init dominates at 6%"
      - "Access list operations are in hot path"

  - date: "2026-01-10"
    commit: "def456"
    baseline:
      time_ms: 15.0
      throughput_mgas_s: 566
    final:
      time_ms: 12.68
      throughput_mgas_s: 669
    improvement: "-15.5%"
    optimizations_kept:
      - id: "001"
        name: "Opcode inlining"
        improvement: "-12.5%"
    key_learnings:
      - "Opcode dispatch was major overhead"
```

This enables:
- Detecting regressions across sessions
- Building on previous learnings
- Avoiding re-testing discarded ideas

---

## Conflicts Matrix

Track which optimizations conflict with each other:

```markdown
## Conflicts Matrix

| Opt A | Opt B | Result | Recommendation |
|-------|-------|--------|----------------|
| Skip zero-init | Arena allocator | Conflict (-2% vs +3% individually, combined +1%) | Use Arena only |
| FxHashSet | Parallel warm | No conflict | Use both |
| Opcode inline | Jumptable | No conflict | Use both |
```

**How to detect conflicts:**
1. Test optimization A alone: record result
2. Test optimization B alone: record result
3. Test A+B together: record result
4. If A+B result < max(A, B): they conflict
5. Document which to prefer and why

---

## Ideas Backlog Format

Track ideas for future testing:

```markdown
## Ideas Backlog

### High Priority
- [ ] **Arena allocator for substate** - Expected 10-15% from profiling
- [ ] **SSTORE cache consolidation** - Two lookups, can be one

### Medium Priority
- [ ] **keccak LRU cache** - Top 10k hashes, contract constants
- [ ] **Buffer reuse** - Free-list pattern

### Low Priority
- [ ] **SIMD operations** - Requires significant refactoring
- [ ] **ruint crate** - Needs targeted evaluation

### Tested & Discarded
- [x] ~~Zero-copy deserialization~~ - Only 0.8% gain
- [x] ~~Jumpdest bitmaps~~ - Actually regressed 3%
```

---

## Update Frequency

| Document | When to Update |
|----------|----------------|
| TRACKER.md | After every experiment |
| HISTORY.yaml | End of session |
| Conflicts Matrix | When conflict discovered |
| Ideas Backlog | When ideas added/tested |
| ideas.md | After every experiment |

---

## Integration with ideas.md

The [ideas.md](ideas.md) file is the canonical list of all performance ideas. When tracking:

1. **Before experiment**: Ensure idea exists in ideas.md with status "Benches"
2. **After experiment**: Update ideas.md with:
   - Improvements column: actual percentage gain
   - Regressions column: any negative impacts
   - Notes column: key learnings
   - Status: "Done", "Discarded", or back to "To do" if inconclusive

Example update flow:
```
BEFORE: | Skip memory zero-init | Benches | | | Testing on server |
AFTER:  | Skip memory zero-init | Done | 6.5% avg | None | Merged in PR #1234 |
```

---

## Archiving Sessions

After a benchmarking session is complete:

1. Commit TRACKER.md to `docs/perf/sessions/YYYY-MM-DD.md`
2. Update HISTORY.yaml with session summary
3. Update ideas.md with final results
4. Archive detailed experiment data:

```
docs/perf/
├── sessions/
│   ├── 2026-01-16.md
│   └── 2026-01-10.md
├── HISTORY.yaml
└── experiments/
    ├── 001-skip-zero-init/
    ├── 002-zero-copy/
    └── ...
```
