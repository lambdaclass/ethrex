# JIT Limitations Resolution Roadmap

**Date**: 2026-02-26
**Context**: Tokamak JIT achieves 1.46-2.53x speedup but has critical limitations blocking production deployment. This roadmap prioritizes resolution by impact and dependency order.

---

## Severity Overview

```
CRITICAL (production blockers)
  ├── G-1. LLVM Memory Lifecycle       ← 모든 것의 근본 원인
  └── G-2. Cache Eviction Effectiveness ← G-1 해결 시 자동 해결

SIGNIFICANT (v1.1 targets)
  ├── G-3. CALL/CREATE Validation Gap   ← 가장 중요한 코드가 미검증
  ├── G-4. Recursive CALL Performance   ← revmc upstream 변경 필요
  └── G-5. Parallel Compilation         ← 단일 스레드 병목

MODERATE (v1.2 optimization)
  ├── G-6. LRU Cache Policy
  ├── G-7. Constant Folding Enhancement
  └── G-8. Precompile JIT Acceleration
```

---

## Phase G-1: LLVM Memory Lifecycle [P0-CRITICAL]

> "컴파일할수록 메모리가 새는 집은 살 수 없다."

### Problem

```rust
// compiler.rs — 컴파일된 함수 포인터를 유지하기 위해 LLVM context를 의도적으로 leak
std::mem::forget(compiler);
```

- 컨트랙트 1개 컴파일 = ~1-5MB LLVM context 영구 점유
- 캐시에서 evict해도 메모리 회수 불가
- 장시간 운영 노드에서 OOM 필연적

### Root Cause

revmc의 `with_llvm_context`가 thread-local closure 기반이라 개별 함수 해제 API가 없음. LLVM Module/Context 수명이 컴파일된 함수 포인터 수명과 결합되어 있음.

### Solution Options

| Option | Approach | Effort | Risk |
|--------|----------|--------|------|
| **(a) Persistent LLVM Context** | 단일 LLVM Module에 모든 함수를 누적 컴파일. 개별 함수 삭제는 LLVM `deleteFunction()` 사용 | 24-32h | 중간 — revmc API 변경 필요 |
| **(b) Arena Allocator + Generation GC** | N개 함수를 하나의 arena에 모음. arena 내 모든 함수가 evict되면 arena 전체 해제 | 16-24h | 낮음 — revmc 변경 최소화 |
| **(c) Process-level Isolation** | 컴파일을 별도 프로세스(child process)에서 수행. shared memory로 함수 포인터 공유. 프로세스 종료 시 메모리 자동 회수 | 32-40h | 높음 — IPC 오버헤드 |

### Recommendation: **(b) Arena Allocator**

가장 적은 revmc 변경으로 메모리 bounded를 달성. arena 크기를 설정 가능하게 만들면 운영자가 메모리 상한을 제어할 수 있음.

### Acceptance Criteria

- [ ] 1000개 고유 컨트랙트 컴파일 후 메모리 사용량이 설정 상한 이내
- [ ] evict된 컨트랙트의 메모리가 실제로 회수됨 (RSS 측정)
- [ ] 기존 벤치마크 성능 회귀 없음 (< 5%)
- [ ] Hive 6/6 PASS 유지

### Dependency

- G-2는 G-1 해결 시 자동 해결됨 (별도 작업 불필요)

### Estimate: 16-32h

---

## Phase G-2: Cache Eviction Effectiveness [P0-CRITICAL]

> G-1 해결 시 자동 해결. 별도 구현 불필요.

### Problem

```rust
// lib.rs:100-106
CompilerRequest::Free { func_id } => {
    tracing::debug!("Free request for {func_id} (no-op in current PoC)");
}
```

### Solution

G-1의 arena/persistent context가 구현되면, Free 핸들러에서 실제 메모리 해제 로직 연결.

### Estimate: G-1에 포함 (추가 2-4h)

---

## Phase G-3: CALL/CREATE Dual-Execution Validation [P1-SIGNIFICANT]

> "실제 디앱의 대부분이 CALL을 포함하는데, 그 코드가 검증되지 않는 역설."

### Problem

```rust
// analyzer.rs — 외부 호출이 있으면 dual-execution 검증 스킵
if analysis.has_external_calls {
    skip_validation = true; // state-swap이 subcall을 재현할 수 없음
}
```

Uniswap, Aave 등 모든 실전 컨트랙트가 CALL을 포함 → JIT 정합성이 검증되지 않는 가장 위험한 영역.

### Solution Options

| Option | Approach | Effort | Risk |
|--------|----------|--------|------|
| **(a) TX-level Validation** | opcode-level이 아닌 TX 전체 결과(state root, gas, output)를 비교. subcall 내부는 블랙박스 처리 | 12-16h | 낮음 |
| **(b) Nested State-Swap** | subcall마다 state checkpoint를 만들어 개별 검증 | 24-32h | 높음 — 재진입 문제 |
| **(c) Shadow Execution** | JIT 실행과 병렬로 인터프리터가 같은 TX를 독립 실행. 최종 상태만 비교 | 16-24h | 중간 — 2x 리소스 |

### Recommendation: **(a) TX-level Validation**

완벽한 opcode-level 검증보다 실용적. TX 결과가 일치하면 내부 경로가 달라도 consensus 안전성은 보장됨.

### Acceptance Criteria

- [ ] CALL/CREATE 포함 바이트코드에 대해 TX-level dual-execution 검증 활성화
- [ ] ERC20 transfer, Uniswap swap 시나리오 테스트 추가
- [ ] validation mismatch 발생 시 상세 diff 로깅
- [ ] 기존 validation_mode 테스트 유지

### Dependency: G-1 (메모리 안정성 먼저)

### Estimate: 12-16h

---

## Phase G-4: JIT-to-JIT Direct Dispatch [P1-SIGNIFICANT]

> "suspend/resume 없이 JIT 코드가 직접 JIT 코드를 호출."

### Problem

현재: JIT 코드 → CALL → suspend → LEVM dispatch → (JIT or interp) → resume
오버헤드: 상태 패킹/언패킹 (~10KB), LLVM context switch, storage journal 이동

### Solution Options

| Option | Approach | Effort | Risk |
|--------|----------|--------|------|
| **(a) Inline Small Calls** | 자식 바이트코드를 부모 JIT에 인라인. 코드 크기 < 1KB이고 CALL depth < 3인 경우만 | 20-30h | 중간 — 코드 팽창 |
| **(b) JIT-to-JIT Trampoline** | JIT 코드가 캐시에서 자식 함수 포인터를 직접 조회하여 호출. suspend 불필요 | 30-40h | 높음 — revmc 변경 |
| **(c) Hybrid** | depth 1은 inline, depth 2+는 trampoline | 40-50h | 높음 |

### Recommendation: **(a) Inline Small Calls** (v1.1)

가장 일반적인 케이스(ERC20 transfer → 1 CALL)를 커버. 복잡한 트램폴린은 v1.2로 연기.

### Acceptance Criteria

- [ ] CALL depth 1, 자식 bytecode < 1KB인 경우 inline 컴파일
- [ ] ERC20Transfer 벤치마크 JIT 활성화 (현재 스킵 → 실행)
- [ ] speedup > 1.5x (현재 인터프리터 대비)
- [ ] inline 불가 시 기존 suspend/resume으로 graceful fallback

### Dependency: G-1 (inline 시 메모리 사용량 증가)

### Estimate: 20-30h

---

## Phase G-5: Parallel Compilation [P1-SIGNIFICANT]

> "멀티코어 시대에 단일 스레드 컴파일은 병목."

### Problem

```
모든 컴파일 요청 → [단일 mpsc 채널] → [단일 스레드] → 순차 처리
```

바쁜 노드에서 새 컨트랙트가 몰리면 컴파일 큐 적체. 첫 실행은 항상 인터프리터 fallback.

### Solution

```
컴파일 요청 → [work-stealing 큐] → [스레드 풀 (N workers)]
                                      ├── Worker 1: 컨트랙트 A
                                      ├── Worker 2: 컨트랙트 B
                                      └── Worker 3: 컨트랙트 C
```

### Implementation

- `mpsc` → `crossbeam` work-stealing 큐 또는 `rayon` 스레드 풀
- LLVM context는 thread-local이므로 각 worker가 독립 context 보유
- 캐시 삽입은 기존 `DashMap` (lock-free concurrent map) 그대로 사용
- worker 수 = `num_cpus::get() / 2` (VM 실행 스레드와 공유)

### Acceptance Criteria

- [ ] 컴파일 스레드 풀 크기 설정 가능 (`--jit-compile-threads N`)
- [ ] 동시 컴파일 시 race condition 없음 (같은 bytecode 중복 컴파일 방지)
- [ ] 100개 고유 컨트랙트 동시 요청 시 컴파일 지연 50% 이상 감소
- [ ] 기존 단일 스레드 동작도 유지 (N=1)

### Dependency: G-1 (arena allocator가 thread-safe여야 함)

### Estimate: 12-16h

---

## Phase G-6: LRU Cache Eviction [P2-MODERATE]

> "자주 쓰는 컨트랙트를 오래됐다고 쫓아내면 안 된다."

### Problem

현재 FIFO: 삽입 순서대로 evict → Uniswap Router처럼 매초 호출되는 컨트랙트가 먼저 evict될 수 있음.

### Solution

- `DashMap` + 별도 access timestamp 추적 (per-entry `AtomicU64`)
- eviction 시 가장 오래 접근 안 된 항목 선택
- `get()` 호출 시 write lock 대신 atomic timestamp update (lock-free)

### Acceptance Criteria

- [ ] 접근 빈도 높은 항목이 접근 빈도 낮은 항목보다 오래 캐시에 유지
- [ ] `get()` 경로에 추가 lock 없음 (atomic only)
- [ ] 벤치마크 오버헤드 < 1%

### Dependency: G-1 (evict 시 실제 메모리 해제가 가능해야 의미 있음)

### Estimate: 8-12h

---

## Phase G-7: Constant Folding Enhancement [P2-MODERATE]

> "바이트 수가 달라지면 JUMP 오프셋이 깨진다는 제약 해소."

### Problem

현재: 결과가 원래 패턴과 같은 바이트 수에 맞아야 fold 가능. `PUSH 5, PUSH 3, SUB` → U256::MAX-2 (32 bytes) → 원래 5 bytes에 안 맞아서 스킵.

### Solution Options

| Option | Approach | Effort | Risk |
|--------|----------|--------|------|
| **(a) JUMP Offset Rewriting** | fold 후 모든 JUMP/JUMPI 대상 주소를 재계산 | 16-24h | 높음 — correctness 위험 |
| **(b) NOP Padding** | 줄어든 바이트를 NOP(JUMPDEST)으로 채움 | 4-8h | 낮음 — 이미 same-length와 유사 |
| **(c) IR-level Optimization** | 바이트코드가 아닌 revmc IR 단에서 constant fold | 24-32h | 중간 — revmc 이해 필요 |

### Recommendation: **(b) NOP Padding** (quick win) → **(c) IR-level** (v1.2)

### Estimate: 4-8h (b) / 24-32h (c)

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
Phase 1 (v1.0 → v1.0.1): CRITICAL — 프로덕션 배포 전제조건
┌─────────────────────────────────────────────────────┐
│ G-1  LLVM Memory Lifecycle        [16-32h]          │
│  └── G-2  Cache Eviction (자동 해결) [+2-4h]        │
└─────────────────────────────────────────────────────┘

Phase 2 (v1.1): SIGNIFICANT — 실전 디앱 지원
┌─────────────────────────────────────────────────────┐
│ G-3  CALL/CREATE Validation       [12-16h]          │
│ G-5  Parallel Compilation         [12-16h] (병렬)   │
│  └── G-4  JIT-to-JIT Dispatch     [20-30h]          │
└─────────────────────────────────────────────────────┘

Phase 3 (v1.2): MODERATE — 최적화 및 성능 향상
┌─────────────────────────────────────────────────────┐
│ G-6  LRU Cache Policy             [8-12h]           │
│ G-7  Constant Folding Enhancement [4-8h]  (병렬)    │
│ G-8  Precompile Acceleration      [16-24h] (병렬)   │
└─────────────────────────────────────────────────────┘
```

### Timeline Summary

| Phase | Version | Tasks | Total Effort | Target Speedup |
|-------|---------|-------|-------------|----------------|
| Phase 1 | v1.0.1 | G-1, G-2 | 18-36h | — (안정성) |
| Phase 2 | v1.1 | G-3, G-4, G-5 | 44-62h | 2.5-3.5x |
| Phase 3 | v1.2 | G-6, G-7, G-8 | 28-44h | 3.5-5.0x |
| **Total** | | **8 tasks** | **90-142h** | **3.5-5.0x target** |

---

## Dependency Graph

```
G-1 (Memory Lifecycle)
 ├──→ G-2 (Cache Eviction) — 자동 해결
 ├──→ G-3 (CALL Validation)
 ├──→ G-4 (JIT-to-JIT)
 ├──→ G-5 (Parallel Compilation)
 └──→ G-6 (LRU Cache)

G-7 (Constant Folding) — 독립
G-8 (Precompile) — 독립
```

G-1이 모든 것의 선행 조건. G-7과 G-8은 독립적이므로 아무 시점에나 병렬 진행 가능.
