# Tokamak Remaining Work Roadmap

**Created**: 2026-02-24
**Context**: Overall ~35-40% complete. JIT core done (Phases 2-8). Phase A infra built, CI verification pending.

---

## Priority Classification

| Grade | Meaning | Rule |
|-------|---------|------|
| **P0** | Must-have | Launch impossible without this |
| **P1** | Important | Launch possible but quality at risk |
| **P2** | Nice-to-have | Improves experience but not blocking |
| **P3** | Backlog | Post-launch |

**Rule: P0 must ALL be done before touching P1.**

---

## Phase A: Production Foundation (P0)

> "Without Hive and sync, this is not an Ethereum client. It's a library."

### A-1. Hive Test Integration [P0] 🔧 INFRA DONE / ⏳ VERIFICATION PENDING
- ~~Add Hive test suites to `pr-tokamak.yaml` (mirror upstream `pr-main_l1.yaml`)~~ ✅
- ~~Suites: RPC Compat, Devp2p, Engine Auth, Engine Cancun, Engine Paris, Engine Withdrawals~~ ✅
- ~~Reuse upstream `check-hive-results.sh` + pinned Hive version~~ ✅
- **Verification**: All 6 Hive suites pass on `feat/tokamak-proven-execution` — ❌ NOT YET RUN
- **Infra**: `fc720f46f` — 6 Hive suites in `pr-tokamak.yaml`, Docker build with `--features tokamak-jit`, Hive Gate aggregation job
- **Remaining**: Push commit → PR CI 트리거 → Hive 6개 Suite 통과 확인

### A-2. Testnet Sync Verification [P0] 🔧 INFRA DONE / ⏳ VERIFICATION PENDING
- ~~Run Hoodi testnet sync using existing `tooling/sync/` infrastructure~~ ✅ (workflow created)
- Verify state trie validation passes — ❌ NOT YET RUN
- Document sync time + any failures — ❌ NOT YET RUN
- **Verification**: Hoodi sync completes, state root matches — ❌ NOT YET RUN
- **Infra**: `fc720f46f` — `tokamak-sync.yaml` (manual dispatch, Hoodi/Sepolia, Kurtosis + Lighthouse, `--features tokamak-jit`)
- **Remaining**: workflow_dispatch 수동 실행 → Hoodi sync 완료 확인 → 결과 문서화

### A-3. Tokamak Feature Flag Safety [P0] 🔧 INFRA DONE / ⏳ VERIFICATION PENDING
- ~~Verify `--features tokamak` does NOT break Hive tests~~ (CI checks build, Hive not yet run)
- ~~Verify `--features tokamak-jit` does NOT break Hive tests~~ (CI checks build, Hive not yet run)
- Key concern: JIT dispatch must not interfere with consensus
- **Verification**: Hive pass rate with tokamak features == without — ❌ COMPARISON NOT YET DONE
- **Infra**: Quality Gate checks all 4 feature flags (build + clippy + tests), Docker build uses `--features tokamak-jit`
- **Remaining**: A-1 Hive 통과 후 → upstream main Hive 통과율과 비교

### A-4. Phase 1.2 Completion [P0] ⏳ PARTIALLY DONE
- ~~Build verification (Phase 1.2-5): all workspace crates compile with tokamak features~~ ✅ (criteria 1-5 PASS)
- Record baseline Hive pass rate for Tokamak branch — ❌ PENDING (A-1 필요)
- Document any regressions vs upstream — ❌ PENDING
- **Verification**: Phase 1.2 criteria 1-5 PASS, criteria 6-9 PENDING (CI)
- **Remaining**: A-1/A-2 검증 완료 → criteria 6 (pr-tokamak CI), 7 (Docker), 8 (Hive baseline), 9 (Snapsync) 확인

---

## Phase B: JIT Hardening (P1)

> "JIT works but isn't production-safe yet."

### B-1. JIT Gas Accounting Alignment [P1]
- Root-cause gas mismatch between JIT and interpreter
- Known: JitOutcome::gas_used excludes intrinsic gas (handled by apply_jit_outcome)
- Unknown: Edge cases in SSTORE gas (EIP-2929 warm/cold), CALL stipend
- Verification: `test_jit_gas_matches_interpreter` passing is necessary but not sufficient
- **Verification**: Run dual-execution on full Hive engine test suite, zero gas mismatches
- **Dependency**: A-1 (need Hive for comprehensive testing)
- **Estimate**: 8-16h

### B-2. Test Quality (Volkov R24 Recommendations) [P1] ✅ DONE
- R1: Extract `make_test_db()` helper from 4 duplicate test setups ✅
- R2: Replace `let _ =` in rollback with `eprintln!` logging — deferred (low impact)
- R3: Replace `21_000u64` magic number with named constant ✅
- R4: DRY merge `init_vm` / `init_vm_interpreter_only` — deferred (needs subcall.rs refactor)
- **Verification**: All tests pass, clippy clean ✅
- **Dependency**: None
- **Estimate**: 1-2h
- **Completed**: Session 224921e1f — Created `test_helpers.rs`, added `INTRINSIC_GAS` constant, refactored 15+ duplicate test setups

### B-3. EIP-7928 BAL Recording for JIT [P1]
- 4 TODO comments exist in `host.rs` for BAL recording
- Implement BAL recording in sload/sstore JIT paths
- **Verification**: BAL entries match between JIT and interpreter execution
- **Dependency**: B-1
- **Estimate**: 4-8h

---

## Phase C: Benchmark CI & Regression Detection (P1)

> "Performance gains mean nothing without regression prevention."

### C-1. Phase 9: JIT Benchmark CI [P1]
- Add JIT benchmark job to `pr-tokamak-bench.yaml`
- Compare JIT speedup ratios between PR and base
- Flag regression if speedup drops >20%
- **Verification**: PR with intentional regression is flagged
- **Dependency**: None
- **Estimate**: 4h

### C-2. LLVM 21 CI Provisioning [P1]
- Remove `continue-on-error: true` from jit-backend CI job
- Either: package LLVM 21 in custom Docker image, OR use GitHub-hosted runner with brew
- **Verification**: JIT backend job fails the PR if compilation breaks
- **Dependency**: None
- **Estimate**: 4-8h

### C-3. Benchmark Statistics [P1] ✅ DONE
- Add warmup runs (discard first 2) ✅
- Add stddev + 95% confidence interval to output ✅
- Multiple independent trial invocations (not just loop iterations) ✅
- **Verification**: Benchmark output includes stddev, CI in JSON and markdown ✅
- **Dependency**: None
- **Estimate**: 2-4h
- **Completed**: Session 224921e1f — Created `stats.rs` module, added `--warmup` CLI param, warmup/stddev/CI support to tokamak-bench

---

## Phase D: Performance Optimization (P2)

> "From 2x to 3-5x target."

### D-1. Recursive CALL Performance [P2]
- Current: JIT suspend -> LEVM dispatch -> JIT resume is extremely slow
- Options: (a) inline small calls, (b) JIT-to-JIT direct dispatch, (c) accept limitation
- Impact: FibonacciRecursive, ERC20 scenarios currently skipped
- **Decision needed**: Which approach? Cost/benefit analysis.
- **Dependency**: B-1
- **Estimate**: 16-40h (high uncertainty)

### D-2. Bytecode Size Limit Workaround [P2]
- revmc hard limit: 24576 bytes
- Options: (a) chunk compilation, (b) interpreter fallback for large contracts, (c) upstream fix
- Impact: Push/MstoreBench/SstoreBench skip compilation
- **Decision needed**: Accept fallback or invest in chunking?
- **Dependency**: None
- **Estimate**: 8-16h

### D-3. Opcode Fusion / Constant Folding [P2]
- PUSH+PUSH+ADD -> single operation
- Requires bytecode analysis pass before compilation
- Impact: Potentially +30-50% on arithmetic-heavy contracts
- **Dependency**: D-1, D-2 (optimizations build on stable base)
- **Estimate**: 20-40h (research + implementation)

---

## Phase E: Developer Experience (P2)

> "Time-Travel Debugger MVP."

### E-1. Debugger Core: TX Replay Engine [P2]
- Replay transaction opcode-by-opcode using LEVM
- Record state snapshots at each step
- Support forward/backward navigation
- **Verification**: Can replay a known mainnet TX and show each opcode + state
- **Dependency**: A-2 (need synced state for real TX replay)
- **Estimate**: 20-30h

### E-2. Debugger CLI [P2]
- Interactive CLI: `step`, `step-back`, `break <pc>`, `inspect <slot>`, `continue`
- Print: opcode, stack top 4, gas remaining, storage reads/writes
- **Verification**: Demo video showing stepping through a real TX
- **Dependency**: E-1
- **Estimate**: 10-15h

### E-3. debug_timeTravel RPC Endpoint [P2]
- JSON-RPC method: `debug_timeTravel(txHash, { stepIndex, breakpoints })`
- Returns: opcode, stack, memory slice, storage diff
- **Verification**: curl to local node returns correct step data
- **Dependency**: E-1, E-2
- **Estimate**: 8-12h

---

## Phase F: Ecosystem & Launch (P3)

### F-1. Cross-Client Benchmarking [P3]
- Run same scenarios on Geth and Reth via JSON-RPC
- Compare TX execution time, state root computation, sync speed
- **Dependency**: A-2, C-1
- **Estimate**: 16-24h

### F-2. Public Dashboard [P3]
- clients.tokamak.network
- Time-series benchmark results, Hive pass rates, sync times
- **Dependency**: F-1, C-1
- **Estimate**: 20-30h

### F-3. L2 Integration [P3]
- Implement `tokamak-l2` feature: custom fee config, L2 hooks
- Currently: zero code behind the feature flag
- **Dependency**: A-1 (L1 must work first)
- **Estimate**: 40-80h (high uncertainty, depends on L2 spec)

### F-4. Security Audit Prep [P3]
- JIT fuzzing (bytecode generation + differential testing)
- unsafe code audit (transmute in execution.rs, mem::forget in compiler.rs)
- **Dependency**: B-1, D-1
- **Estimate**: 40h

### F-5. Mainnet Full Sync [P3]
- Full mainnet state sync as Tokamak client
- Verify state root matches at head
- **Dependency**: A-2, A-3
- **Estimate**: 24-48h (mostly wait time)

---

## Execution Order

```
Week 1:  [P0] A-1 + A-2 (parallel) → A-3 → A-4  🔧 INFRA DONE, ⏳ CI VERIFICATION PENDING
Week 2:  [P1] B-2 ✅ + C-2 + C-3 ✅ (parallel) → B-1
Week 3:  [P1] B-1 (continued) + C-1 → B-3
Week 4:  [P2] D-1 decision + D-2 → E-1 start
Week 5+: [P2] E-1 + E-2 → D-3 → E-3
Later:   [P3] F-1 → F-2 → F-3 → F-4 → F-5
```

---

## Decisions Needed

| Decision | Options | Recommendation |
|----------|---------|----------------|
| Recursive CALL strategy | (a) Inline (b) JIT-to-JIT (c) Accept | (c) Accept for v1.0, (b) for v1.1 |
| Bytecode size limit | (a) Chunk (b) Fallback (c) Upstream fix | (b) Fallback -- least effort, already works |
| L2 timeline | (a) Now (b) After mainnet (c) Skip | (b) After mainnet -- L1 correctness first |
| Debugger scope | (a) Full Web UI (b) CLI only (c) Skip | (b) CLI MVP -- prove value, web UI in v1.1 |
