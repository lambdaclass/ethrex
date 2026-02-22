# Handoff: Tokamak Ethereum Client

## 현재 작업 상태

| 항목 | 상태 |
|------|------|
| Phase 0-4: 개발 환경 구축 (monorepo) | **완료** |
| Phase 0-1: ethrex 코드베이스 분석 | **완료** |
| Phase 0-2: 대안 평가 (Reth 등) | **완료** |
| Phase 0-3: DECISION.md 작성 | **완료 (FINAL)** |
| Phase 0-3a: Volkov R6 리뷰 + 수정 | **완료** |
| Phase 0-3b: DECISION.md 확정 (이전 세션에서 Volkov PROCEED) | **완료** |
| Phase 1.1-1: 아키텍처 분석 문서 | **완료** |
| Phase 1.1-2: Skeleton crate + feature flag | **완료** |
| Phase 1.1-3: 빌드 검증 + CI 계획 | **진행중** |

## 이번 세션에서 수행한 작업

### 1. 아키텍처 분석 문서 4건 작성

`docs/tokamak/architecture/` 하위에 작성:

- **OVERVIEW.md** — 전체 아키텍처, 25+2 crate 의존성 그래프 (13-layer), 노드 시작 흐름, 빌드 프로파일, feature flag 전체 목록, CI 워크플로우 29개 분류
- **LEVM.md** — VM 구조체 13개 필드, 트랜잭션 실행 흐름 (prepare→run→finalize), 메인 루프 듀얼 디스패치 구조 (vm.rs:528-663), Hook 시스템, Substate 체크포인팅, Lint 설정
- **MODIFICATION-POINTS.md** — Tokamak 수정 지점 5개 + Hybrid 격리 전략 (feature flag ~30줄 + 신규 crate 3개), upstream 충돌 위험도 평가
- **PHASE-1-1.md** — Phase 1.1 상세 실행 계획, CI 파이프라인 설계, 성공 기준 7개

### 2. Skeleton crate 3개 생성

| Crate | Path | Purpose |
|-------|------|---------|
| `tokamak-jit` | `crates/vm/tokamak-jit/` | JIT 컴파일러 (Phase 3) |
| `tokamak-bench` | `crates/tokamak-bench/` | 벤치마크 러너 (Phase 1.3) |
| `tokamak-debugger` | `crates/tokamak-debugger/` | Time-Travel Debugger (Phase 2) |

모두 빌드 성공 확인 (`cargo check` PASS).

### 3. `tokamak` Feature Flag 선언

Feature propagation chain 구축:
```
cmd/ethrex → ethrex-vm → ethrex-levm
  tokamak     tokamak     tokamak
```

`cargo check -p ethrex-levm --features tokamak` PASS.

### 4. Workspace 등록

Root `Cargo.toml` members에 3개 skeleton crate 추가.

## Git 상태

- 브랜치: `feat/tokamak-proven-execution`
- 리모트: `origin` (tokamak-network/ethrex)
- 마지막 커밋: `36f9bf7a8` (이전 세션)

## 변경된 파일 목록

### 신규 생성
- `docs/tokamak/architecture/OVERVIEW.md`
- `docs/tokamak/architecture/LEVM.md`
- `docs/tokamak/architecture/MODIFICATION-POINTS.md`
- `docs/tokamak/architecture/PHASE-1-1.md`
- `crates/vm/tokamak-jit/Cargo.toml`
- `crates/vm/tokamak-jit/src/lib.rs`
- `crates/tokamak-bench/Cargo.toml`
- `crates/tokamak-bench/src/lib.rs`
- `crates/tokamak-debugger/Cargo.toml`
- `crates/tokamak-debugger/src/lib.rs`

### 수정
- `Cargo.toml` (workspace members 추가)
- `crates/vm/levm/Cargo.toml` (tokamak feature)
- `crates/vm/Cargo.toml` (tokamak feature propagation)
- `cmd/ethrex/Cargo.toml` (tokamak feature propagation)

## 다음 단계

### Phase 1.1 완료를 위해 남은 작업

1. **빌드 검증 결과 기록** — `cargo build/test/clippy --workspace` 결과를 PHASE-1-1.md에 기록
2. **CI 워크플로우 생성** — `pr-tokamak.yaml` 작성 및 테스트
3. **커밋 + 푸시**

### Phase 1.2: Sync & Hive (Week 3-4)

4. 메인넷/Holesky 동기화 테스트
5. Hive 테스트 프레임워크 통합

### Phase 1.3: Benchmarking Foundation (Week 5-6)

6. `tokamak-bench` 구현 시작
7. `perf_opcode_timings` CI 연동

## 핵심 컨텍스트

- DECISION.md: **FINAL 확정** (2026-02-22)
- Volkov 점수: PROCEED 달성 (이전 세션, R6 이후 추가 리뷰에서 7.5 도달)
- 아키텍처 분석: `docs/tokamak/architecture/` 참조
- 격리 전략: Hybrid (feature flag + 신규 crate)
- Tier S 기능: JIT EVM + Continuous Benchmarking + Time-Travel Debugger
