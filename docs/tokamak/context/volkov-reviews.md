# Comrade Volkov's Review History

> "완벽은 존재하지 않는다. 다만 덜 불완전한 것이 있을 뿐이다."

## Score Progression

```
10.0 ┬─────────────────────────────────────
     │
 8.0 ┤ ·································· PROCEED (7.5+)
     │
 6.0 ┤ ·································· REVISE (6.0-7.4)
     │              ▲5.25
 5.0 ┤ ·············│·····▲4.5··········· REJECT (5.0-5.9)
     │              │     │    ▲4.0
 4.0 ┤ ·············│·····│····│·········· НЕЛЬЗЯ (<5.0)
     │  ▲3.0  ▲3.0 │     │    │
 3.0 ┤──│─────│────│─────│────│──────────
     │  │     │    │     │    │
 0.0 ┴──┴─────┴────┴─────┴────┴──────────
     R1    R2    R3    R4    R5
```

| Round | Subject | Score | Verdict | Trend |
|-------|---------|-------|---------|-------|
| R1 | L1 점유율 → L2 채택 전략 | 3.0 | НЕЛЬЗЯ | - |
| R2 | Hammer의 5,000줄 Python 클라이언트 | 3.0 | НЕЛЬЗЯ | → |
| R3 | Harvey + Jake 의견 추가 | 5.25 | REJECT | +2.25 |
| R4 | 팀 전체 토론 (8명) | 4.5 | НЕЛЬЗЯ | -0.75 |
| R5 | 40개 기능 아이디어 | 4.0 | НЕЛЬЗЯ | -0.5 |

**아직 한 번도 PROCEED(7.5+)를 받지 못했다.**

---

## Round 1: "L1 점유율 → L2 채택" 전략 (3.0/10)

### 제출자
Jason이 제기한 전략적 질문에 대한 초기 분석

### 핵심 감점 사유

**구조적 결함 (-4.0)**
- "L1 노드 운영자가 자연스럽게 L2 사용자가 될 것이다" — 미검증
  - Nethermind 18% 점유율이지만 관련 L2 없음
  - Besu(ConsenSys) 12%이지만 Linea는 기대 이하
  - **노드 운영 ≠ L2 채택**, 인과 메커니즘 부재
- 기존 클라이언트에 플러그인/모듈 통합, Restaking 생태계 결합 등 대안 비교 없음

**논리적 허점 (-3.0)**
- 10% 점유율 가정이 비현실적 — Besu가 수년 걸려 12%
- EtherX 0.4%를 벤치마크로 삼았으나, 0.4%는 성공이 아닌 시작 단계

**비즈니스 (-3.0)**
- 노드 운영자 전환 비용 분석 없음
- 경쟁사 비교 없음
- ROI 정량화 없음

### Volkov 코멘트
> "이 전략의 핵심 결함은 'If you build it, they will come'이라는
> Field of Dreams 사고방식이다. 진짜 질문은 'How to beat Geth'가
> 아니라 'Why beat Geth at all?'이다."

### 요구된 개선사항
1. L1 → L2 전환의 구체적 인센티브 메커니즘
2. 최소 3개 대안 전략의 비용-효과 비교
3. 리소스 현실성 분석 (Reth 팀 규모 대비)
4. 노드 운영자 전환 비용 분석
5. EXIT 기준 정의

---

## Round 2: Hammer의 초경량 연구 클라이언트 (3.0/10)

### 제출자
Jeongun Baek (Hammer)

### 제안 핵심
- 5,000줄 미만 Python 코드베이스
- AI 최적화 아키텍처 (Claude Code/Codex 즉시 파싱 가능)
- EIP 최빠 구현 → "사실상의 참조 구현"
- 자연어 EIP 구현 지원

### 핵심 감점 사유

**구조적 결함 (-4.0)**
- **5,000줄로 EL 클라이언트 불가능** — Geth 50만줄, Reth 20만줄
  - EVM + 상태관리 + P2P + JSON-RPC + TX pool + 동기화를 5,000줄로?
  - "장난감이지 프로덕션 노드가 아니다"
- 기존 연구 도구(py-evm, execution-specs, Foundry)와의 비교 없음
  - EF의 execution-specs가 이미 "참조 구현" 역할

**논리적 허점 (-3.0)**
- "사실상의 참조 구현이 됩니다" — 누가 그렇게 인정하는가?
- "거버넌스 영향력" — EIP 영향력은 코드 속도가 아닌 기술적 깊이에서 나옴
- AI 섹션이 제품 정체성을 흐림 — 노드인가 AI 코딩 도구인가?

**실행 미비 (-2.0)**
- "즉시 배포", "몇 분 만에 테스트" — 비현실적 시간 표현
- 구체적 인력/기간/예산 없음

### Volkov 코멘트
> "'5,000줄로 이더리움 노드를 만들겠다'는 것은 마치
> '자전거로 F1에 참가하겠다'는 것과 같다."

### 요구된 개선사항
1. **정체성 선택**: 연구 샌드박스 / 경량 프로덕션 노드 / AI 코딩 플랫폼 — 하나만
2. 기존 도구(Foundry, execution-specs) 대비 "왜 우리인가?" 3가지
3. 5,000줄로 가능한 EL 기능 범위 명시
4. L2 채택 연결고리 구체화

---

## Round 3: Harvey + Jake 의견 추가 (5.25/10)

### 제출자
Harvey & Jake Jang (Slack)

### Harvey 핵심 기여 (5.0/10)
- **운영 이력으로 신뢰 구축** (메트릭 공개, 업타임 투명성, 하드포크 대응력)
  - → **감점하지 않음**. R1/R2에서 빠져있던 핵심
- **경제적 정렬** — L2 수수료 일부를 노드 운영자에게 공유
  - → **감점하지 않음**. L1→L2 전환 인센티브의 첫 실질적 답변
- ethrex 기준 1TB/64GB RAM → 줄이겠다 → "어떻게?"가 없음 (-2.0)
- "AI로 줄일 수 있다" — AI는 합의 프로토콜 복잡성을 줄이지 못함 (-1.5)

### Jake 핵심 기여 (5.5/10)
- **zk-VM 호환 → Rust가 유일한 선택** — 기술적으로 정확
  - → **감점하지 않음**
- **Ooo의 ZK L2는 Yellow Paper 스펙 기반** — 클라이언트 종속성 제거
  - → **감점하지 않음**
- 이더리움 상태 크기(~250GB)는 언어와 무관한 물리적 한계 — "Rust로 바꾼다고 변하지 않는다" (-2.0)
- "새로운 가치를 수용" — 가장 중요한 제안이 가장 모호 (-1.5)

### Volkov 코멘트
> "두 사람 모두 방 안의 코끼리를 무시하고 있다:
> ethrex와 Reth가 이미 존재한다."

### 요구된 개선사항
1. **BUILD vs FORK vs CONTRIBUTE 의사결정** (ethrex 포크 / ethrex 기여 / Reth 포크 / 신규 개발)
2. 메모리 절감의 구체적 방법론 (Stateless client? Verkle? State pruning?)
3. "새로운 가치"를 3줄 이내로 정의
4. Reth 대비 이기는 차원 3가지

---

## Round 4: 팀 전체 토론 — "Occupy Ethereum" (4.5/10)

### 제출자
Kevin, Hammer, Harvey, Jake, Jason, Sahil, Suhyeon, Thomas (8명)

### R3 대비 점수 하락 이유 (-0.75)
**방향이 6개로 발산했기 때문.**
8명이 1시간+ 토론했으나 결론 없이 종료.

### 개인별 기여도

**Sahil — 토론 MVP**
1. "L1 share → L2 adoption 인과관계 부정" (데이터 기반)
   - Besu 10% → Linea 간신히 top 10
   - Reth 3% → Base #1 — L2가 Reth를 끌어올린 것 (역인과)
2. "AI/Python 네이티브 이더리움 클라이언트" — 유일한 빈 시장
   - EF의 EELS = Python, AI 에이전트 99% = Python
3. "Geth 버그 differential testing → responsible disclosure → ACD 진입"
   - Harvey의 "운영 이력 → 신뢰"보다 10배 효율적
   - **이 세 가지 모두 감점하지 않음**

**Jason — 핵심 질문**
- "L1 점유율이 L2 채택과 상관 있는가?" → 토론 방향을 바꿈

**Kevin — 열정은 있으나 전략 부재**
- "Occupy Ethereum", "boil the ocean" — 구호는 전략이 아님
- 전략 미합의 상태에서 py-ethclient 구현 시작 — 조급함
- etherX "L1 코드 90% 공유 → L2 플래그 배포" 모델 → 감점하지 않음

**Thomas — 유일한 시장 분석**
- L1 개선 → L2 가치 하락 트렌드 분석
- Agent 전용 L2 제안 — 하지만 EL 클라이언트와 연결 느슨

**Suhyeon — 흥미롭지만 접선**
- 지리적 탈중앙화 인센티브 (FC26)
- 현재 전략과 직접 연결 안 됨

### 핵심 구조적 문제
1. **핵심 전제(L1→L2)가 반박되었으나 해결 안 됨** — Jason/Sahil이 흔들었는데 팀이 넘어감
2. **6개 방향 발산**: Python 연구 / Rust zk-VM / 운영 신뢰 / AI Python 플랫폼 / etherX L2 / Agent L2
3. **퍼실리테이션 실패** — 결론 없이 종료

### Volkov 코멘트
> "8명의 선수가 6개 종목에 출전 신청을 했는데,
> 어떤 종목에 집중할지 결정하지 않았다.
> Sahil이 유일하게 빈 레인을 찾았다:
> 'AI 네이티브 Python 이더리움 클라이언트.'
> 나침반 없이 노를 젓는 것은 항해가 아니라 표류다."

### 요구된 의사결정 (4개)
| Q | Question | 답변 상태 |
|---|----------|-----------|
| Q1 | 프로덕션 노드 vs 연구/개발 도구? | **미결정** |
| Q2 | Python vs Rust? | **미결정** (Kevin이 Python으로 선행) |
| Q3 | L2 채택 목표 vs 노드 점유율 자체? | **미결정** |
| Q4 | 6개월 측정 가능 성공 기준? | **미정의** |

---

## Round 5: 40개 기능 아이디어 (4.0/10)

### 제출자
(아이디어 리스트 제공자)

### 분류 결과

| Category | Count | % |
|----------|-------|---|
| 즉시 퇴장 (SF/황당) | 12 | 30% |
| 관할 밖 (EL 클라이언트 아님) | 11 | 28% |
| 비현실적 (시기상조) | 6 | 15% |
| **개발 가치 있음** | **11** | **28%** |

### 즉시 퇴장 12개
#33 Emotional Intelligence, #36 Space Node, #37 DNA Storage,
#38 Teleportation, #39 Self-Evolving, #40 Consciousness,
#31 AR/VR, #32 Voice-Activated, #35 Biological Data,
#16 Carbon-Negative, #28 Energy-Aware, #30 Biometric
> "이것들을 제출한 판단력 자체가 문제다"

### 관할 밖 11개
#4 Cross-Chain, #13 Prediction Market, #18 Social Recovery,
#22 Federated Learning, #23 HSM Cloud, #24 Semantic Search,
#25 Autonomous Agent, #26 Streaming Payments, #27 DID,
#29 Collaborative Filtering, #20 ML Execution
> "다른 대회에 출전하라"

### 비현실적 6개
#1 ZK Prover (경량과 모순), #2 Quantum-Resistant (시기상조),
#3 Decentralized Sequencer (별도 프로젝트), #8 WASM-First (EVM 호환성 포기),
#11 Privacy/FHE (3-5년 후), #34 Quantum Randomness (장비 요구)

### Tier S — 즉시 착수 가치 (PROCEED)

| # | Feature | Score | Key Reason |
|---|---------|-------|------------|
| **#21** | **Time-Travel Debugger** | **7.5** | 로컬 내장 time-travel은 없다. 연구 도구 정체성과 완벽 일치 |
| **#10** | **Continuous Benchmarking** | **7.5** | Sahil의 differential testing 전략 자동화. 다른 모든 전략의 기반 |
| **#9** | **JIT-Compiled EVM** | **7.0** | 유일한 측정 가능 성능 우위. **Rust only** |

### Tier A — 고려 가치 (REVISE)
#15 Formal Verification (6.5), #5 Event Streaming (6.5),
#6 Deterministic Performance (6.0), #7 Edge/Ultra-Light (6.0),
#12 Self-Healing (6.0), #17 DeFi/MEV Protection (6.0),
#19 Data Availability Sampling (6.0)

### Volkov 추천 조합

**Python 선택 시:**
```
#21 Time-Travel Debugger + #10 Continuous Benchmarking + #5 Event Streaming
= "연구자/AI 에이전트를 위한 디버깅·분석 도구 내장 Python EL"
```

**Rust 선택 시:**
```
#9 JIT EVM + #10 Continuous Benchmarking + #21 Time-Travel Debugger
= "EVM 성능에서 Geth/Reth를 능가하는 ZK-native Rust EL"
```

### Volkov 코멘트
> "'이미 연습한 종목에 출전하라.'
> Tokamak이 이미 가진 것: Python 경험, ZK 회로 전문성, AI 도구 활용.
> 일치하는 3개를 골라 6개월 안에 프로토타입을 만들어라.
> 나머지 37개는 Забудь (잊어라)."

---

## Cross-Review Patterns (5회 심판을 관통하는 패턴)

### 반복적으로 감점된 항목 (해결 안 됨)

| Issue | R1 | R2 | R3 | R4 | R5 |
|-------|:--:|:--:|:--:|:--:|:--:|
| L1→L2 인과관계 미검증 | -2.0 | - | - | -2.0 | - |
| 정체성 혼란 (노드? 도구? 플랫폼?) | - | -2.0 | - | -2.0 | - |
| Python vs Rust 미결정 | - | - | -1.0 | -1.0 | -1.0 |
| 구체적 수치/메트릭 부재 | -1.0 | -1.0 | -1.0 | -1.0 | - |
| 경쟁 분석 부족 | -1.0 | -1.0 | -1.0 | -0.5 | - |
| 과대 포장 표현 | -0.5 | -0.5 | -0.5 | -0.5 | - |

### 감점되지 않은 항목 (구출 가능)

| Idea | Who | Round | Status |
|------|-----|-------|--------|
| 운영 이력으로 신뢰 구축 | Harvey | R3 | Tier S #10으로 자동화 |
| 수수료 공유 인센티브 | Harvey | R3 | 경제 모델 구체화 필요 |
| Rust + zk-VM 호환 | Jake | R3 | Q2 결정에 의존 |
| Yellow Paper 스펙 기반 L2 | Jake | R3 | 클라이언트 독립성 확보 |
| L1→L2 인과관계 부정 (데이터) | Sahil | R4 | **팀이 수용해야 함** |
| AI/Python 네이티브 포지셔닝 | Sahil | R4 | 유일한 빈 시장 |
| Differential testing → ACD | Sahil | R4 | Tier S #10에 통합 |
| etherX "L1→L2 플래그" 모델 | Kevin | R4 | 아키텍처 참조 가능 |
| Time-Travel Debugger | - | R5 | **Tier S — 즉시 착수** |
| Continuous Benchmarking | - | R5 | **Tier S — 즉시 착수** |
| JIT-Compiled EVM | - | R5 | **Tier S — Rust only** |

---

## PROCEED(7.5+)를 받기 위한 최소 조건

```
┌─────────────────────────────────────────────┐
│  다음 제출에서 PROCEED를 받으려면:          │
│                                             │
│  1. Q1-Q4 의사결정 완료 (숫자 포함)         │
│  2. 선택한 방향의 6개월 로드맵              │
│  3. 구체적 인력/예산 배분                   │
│  4. 경쟁사 대비 차별점 3가지 (데이터 기반)  │
│  5. EXIT 기준 (어떤 수치 미달 시 포기?)     │
│  6. Tier S 기능 중 1개의 2주 PoC 결과       │
│                                             │
│  이 6개가 모두 충족되면 7.5를 고려하겠다.   │
│  "고려"이지 "보장"이 아니다.                │
│  Посмотрим.                                 │
└─────────────────────────────────────────────┘
```
