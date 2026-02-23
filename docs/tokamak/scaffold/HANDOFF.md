# Handoff: Tokamak Ethereum Client

## 현재 작업 상태

| 항목 | 상태 |
|------|------|
| Phase 0-4: 개발 환경 구축 (monorepo) | **완료** |
| Phase 0-1: ethrex 코드베이스 분석 | **완료** |
| Phase 0-2: 대안 평가 (Reth 등) | **완료** |
| Phase 0-3: DECISION.md 작성 | **완료 (FINAL)** |
| Phase 0-3a: Volkov R6 리뷰 + 수정 | **완료** |
| Phase 0-3b: DECISION.md 확정 | **완료** |
| Phase 1.1-1: 아키텍처 분석 문서 | **완료** |
| Phase 1.1-2: Skeleton crate + feature flag | **완료** |
| Phase 1.1-3: 빌드 검증 + CI 계획 | **완료** |
| Phase 1.1-4: Volkov R8-R10 리뷰 + 수정 | **완료 (8.25 PROCEED)** |
| Phase 1.2-1: Feature flag 분할 | **완료** |
| Phase 1.2-2: pr-tokamak.yaml CI 워크플로우 | **완료** |
| Phase 1.2-3: Fork CI 조정 (snapsync image) | **완료** |
| Phase 1.2-4: PHASE-1-2.md 문서화 | **완료** |
| Phase 1.2-5: 빌드 검증 | **진행중** |
| Phase 1.2-6: Sync & Hive 검증 (CI 필요) | **미착수** |
| Phase 1.3-1: timings.rs accessor methods | **완료** |
| Phase 1.3-2: tokamak-bench 모듈 구현 | **완료** |
| Phase 1.3-3: pr-tokamak-bench.yaml CI | **완료** |
| Phase 1.3-4: PHASE-1-3.md 문서화 | **완료** |
| Phase 2-1: JIT infra in LEVM (jit/) | **완료** |
| Phase 2-2: vm.rs JIT dispatch 통합 | **완료** |
| Phase 2-3: tokamak-jit revmc adapter | **완료** |
| Phase 2-4: Fibonacci PoC 테스트 | **완료** |
| Phase 2-5: CI, benchmark, docs | **완료** |
| Phase 3-1: JitBackend trait (dispatch.rs) | **완료** |
| Phase 3-2: LevmHost (host.rs) | **완료** |
| Phase 3-3: Execution bridge (execution.rs) | **완료** |
| Phase 3-4: RevmcBackend JitBackend impl | **완료** |
| Phase 3-5: vm.rs JIT dispatch wiring | **완료** |
| Phase 3-6: Backend registration + E2E tests | **완료** |
| Phase 3-7: PHASE-3.md + HANDOFF update | **완료** |

## Phase 3 완료 요약

### 핵심 변경: JIT Execution Wiring

Phase 2에서 컴파일만 가능했던 JIT 코드를 실제 실행 가능하게 연결.

### 의존성 역전 패턴 (Dependency Inversion)

LEVM은 `tokamak-jit`에 의존할 수 없음 (순환 참조). 해결:
- `JitBackend` trait을 LEVM `dispatch.rs`에 정의
- `tokamak-jit::RevmcBackend`가 구현
- 런타임에 `register_backend()`로 등록

### 새 모듈

| 모듈 | 위치 | 용도 |
|------|------|------|
| `JitBackend` trait | `levm/src/jit/dispatch.rs` | 실행 백엔드 인터페이스 |
| `host.rs` | `tokamak-jit/src/` | revm Host ↔ LEVM 상태 브릿지 |
| `execution.rs` | `tokamak-jit/src/` | JIT 실행 브릿지 (Interpreter + Host 구성) |

### revm Host 매핑 (v14.0)

19개 required methods 구현. 주요 매핑:
- `basefee()` → `env.base_fee_per_gas`
- `block_hash(n)` → `db.store.get_block_hash(n)`
- `sload_skip_cold_load()` → `db.get_storage_value()`
- `sstore_skip_cold_load()` → `db.update_account_storage()`
- `load_account_info_skip_cold_load()` → `db.get_account()` + code lookup
- `tload/tstore` → `substate.get_transient/set_transient`
- `log()` → `substate.add_log()`

### vm.rs 변경

`run_execution()` 내 인터프리터 루프 전:
```
JIT_STATE.counter.increment()
try_jit_dispatch() → execute_jit() → apply_jit_outcome()
```
JIT 실행 실패 시 인터프리터로 fallback.

### E2E 테스트 (revmc-backend feature 뒤)

- `test_fibonacci_jit_execution` — 전체 VM dispatch 경로 통과 JIT 실행
- `test_fibonacci_jit_vs_interpreter_validation` — JIT vs 인터프리터 결과 비교

### Phase 3 범위 제한 (Phase 4에서 처리)

- CALL/CREATE 중첩 지원 — JIT에서 발생 시 에러 반환
- 자동 컴파일 트리거 — 카운터 추적만, 자동 컴파일 미구현
- LRU 캐시 eviction — 캐시 무제한 증가
- is_static 전파 — PoC에서 false 고정
- Gas refund 처리 — finalize_execution에 위임

## Phase 2 완료 요약

### 핵심 결정

Cranelift은 i256 미지원으로 불가. **revmc (Paradigm, LLVM backend)** 채택.

### 아키텍처: 2-Location 전략

- `ethrex-levm/src/jit/` — 경량 인프라 (cache, counter, dispatch). 외부 dep 없음.
- `tokamak-jit` — 무거운 revmc/LLVM 백엔드. `revmc-backend` feature flag 뒤에.

### LEVM JIT 인프라 (`crates/vm/levm/src/jit/`)

| 모듈 | 용도 |
|------|------|
| `types.rs` | JitConfig, JitOutcome, AnalyzedBytecode |
| `analyzer.rs` | 기본 블록 경계 식별 |
| `counter.rs` | 실행 카운터 (Arc<RwLock<HashMap>>) |
| `cache.rs` | CompiledCode (type-erased fn ptr) + CodeCache |
| `dispatch.rs` | JitState + try_jit_dispatch() |

### tokamak-jit Crate

| 모듈 | 용도 |
|------|------|
| `error.rs` | JitError enum |
| `adapter.rs` | LEVM U256/H256/Address/Gas ↔ revm 타입 변환 |
| `compiler.rs` | revmc EvmCompiler + LLVM 래퍼 |
| `backend.rs` | RevmcBackend (compile_and_cache, analyze) |
| `validation.rs` | JIT vs interpreter 이중 실행 검증 |
| `tests/fibonacci.rs` | Fibonacci PoC (fib(0)..fib(20) 검증) |

### vm.rs 통합

`run_execution()` 내 precompile 체크 후, 인터프리터 루프 전:
- `JIT_STATE.counter.increment()` — 실행 카운트 추적
- Phase 3에서 `try_jit_dispatch()` → JIT 실행 경로 활성화 예정

### CI

- `pr-tokamak.yaml` — `jit-backend` job 추가 (LLVM 18 설치 + revmc-backend 빌드/테스트)
- 기존 quality-gate job은 LLVM 없이 기본 기능만 체크

### 검증 결과

- `cargo check --features tokamak` — 성공
- `cargo check -p tokamak-jit` — 성공 (revmc 없이)
- `cargo test -p tokamak-jit` — 7 tests pass (fibonacci 포함)
- `cargo test -p ethrex-levm --features tokamak-jit -- jit::` — 8 tests pass
- `cargo clippy --features tokamak -- -D warnings` — clean

### 변경 파일 (총 ~1,100 lines 신규)

| 파일 | 변경 |
|------|------|
| `crates/vm/levm/src/jit/` (6 files) | 신규 (~370 lines) |
| `crates/vm/levm/src/lib.rs` | +2 lines |
| `crates/vm/levm/src/vm.rs` | +15 lines |
| `crates/vm/tokamak-jit/` (8 files) | 신규/변경 (~650 lines) |
| `crates/tokamak-bench/src/jit_bench.rs` | 신규 (~65 lines) |
| `crates/tokamak-bench/src/lib.rs` | +1 line |
| `.github/workflows/pr-tokamak.yaml` | jit-backend job 추가 |
| `docs/tokamak/architecture/PHASE-2.md` | 신규 |

## Git 상태

- 브랜치: `feat/tokamak-proven-execution`
- 리모트: `origin` (tokamak-network/ethrex)

## 커밋 이력

| 커밋 | 내용 |
|------|------|
| (pending) | feat: Phase 2 — JIT foundation with revmc integration |
| `c00435a33` | ci(l1): add rustfmt/clippy components to pr-tokamak workflow |
| `cfb161652` | style(l1): fix cargo fmt formatting in tokamak-bench |
| `f6d6ac3b6` | feat: Phase 1.3 — benchmarking foundation with opcode timing CI |
| `3ed011be8` | feat: Phase 1.2 — feature flag split, CI workflow, fork adjustments |
| `864ac9e2c` | docs: mark Phase 1.1 complete, update HANDOFF for next phases |

## 다음 단계

### Phase 4: Production JIT

1. **Automatic compilation trigger** — counter threshold → compile in background
2. **Nested CALL/CREATE** — suspend JIT, call interpreter, resume
3. **LRU cache eviction** — bound cache size, evict cold entries
4. **is_static propagation** — from CallFrame to JIT Interpreter
5. **Gas refund reconciliation** — exact match JIT ↔ interpreter
6. **Tracing integration** — JIT fallback event logging
7. **Production error recovery** — graceful fallback with metrics

## 핵심 컨텍스트

- DECISION.md: **FINAL 확정** (2026-02-22)
- Volkov 점수: DECISION R6 PROCEED(7.5) → Architecture R10 PROCEED(8.25)
- 아키텍처 분석: `docs/tokamak/architecture/` 참조
- 격리 전략: Hybrid (feature flag ~45줄 + 신규 crate 내 ~650줄)
- Feature flag 분할: tokamak → tokamak-jit/debugger/l2 (완료)
- revmc: git rev `4995ac64fb4e` (2026-01-23), LLVM backend
- Codebase: ~103K lines Rust, 28 workspace crates, 30+ CI workflows
- Test baseline: 725+ passed, 0 failed
