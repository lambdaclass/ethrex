# Handoff: Tokamak Ethereum Client

## 현재 작업 상태

| 항목 | 상태 |
|------|------|
| Phase 0-4: 개발 환경 구축 (monorepo) | **완료** |
| Phase 0-1: ethrex 코드베이스 분석 | **완료** |
| Phase 0-2: 대안 평가 (Reth 등) | **완료** |
| Phase 0-3: DECISION.md 작성 | **완료** |
| Phase 0-3a: Volkov R6 리뷰 + 수정 | **완료** |

## 이번 세션에서 수행한 작업

### 1. DECISION.md 초안 작성 (커밋 `ca65752`)

14개 문서를 `docs/tokamak/` 하위에 작성하고 커밋/푸시:
- `DECISION.md` — ethrex fork 결정 문서 (초안)
- `vision.md`, `context/`, `features/`, `scaffold/` 등

### 2. Volkov R6 리뷰 수행 → 6.5/10 (REVISE)

Volkov가 지적한 3가지 필수 수정사항:
1. **결정 매트릭스 편향** — 허수아비 옵션, Reth 과소평가
2. **EXIT 기준 부재** — "재평가"는 행동이 아님
3. **Tier S PoC 미실행** — 계획이 아니라 결과 필요

### 3. 필수 수정사항 3건 반영 (커밋 `adbfeca`)

**Fix 1: 매트릭스 보정**
- "처음부터 구축"/"revm 단독"을 부록으로 이동
- ethrex vs Reth 이원 비교로 재구성
- Reth ZK: 1→2 (Zeth 존재 반영, 단 별도 프로젝트/RISC Zero 단일 프루버)
- Reth 관리성: 2→3 (모듈러 아키텍처/Paradigm 투자 인정)
- ethrex 동기화: 5→4 (<1% 점유율, 실전 검증 적음)
- ExEx가 post-execution hook이며 EVM 수정 메커니즘이 아님을 명시
- 최종: ethrex 4.60 vs Reth 2.80

**Fix 2: EXIT 기준 4요소 완성**
| 수치 | 기한 | 미달 시 행동 | 의사결정자 |
|------|------|-------------|-----------|
| 메인넷 싱크 | 4개월 | 버그 리포트 + 재시도 → 실패 시 Reth 전환 평가 | Tech leads |
| Hive 95%+ | 6개월 | upstream 기여 시도. 80% 미만이면 중단 검토 | Tech leads + Kevin |
| 30일 업타임 | 6개월 | 아키텍처 재검토 | Full team |
| Rust 2명 확보 | 3개월 | Phase 축소 (JIT 제외) | Kevin |

**Fix 3: Tier S PoC 실행**
- `cargo build --features perf_opcode_timings` 빌드 성공 (3m 44s)
- 코드 경로 분석 완료 (vm.rs → Instant::now() → elapsed() → timings.update())
- PoC 결론: feature flag 동작 확인, CI 연결 경로 문서화

### 4. 코드 리뷰 통과 (9.0/10)
- REJECT 1건: Reth 가중 합계 산술 오류 (2.85→2.80) → 수정 완료

## Volkov R6 점수 추이

```
R1: 3.0 → R2: 3.0 → R3: 5.25 → R4: 4.5 → R5: 4.0 → R6: 6.5 (REVISE)
```

PROCEED(7.5)까지 1.0 남음. 미충족: #3 인력 배분 (부분).

## Phase 0-2 결정 매트릭스 (보정 후)

| 기준 (가중치) | ethrex | Reth |
|--------------|--------|------|
| 메인넷 동기화 (25%) | 4 | 4 |
| EVM 수정 가능성 (25%) | 5 | 2 |
| ZK 호환성 (20%) | 5 | 2 |
| 코드베이스 관리성 (15%) | 4 | 3 |
| L2 아키텍처 정합성 (15%) | 5 | 3 |
| **가중 합계** | **4.60** | **2.80** |

**결정: ethrex fork** — `docs/tokamak/DECISION.md` 참조

## Git 상태

- 브랜치: `feat/tokamak-proven-execution`
- 리모트: `origin` (tokamak-network/ethrex) — 푸시 완료
- 마지막 커밋: `adbfeca` — Volkov R6 피드백 반영

```
adbfeca docs: revise DECISION.md per Volkov R6 review feedback
ca65752 docs: add Tokamak EL client decision and planning documents
```

## 다음 단계

### 즉시 필요

1. **DECISION.md 팀 리뷰** — DRAFT 상태. 팀 확인 후 확정
2. **인력 배분 확정** — Senior Rust 2명 + JIT 경험자 1명 (Volkov 유일한 부분 충족 항목)
3. **LambdaClass 커뮤니케이션** — Fork 전 협력적 fork 의향 확인 (Volkov 권장사항)

### Phase 1.1: Fork & 환경 구축 (Week 1-2)

4. ethrex fork 기반으로 빌드 검증 (메인넷/Holesky)
5. CI 파이프라인 설정
6. Hive 테스트 프레임워크 통합 시작

### Volkov 권장사항 (점수 상승에 기여)

- 인력 계획 현실화: Phase별 인력 집중 계획 수립
- JIT 기술적 장벽 심화 분석: revmc 선행 사례, validation mode 성능 오버헤드
- LambdaClass 관계 전략

## 핵심 컨텍스트

- 개발 계획 전문: `docs/tokamak/` 내 문서들
  - `vision.md` — 전체 비전 ("Performance you can see, verify, and debug")
  - `DECISION.md` — ethrex fork 결정 문서 (Volkov R6 피드백 반영, DRAFT)
  - `context/competitive-landscape.md` — 경쟁 분석
  - `context/volkov-reviews.md` — R1-R5 리뷰 이력
  - `features/01~03-*.md` — Tier S 기능 상세
- 포지셔닝: "Performance you can see, verify, and debug"
- Tier S 기능 3개: JIT EVM + Continuous Benchmarking + Time-Travel Debugger
- Base client: **ethrex fork 확정** (DECISION.md)

## Reth 조사 결과 (이번 세션)

- **Zeth** (risc0/zeth, 439 stars): RISC Zero가 관리하는 별도 프로젝트. Reth의 stateless execution을 zkVM 내에서 사용. RISC Zero 프루버만 지원. Reth에 내장된 것 아님
- **ExEx** (Execution Extensions): 블록 실행 후 상태 변경을 수신하는 post-execution hook. EVM 실행 자체를 수정하는 메커니즘이 아님. 롤업/브릿지/인덱서용
- **결론**: Reth ZK 1→2 상향 조정은 공정하나, ethrex의 네이티브 4-프루버 통합과는 깊이가 다름
