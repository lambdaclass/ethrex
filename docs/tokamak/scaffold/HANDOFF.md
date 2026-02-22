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

## Phase 1.2 완료 요약

### Feature Flag 분할

`tokamak` → 3 독립 feature + 1 umbrella:

| Feature | 용도 |
|---------|------|
| `tokamak-jit` | JIT 컴파일 계층 |
| `tokamak-debugger` | 타임트래블 디버거 |
| `tokamak-l2` | Tokamak L2 훅 |
| `tokamak` | 위 3개 모두 활성화 (umbrella) |

전파 경로: `cmd/ethrex → ethrex-vm → ethrex-levm`

### CI Infrastructure

- **pr-tokamak.yaml**: quality-gate (4 feature check + test + clippy) + format-check
- **snapsync-run action**: 이미지 기본값 `ghcr.io/tokamak-network/ethrex`로 변경
- Hive client config, Assertoor, Dockerfile: 이미 fork-safe (변경 불필요)

### 변경 파일

| 파일 | 변경 내용 |
|------|-----------|
| `crates/vm/levm/Cargo.toml` | tokamak → tokamak-jit/debugger/l2 + umbrella |
| `crates/vm/Cargo.toml` | tokamak-jit/debugger/l2 전파 추가 |
| `cmd/ethrex/Cargo.toml` | tokamak-jit/debugger/l2 전파 추가 |
| `.github/workflows/pr-tokamak.yaml` | 신규 생성 |
| `.github/actions/snapsync-run/action.yml` | 이미지 기본값 변경 |
| `docs/tokamak/architecture/PHASE-1-2.md` | 신규 생성 |

## Git 상태

- 브랜치: `feat/tokamak-proven-execution`
- 리모트: `origin` (tokamak-network/ethrex)

## 커밋 이력

| 커밋 | 내용 |
|------|------|
| `864ac9e2c` | docs: mark Phase 1.1 complete, update HANDOFF |
| `42ebbe926` | docs: fix architecture docs per Volkov R8-R10 review |
| `c1e4f988b` | docs: add ethrex architecture analysis and Phase 1.1 infrastructure |
| `36f9bf7a8` | docs: finalize DECISION.md with agent model |
| `52fa4bc77` | docs: update HANDOFF.md with session progress |

## 다음 단계

### Phase 1.2 나머지

1. **빌드 검증** — `cargo check --features tokamak-{jit,debugger,l2}` + `cargo test --workspace`
2. **CI 검증** — Push하여 `pr-tokamak.yaml` 트리거 확인
3. **Sync 검증** — Hoodi snapsync 완료 확인 (CI runner 필요)
4. **Hive 검증** — PR Hive 6 suite + Assertoor 2 suite baseline 기록

### Phase 1.3: Benchmarking Foundation

5. `tokamak-bench` 구현 시작
6. `perf_opcode_timings` CI 연동

## 핵심 컨텍스트

- DECISION.md: **FINAL 확정** (2026-02-22)
- Volkov 점수: DECISION R6 PROCEED(7.5) → Architecture R10 PROCEED(8.25)
- 아키텍처 분석: `docs/tokamak/architecture/` 참조
- 격리 전략: Hybrid (feature flag ~30줄 + 신규 crate 3개)
- Feature flag 분할: tokamak → tokamak-jit/debugger/l2 (완료)
- Codebase: ~103K lines Rust, 28 workspace crates, 29+ CI workflows
- Test baseline: 718 passed, 0 failed
