# JIT Limitations Resolution Roadmap

**Date**: 2026-02-27
**Context**: Tokamak JIT achieves 1.46-2.53x speedup. Critical limitations (G-1/G-2) resolved. All significant issues (G-3/G-4/G-5) resolved. G-6 LRU ✅, G-7 enhanced ✅. Remaining: G-8 (Precompile).

---

## Severity Overview

```
CRITICAL (production blockers) — ALL RESOLVED ✅
  ├── G-1. LLVM Memory Lifecycle       ✅ Arena allocator (f8e9ba540)
  └── G-2. Cache Eviction Effectiveness ✅ Auto-resolved by G-1

SIGNIFICANT (v1.1 targets) — ALL RESOLVED ✅
  ├── G-3. CALL/CREATE Validation Gap   ✅ TX-level validation (8c05d3412)
  ├── G-4. JIT-to-JIT Direct Dispatch   ✅ Fast dispatch in VM layer
  └── G-5. Parallel Compilation         ✅ CompilerThreadPool (299d03720)

MODERATE (v1.2 optimization) — 2/3 RESOLVED
  ├── G-6. LRU Cache Policy             ✅ AtomicU64 LRU eviction
  ├── G-7. Constant Folding Enhancement ✅ 22 opcodes + unary (43026d7cf)
  └── G-8. Precompile JIT Acceleration
```

---

## Phase G-1: LLVM Memory Lifecycle [P0-CRITICAL] ✅ DONE

> "컴파일할수록 메모리가 새는 집은 살 수 없다."

### Solution Implemented: **(b) Arena Allocator**

- `ArenaManager` + `ArenaEntry` + `FuncSlot` types in `levm/jit/arena.rs`
- `ArenaCompiler` in `tokamak-jit/compiler.rs` — stores compilers instead of leaking
- `thread_local! ArenaState` in `lib.rs` — manages arena rotation + eviction-triggered freeing
- `CompilerRequest::Free{slot}` and `FreeArena{arena_id}` request variants
- `JitConfig` extended: `arena_capacity`, `max_arenas`, `max_memory_mb`
- `JitMetrics` extended: `arenas_created`, `arenas_freed`, `functions_evicted`

### Verification: 12 arena + 4 ArenaCompiler tests, all 178 tests pass ✅

### Completed: 2026-02-26 — f8e9ba540

---

## Phase G-2: Cache Eviction Effectiveness [P0-CRITICAL] ✅ DONE

> Auto-resolved by G-1 arena system.

- `Free{slot}` handler decrements arena live count and frees empty arenas
- `cache.insert()` returns `Option<FuncSlot>` on eviction → `ArenaManager::mark_evicted()`
- No additional implementation needed

### Completed: 2026-02-27 — auto-resolved by G-1 (f8e9ba540)

---

## Phase G-3: CALL/CREATE Dual-Execution Validation [P1-SIGNIFICANT] ✅ DONE

> "실제 디앱의 대부분이 CALL을 포함하는데, 그 코드가 검증되지 않는 역설."

### Solution Implemented

- Removed `!compiled.has_external_calls` guard from `vm.rs` validation path
- Dual-execution validation now runs for ALL bytecodes including CALL/CREATE/DELEGATECALL/STATICCALL
- Interpreter replay handles sub-calls natively via `handle_call_opcodes()`
- Refactored test infrastructure: shared `MismatchBackend`, `make_external_call_bytecode()`, `setup_call_vm()`, `run_g3_mismatch_test()` helpers

### Verification: 5 G-3 tests (CALL/STATICCALL/DELEGATECALL mismatch + pure regression + combined), 41 total tokamak-jit tests ✅

### Completed: 2026-02-27 — 8c05d3412

---

## Phase G-4: JIT-to-JIT Direct Dispatch [P1-SIGNIFICANT] ✅ DONE

> "suspend/resume 없이 JIT 코드가 직접 JIT 코드를 호출."

### Problem

현재: JIT 코드 → CALL → suspend → LEVM dispatch → (JIT or interp) → resume
오버헤드: 상태 패킹/언패킹 (~10KB), LLVM context switch, storage journal 이동

### Solution Implemented: **Fast JIT-to-JIT Dispatch** (variant of option b)

Instead of bytecode inlining (option a) or revmc-level trampolines (option b), implemented
**VM-layer fast dispatch**: when a parent JIT-compiled contract executes a CALL, the VM
checks if the child bytecode is also JIT-compiled and executes it directly via
`JitState.execute_jit()`, bypassing the full LEVM interpreter dispatch.

Key design decisions:
- **VM-layer approach**: Modifies `handle_jit_subcall()` CALL arm in `vm.rs` — no revmc changes needed
- **Cache check at dispatch time**: `try_jit_dispatch(&JIT_STATE, &child_hash, fork)` — O(1) lookup
- **Recursive suspend/resume**: If child JIT code also suspends (nested CALL), handled via loop
- **Error-safe fallback**: JIT errors during child execution treated as revert (not interpreter fallback) since state may be partially mutated
- **CREATE excluded**: CREATE init code uses interpreter (needs `validate_contract_creation`)
- **Precompile guard**: Precompile addresses always use interpreter path
- **Configurable**: `enable_jit_dispatch` in `JitConfig` (default: true)
- **Observable**: `jit_to_jit_dispatches` metric in `JitMetrics`

### Verification: 10 G-4 tests, 48 total tokamak-jit tests ✅

Tests cover: simple STATICCALL, checked STATICCALL, revert child, 3-level nesting,
cache miss fallback, differential (JIT vs interpreter), CREATE factory, depth limit,
config disable, multiple calls.

### Completed: 2026-02-27

### Dependency: G-1 ✅ (arena memory management)

---

## Phase G-5: Parallel Compilation [P1-SIGNIFICANT] ✅ DONE

> "멀티코어 시대에 단일 스레드 컴파일은 병목."

### Solution Implemented

- Replaced single `CompilerThread` (mpsc) with `CompilerThreadPool` (crossbeam-channel multi-consumer)
- Configurable N workers via `JitConfig.compile_workers` (default: `num_cpus / 2`, min 1)
- Each worker has its own `thread_local! ArenaState` — LLVM context thread-affinity preserved
- Deduplication guard (`compiling_in_progress` set) prevents duplicate compilations across workers
- `crossbeam-channel` unbounded channel for fair work distribution

### Verification: 4 G-5 tests (concurrent compilation, single worker equiv, deduplication guard, different keys), 48 total tokamak-jit tests ✅

### Completed: 2026-02-27 — 299d03720

---

## Phase G-6: LRU Cache Eviction [P2-MODERATE] ✅ DONE

> "자주 쓰는 컨트랙트를 오래됐다고 쫓아내면 안 된다."

### Solution Implemented

Replaced FIFO (`VecDeque<CacheKey>` insertion order) with LRU eviction using per-entry `AtomicU64` timestamps:

- `CacheEntry` wrapper: `Arc<CompiledCode>` + `AtomicU64` last_access timestamp
- `CodeCache.access_counter: Arc<AtomicU64>` — monotonic counter outside `RwLock`
- `get()`: atomically updates entry timestamp under read lock (2 atomic ops, no write lock)
- `insert()`: evicts entry with minimum `last_access` via O(n) scan (n ≤ max_cache_entries)
- Removed `insertion_order: VecDeque<CacheKey>` entirely
- No new crate dependencies (uses std `AtomicU64`)

### Acceptance Criteria

- [x] 접근 빈도 높은 항목이 접근 빈도 낮은 항목보다 오래 캐시에 유지
- [x] `get()` 경로에 추가 lock 없음 (atomic only — `fetch_add` + `store` under read lock)
- [x] 벤치마크 오버헤드 < 1% (~2-5ns per `get()` from 2 atomic operations)

### Verification: 9 cache unit tests + 5 integration tests, 53 total tokamak-jit tests ✅

### Completed: 2026-02-27

### Dependency: G-1 ✅ (evict 시 실제 메모리 해제가 가능해야 의미 있음)

---

## Phase G-7: Constant Folding Enhancement [P2-MODERATE] ✅ DONE

> "6개 옵코드로는 최적화 기회가 제한적."

### Solution Implemented

Instead of NOP padding or IR-level optimization, **expanded the opcode set** from 6 to 22:

- **14 new binary opcodes**: DIV, SDIV, MOD, SMOD, EXP, SIGNEXTEND, LT, GT, SLT, SGT, EQ, SHL, SHR, SAR
- **2 new unary opcodes**: NOT, ISZERO (new `UnaryPattern` type + `detect_unary_patterns()` scanner)
- Signed arithmetic helpers: `is_negative`, `negate`, `abs_val`, `u256_from_bool` (exact LEVM semantics)
- Refactored `eval_op` into 6 extracted helpers for readability
- Extracted shared `write_folded_push()` eliminating duplicate rewrite logic
- Same-length constraint still applies — results that exceed original byte count are skipped

### Still Missing (low priority)

- **BYTE** (0x1A): Binary — easy addition
- **ADDMOD/MULMOD** (0x08/0x09): Ternary — needs 3-operand pattern detector

### Verification: 68 unit tests + 8 integration tests (76 total), clippy clean both states ✅

### Completed: 2026-02-27 — 43026d7cf

---

## Phase G-8: Precompile JIT Acceleration [P2-MODERATE]

> "ECADD, KECCAK256 등 암호 연산이 Host trait 경유로 인한 call overhead."

### Problem

Precompile 호출은 JIT 코드 → Host trait → Rust 함수 → 외부 라이브러리(blst, sha2). 호출 오버헤드가 있고 LLVM 최적화의 이점을 받지 못함.

### Solution

- 자주 쓰이는 precompile (KECCAK256, ECADD, ECMUL)에 대해 LLVM IR에서 직접 extern call 생성
- Host trait 경유 없이 네이티브 함수 호출
- 나머지 precompile은 기존 Host 경로 유지

### Acceptance Criteria

- [ ] KECCAK256 precompile call overhead 50% 이상 감소
- [ ] ManyHashes 벤치마크 speedup 1.46x → 2.0x 이상
- [ ] precompile 결과 정합성 테스트

### Dependency: 없음 (독립)

### Estimate: 16-24h

---

## Execution Order

```
Phase 1 (v1.0.1): CRITICAL — ✅ ALL DONE
┌─────────────────────────────────────────────────────┐
│ G-1  LLVM Memory Lifecycle        ✅ f8e9ba540       │
│  └── G-2  Cache Eviction          ✅ auto-resolved   │
└─────────────────────────────────────────────────────┘

Phase 2 (v1.1): SIGNIFICANT — ALL DONE ✅
┌─────────────────────────────────────────────────────┐
│ G-3  CALL/CREATE Validation       ✅ 8c05d3412       │
│ G-5  Parallel Compilation         ✅ 299d03720       │
│  └── G-4  JIT-to-JIT Dispatch     ✅ Fast dispatch   │
└─────────────────────────────────────────────────────┘

Phase 3 (v1.2): MODERATE — 2/3 DONE
┌─────────────────────────────────────────────────────┐
│ G-6  LRU Cache Policy             ✅ AtomicU64 LRU   │
│ G-7  Constant Folding Enhancement ✅ 43026d7cf       │
│ G-8  Precompile Acceleration      [ ] 16-24h         │
└─────────────────────────────────────────────────────┘
```

### Timeline Summary

| Phase | Version | Tasks | Status | Remaining Effort |
|-------|---------|-------|--------|-----------------|
| Phase 1 | v1.0.1 | G-1, G-2 | **✅ ALL DONE** | 0h |
| Phase 2 | v1.1 | G-3, G-4, G-5 | **✅ ALL DONE** | 0h |
| Phase 3 | v1.2 | G-6, G-7, G-8 | **2/3 DONE** (G-8 remaining) | 16-24h |
| **Total** | | **8 tasks** | **7/8 DONE** | **16-24h remaining** |

---

## Dependency Graph

```
G-1 (Memory Lifecycle) ✅
 ├──→ G-2 (Cache Eviction) ✅ auto-resolved
 ├──→ G-3 (CALL Validation) ✅
 ├──→ G-4 (JIT-to-JIT) ✅
 ├──→ G-5 (Parallel Compilation) ✅
 └──→ G-6 (LRU Cache) ✅

G-7 (Constant Folding) ✅
G-8 (Precompile) ← REMAINING
```

G-1이 모든 것의 선행 조건이었으나 이미 완료. G-4 (JIT-to-JIT) 완료. G-6 (LRU) 완료. 남은 작업: G-8 (Precompile) — 독립 진행 가능.
