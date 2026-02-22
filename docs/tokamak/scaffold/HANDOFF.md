# Handoff: Tokamak Ethereum Client

## 현재 작업 상태

| 항목 | 상태 |
|------|------|
| Phase 0-4: 개발 환경 구축 (monorepo) | **완료** |
| Phase 0-1: ethrex 코드베이스 분석 | **완료** |
| Phase 0-2: 대안 평가 (Reth 등) | **완료** |
| Phase 0-3: DECISION.md 작성 | **완료** |

## Phase 0-1 분석 결과 요약

ethrex 코드베이스 (133K줄 Rust) 분석 완료:
- **LEVM**: 자체 EVM 구현. `vm.rs:528-663`에 메인 실행 루프 (match opcode 패턴)
- **Hook 시스템**: `VMType::L1 | L2(FeeConfig)` enum + `Hook` trait (`hook.rs`)로 L1/L2 분기
- **L2Hook**: `l2_hook.rs`에 완전한 L2 구현 (845줄). fee token, privileged tx, operator fee 등
- **Tracing**: `LevmCallTracer` (`tracing.rs`) — Geth callTracer 호환. Time-Travel 확장 대상
- **Benchmarking**: `perf_opcode_timings` feature + `OpcodeTimings` struct (`timings.rs`)
- **Opcode Table**: `build_opcode_table()` (`opcodes.rs:385`) — fork별 분기. JIT 대체 대상
- **ZK**: SP1, RISC0, ZisK, OpenVM 4개 프루버 네이티브 지원

## Phase 0-2 결정 매트릭스 요약

| 기준 (가중치) | ethrex | Reth | 처음부터 | revm |
|--------------|--------|------|---------|------|
| 메인넷 동기화 (25%) | 5 | 4 | 1 | 1 |
| EVM 수정 가능성 (25%) | 5 | 2 | 4 | 3 |
| ZK 호환성 (20%) | 5 | 1 | 2 | 1 |
| 코드베이스 관리성 (15%) | 4 | 2 | 5 | 3 |
| L2 아키텍처 정합성 (15%) | 5 | 3 | 3 | 1 |
| **가중 합계** | **4.85** | **2.45** | **2.65** | **1.60** |

**결정: ethrex fork** — 자세한 내용은 `docs/tokamak/DECISION.md` 참조

## Phase 0-3 산출물

- `docs/tokamak/DECISION.md` — 결정 문서 (DRAFT, 팀 리뷰 대기)

## 완료된 작업

### Cargo workspace monorepo 생성 (`/Users/jason/workspace/tokamak-client/`)

7개 크레이트 스캐폴딩 완료:

```
crates/
├── tokamak-common/      — 공유 타입 (BlockRef, ExecutionStep, TokamakConfig 등)
├── tokamak-evm/         — EVM 실행 엔진 (TokamakExecutor, TokamakInspector)
├── tokamak-jit/         — JIT 컴파일러 인터페이스 (JitCompiler, JitCache, ExecutionProfiler)
├── tokamak-benchmark/   — 벤치마크 프레임워크 (Runner, Comparator, DifferentialTester, Reporter)
├── tokamak-debugger/    — Time-Travel 디버거 (ReplayEngine, SnapshotChain, BreakpointManager)
├── tokamak-rpc/         — JSON-RPC (debug_timeTravel 타입 정의)
└── tokamak-node/        — 메인 바이너리 (CLI: --jit, --debug, --benchmark)
```

### 빌드 & 테스트 상태

- `cargo build --workspace` — **성공** (0 warnings)
- `cargo test --workspace` — **25 tests 전부 통과**
- `cargo clippy --workspace -- -D warnings` — **통과** (0 warnings)

### CI/CD 파이프라인

- `.github/workflows/ci.yml` — check, test, clippy, fmt, audit
- `.github/workflows/benchmark.yml` — PR quick-bench, main full-bench (Phase 2에서 활성화)
- `rust-toolchain.toml` — stable (현재 1.93.1)

### 핵심 의존성

- `alloy-primitives 0.8` (serde feature) — B256, U256, Address
- `revm 19` — EVM 인터프리터 (Phase 1 기본 실행)
- `thiserror 2` — 에러 타입
- `tracing` — 로깅
- `clap 4` — CLI
- Cranelift — 주석 처리됨 (Phase 4에서 활성화)

## 변경된 파일 목록

```
Cargo.toml                          — workspace 루트
rust-toolchain.toml
.gitignore
CLAUDE.md
.github/workflows/ci.yml
.github/workflows/benchmark.yml
crates/tokamak-common/Cargo.toml
crates/tokamak-common/src/lib.rs
crates/tokamak-common/src/types.rs
crates/tokamak-evm/Cargo.toml
crates/tokamak-evm/src/lib.rs
crates/tokamak-evm/src/executor.rs
crates/tokamak-evm/src/inspector.rs
crates/tokamak-jit/Cargo.toml
crates/tokamak-jit/src/lib.rs
crates/tokamak-jit/src/compiler.rs
crates/tokamak-jit/src/cache.rs
crates/tokamak-jit/src/profiler.rs
crates/tokamak-benchmark/Cargo.toml
crates/tokamak-benchmark/src/lib.rs
crates/tokamak-benchmark/src/runner.rs
crates/tokamak-benchmark/src/comparator.rs
crates/tokamak-benchmark/src/differential.rs
crates/tokamak-benchmark/src/reporter.rs
crates/tokamak-benchmark/src/scenarios.rs
crates/tokamak-debugger/Cargo.toml
crates/tokamak-debugger/src/lib.rs
crates/tokamak-debugger/src/replay.rs
crates/tokamak-debugger/src/snapshot.rs
crates/tokamak-debugger/src/breakpoint.rs
crates/tokamak-rpc/Cargo.toml
crates/tokamak-rpc/src/lib.rs
crates/tokamak-rpc/src/types.rs
crates/tokamak-rpc/src/methods.rs
crates/tokamak-node/Cargo.toml
crates/tokamak-node/src/main.rs
docs/tokamak/DECISION.md            — NEW (Phase 0-3)
```

## 다음 단계 — Phase 1.1

### 즉시 필요

1. **DECISION.md 팀 리뷰** — DRAFT 상태. 팀 확인 후 확정
2. **git init + 초기 커밋** — 사용자가 git init을 중단함. 수동으로 실행 필요
3. **GitHub 원격 레포 생성** — `tokamak-network/tokamak-client` 등

### Phase 1.1: Fork & 환경 구축 (Week 1-2)

4. ethrex fork → `tokamak-client` 레포
5. 메인넷/Holesky 빌드 검증
6. CI 파이프라인 설정
7. Hive 테스트 프레임워크 통합 시작

### Volkov PROCEED 조건 미충족 항목

- EXIT 기준 미정의 (6개월 내 Hive 95% 미달 시 재평가 제안)
- Tier S 기능 2주 PoC 미실행 (Phase 1.1 착수 후 `perf_opcode_timings` 기반 벤치마크 PoC 추천)
- 구체 인력 배정 미확정 (팀 결정 필요)

## 핵심 컨텍스트

- 개발 계획 전문: `docs/tokamak/` 내 문서들
  - `vision.md` — 전체 비전 ("Performance you can see, verify, and debug")
  - `DECISION.md` — ethrex fork 결정 문서 (NEW)
  - `context/competitive-landscape.md` — 경쟁 분석
  - `context/volkov-reviews.md` — R1-R5 리뷰 이력
  - `features/01~03-*.md` — Tier S 기능 상세
- 포지셔닝: "Performance you can see, verify, and debug"
- Tier S 기능 3개: JIT EVM + Continuous Benchmarking + Time-Travel Debugger
- Base client: **ethrex fork 확정** (DECISION.md)
- 현재 크레이트들은 인터페이스 + stub 수준. Phase별로 구현 채워넣는 구조
