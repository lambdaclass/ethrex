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

## Phase 1.3 완료 요약

### timings.rs 확장

`OpcodeTimings` 및 `PrecompilesTimings`에 추가:
- `reset()` — 벤치마크 실행 사이 데이터 초기화
- `raw_totals()` / `raw_counts()` — 구조화된 데이터 접근

### tokamak-bench 모듈 구조

| 모듈 | 용도 |
|------|------|
| `types.rs` | BenchSuite, BenchResult, OpcodeEntry, RegressionReport, Thresholds |
| `runner.rs` | VM 초기화 + 시나리오 실행 + opcode timing 추출 |
| `report.rs` | JSON 직렬화/역직렬화, 마크다운 테이블 생성 |
| `regression.rs` | 두 BenchSuite 비교, Stable/Warning/Regression 분류 |
| `bin/runner.rs` | CLI: run / compare / report 서브커맨드 (clap) |

핵심: `ethrex-levm` with `features = ["perf_opcode_timings"]` — 이 crate에만 스코프

### CI Infrastructure

- **pr-tokamak-bench.yaml**: bench-pr → bench-main → compare-results → PR comment
- 트리거: `crates/vm/levm/**`, `crates/tokamak-bench/**` 변경 시

### 검증 결과

- `cargo build --release -p tokamak-bench` — 성공
- `cargo test -p tokamak-bench` — 11 tests pass
- `cargo test --workspace` — 0 failures
- `cargo check --features tokamak` — 성공

### 변경 파일

| 파일 | 변경 내용 |
|------|-----------|
| `crates/vm/levm/src/timings.rs` | reset(), raw_totals(), raw_counts() 추가 |
| `crates/tokamak-bench/Cargo.toml` | 의존성 + binary target 추가 |
| `crates/tokamak-bench/src/lib.rs` | 모듈 선언 |
| `crates/tokamak-bench/src/types.rs` | 신규 생성 |
| `crates/tokamak-bench/src/runner.rs` | 신규 생성 |
| `crates/tokamak-bench/src/report.rs` | 신규 생성 |
| `crates/tokamak-bench/src/regression.rs` | 신규 생성 |
| `crates/tokamak-bench/src/bin/runner.rs` | 신규 생성 |
| `.github/workflows/pr-tokamak-bench.yaml` | 신규 생성 |
| `docs/tokamak/architecture/PHASE-1-3.md` | 신규 생성 |

## Git 상태

- 브랜치: `feat/tokamak-proven-execution`
- 리모트: `origin` (tokamak-network/ethrex)

## 커밋 이력

| 커밋 | 내용 |
|------|------|
| `3ed011be8` | feat: Phase 1.2 — feature flag split, CI workflow, fork adjustments |
| `864ac9e2c` | docs: mark Phase 1.1 complete, update HANDOFF |
| `42ebbe926` | docs: fix architecture docs per Volkov R8-R10 review |
| `c1e4f988b` | docs: add ethrex architecture analysis and Phase 1.1 infrastructure |
| `36f9bf7a8` | docs: finalize DECISION.md with agent model |

## 다음 단계

### Phase 1.2 나머지

1. **CI 검증** — Push하여 `pr-tokamak.yaml` + `pr-tokamak-bench.yaml` 트리거 확인
2. **Sync 검증** — Hoodi snapsync 완료 확인 (CI runner 필요)
3. **Hive 검증** — PR Hive 6 suite + Assertoor 2 suite baseline 기록

### Phase 2: JIT Foundation

4. `tokamak-jit` crate 구현 시작
5. Cranelift 기반 JIT 컴파일 프로토타입

## 핵심 컨텍스트

- DECISION.md: **FINAL 확정** (2026-02-22)
- Volkov 점수: DECISION R6 PROCEED(7.5) → Architecture R10 PROCEED(8.25)
- 아키텍처 분석: `docs/tokamak/architecture/` 참조
- 격리 전략: Hybrid (feature flag ~30줄 + 신규 crate 3개)
- Feature flag 분할: tokamak → tokamak-jit/debugger/l2 (완료)
- Codebase: ~103K lines Rust, 28 workspace crates, 30+ CI workflows
- Test baseline: 725+ passed, 0 failed
