# Phase Management

현재 Phase 상태를 확인하고, 다음 Phase 진입 조건을 검증한다.

## Phase 정의

| Phase | 내용 | 기간 | 진입 조건 |
|-------|------|------|-----------|
| 1.1 | Fork & 환경 구축 | Week 1-2 | DECISION.md FINAL |
| 1.2 | 메인넷 동기화 + Hive | Week 3-6 | Phase 1.1 완료, 빌드 성공 |
| 1.3 | Continuous Benchmarking MVP | Week 7-10 | 메인넷 싱크 완료, Hive 95%+ |
| 2 | Time-Travel Debugger | Month 3-4 | Phase 1.3 완료 |
| 3 | JIT EVM | Month 5-7 | Phase 2 완료, diff-test PASS |
| 4 | Tokamak L2 통합 | Month 8-10 | Phase 3 완료 |

## 실행 순서

1. `docs/tokamak/scaffold/HANDOFF.md` 읽어서 현재 Phase 파악
2. 현재 Phase의 완료 조건 체크:
   - Phase 1.1: `cargo build --workspace` 성공 + CI 파이프라인 존재
   - Phase 1.2: 메인넷 싱크 로그 + Hive 통과율 95%+
   - Phase 1.3: 벤치마크 러너 동작 + Geth 대비 비교 데이터
   - Phase 2: `debug_timeTravel` RPC 구현 + 테스트
   - Phase 3: JIT Tier 0+1 + EF 테스트 100% + `/diff-test` PASS
   - Phase 4: `--tokamak-l2` 플래그 동작 + L2 Hook 테스트
3. 다음 Phase 진입 조건 충족 여부 판정
4. HANDOFF.md 업데이트

## EXIT 기준 체크 (Phase와 무관하게 항상 확인)

| 수치 | 기한 | 현재 상태 |
|------|------|-----------|
| 메인넷 풀 싱크 | 4개월 | {확인} |
| Hive 95%+ | 6개월 | {확인} |
| 30일 업타임 | 6개월 | {확인} |

## 보고 형식

```
[PHASE] Current: {N.N} — {상태}
- completion: {X/Y criteria met}
- next phase ready: {yes|no}
- EXIT criteria: {all clear | WARNING: ...}
- blockers: {none | list}
```
