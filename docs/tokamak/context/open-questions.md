# Open Questions

## Resolution: Dual-Track Strategy

> "점유율을 확보하려면 프로덕션 노드여야 한다" — Jason
> "Python은 병행할 수 있다" — Jason

Q1, Q2는 **"둘 다"가 답이다.** 단, 별개 트랙으로.

```
┌─ Track A: Rust Production Node ─────────────────┐
│ Goal: 노드 점유율 확보 ("Occupy Ethereum")       │
│ Language: Rust                                   │
│ Base: ethrex fork (가장 현실적)                  │
│ Target: 메인넷 합의 참여, nodewatch.io 집계      │
│ L2: --tokamak-l2 플래그로 L2 native 통합        │
│ Team: Rust 엔지니어 중심                         │
│ Tier S: JIT EVM, Continuous Benchmarking,        │
│         Time-Travel Debugger                     │
└──────────────────────────────────────────────────┘

┌─ Track B: Python Research/AI Client ─────────────┐
│ Goal: 개발자/AI 에이전트 생태계 확보             │
│ Language: Python                                 │
│ Base: EELS/py-evm fork 또는 Kevin의 py-ethclient│
│ Target: 연구자, AI 에이전트, Python 개발자       │
│ L2: 플러그인 모듈                                │
│ Team: Kevin + Python 엔지니어                    │
│ Tier S: Time-Travel Debugger, Continuous         │
│         Benchmarking, Event Streaming            │
└──────────────────────────────────────────────────┘
```

---

## Resolved Questions

### Q1: Product Identity ✅ RESOLVED
**답: 둘 다 — 별개 트랙으로 병행**
- Track A = 프로덕션 노드 (점유율)
- Track B = 연구/개발 도구 (생태계)

### Q2: Language ✅ RESOLVED
**답: 둘 다 — 트랙별로 분리**
- Track A = Rust (메인넷 성능 필수)
- Track B = Python (AI/연구 생태계)

---

## Remaining Questions

### Q3: Primary Goal — 트랙별 명확화 필요
**Track A의 노드 점유율 목표치는?**

| Timeline | Target | Meaning |
|----------|--------|---------|
| 6 months | 메인넷 싱크 성공 | 0% → 존재 증명 |
| 12 months | 10-50 노드 | <1% — 신뢰 구축 단계 |
| 24 months | 200-500 노드 | 2-5% — EF 인정 수준 |
| 36 months | 500-1000 노드 | 5-10% — 의미 있는 점유율 |

**Track B의 성공 기준은?**
- GitHub Stars? 다운로드 수? 연구 논문 인용?

### Q4: 6-Month Success Criteria ⚠️ NEEDS DEFINITION

**Track A (Rust Production):**
- [ ] ethrex 포크 후 메인넷 풀 싱크 완료
- [ ] Ethereum Hive 테스트 95%+ 통과
- [ ] Tokamak L2 모드 PoC (`--tokamak-l2`)
- [ ] 내부 노드 3개 이상 안정 운영 (30일+ 업타임)
- [ ] Geth 대비 벤치마크 대시보드 공개
- [ ] Differential testing에서 Geth/Reth 불일치 1건+ 발견

**Track B (Python Research):**
- [ ] 메인넷 트랜잭션 Time-Travel 리플레이 작동
- [ ] AI 에이전트 통합 예제 3개+
- [ ] GitHub Stars 500+
- [ ] 이더리움 연구자 피드백 20건+
- [ ] EF 클라이언트 다양성 그랜트 신청

### Q5: Track A — Build vs Fork ⚠️ CRITICAL

점유율이 목표이므로 속도가 중요. **포크가 가장 현실적:**

| Option | Time to Mainnet Sync | Effort | Risk |
|--------|---------------------|--------|------|
| **ethrex fork** | **3-6 months** | **Medium** | **Medium** |
| Reth fork | 3-6 months | High (복잡) | High (Paradigm 관계) |
| New from scratch | 18-24 months | Very High | Very High |

**ethrex fork 추천 이유:**
- LambdaClass도 L2 통합을 목표로 하고 있어 아키텍처 방향 일치
- Reth보다 코드베이스가 작아 이해/수정 용이
- Apache 2.0 라이선스 — 포크 자유

**결정 필요:** ethrex 팀과 협력(contribute)할 것인가, 독립 포크할 것인가?

### Q6: Team Allocation ⚠️ CRITICAL

현재 동시 진행 중인 프로젝트:
- ZK MIPS 회로 (활발)
- ETH-RPG (활발)
- Delegate Staking MVP (활발)
- + Track A (Rust EL client) — NEW
- + Track B (Python client) — NEW (Kevin 진행 중)

**Track A에 필요한 최소 인력:**
- Senior Rust 엔지니어 2명 (ethrex fork + L2 통합)
- 1명은 JIT EVM 가능한 컴파일러 경험자

**질문:**
- 기존 프로젝트에서 인력을 재배치하는가?
- 신규 채용이 필요한가?
- ZK 회로 팀의 Rust 경험을 활용할 수 있는가?

### Q7: EF Grant Strategy

**두 트랙 모두 EF 그랜트 대상이 될 수 있다:**
- Track A: 클라이언트 다양성 그랜트 (Geth 슈퍼다수 해소)
- Track B: 개발자 도구 / 연구 인프라 그랜트

**신청 타이밍:**
- Track A: 메인넷 싱크 성공 후 (없으면 신뢰성 부족)
- Track B: Time-Travel Debugger MVP 후 (데모 가능해야)

### Q8: Differential Testing → ACD 진입 전략

**Track A와 B 모두에 적용 가능한 신뢰 구축 경로:**

```
1. 두 트랙 모두에서 Continuous Benchmarking (#10) 실행
2. Geth/Reth와 동일 트랜잭션 실행, 결과 비교
3. 불일치 발견 시 → 원인 분석
4. Geth/Reth 버그 확인 → responsible disclosure
5. All Core Devs 미팅 초대 획득
6. 이더리움 커뮤니티 내 Tokamak 신뢰도 상승
```

이것은 Track 선택과 무관하게 즉시 시작 가능.
ethrex나 py-evm을 로컬에서 돌리면서 Geth와 differential testing만 해도 된다.

---

## Decision Timeline (Updated)

| Week | Decision | Owner | Track |
|------|----------|-------|-------|
| W1 | ethrex fork vs contribute 결정 | Tech leads | A |
| W1 | py-ethclient 방향 확인 (EELS 기반?) | Kevin | B |
| W2 | Track A 인력 배정 | Kevin | A+B |
| W2 | 6개월 KPI 확정 (위 후보 기반) | Full team | Both |
| W3 | ethrex fork 시작 / py-ethclient 계속 | Engineers | Both |
| W3 | Continuous Benchmarking 인프라 구축 | 1 engineer | Both |
| W4 | 첫 메인넷 싱크 시도 (Track A) | Rust team | A |
| W4 | Time-Travel Debugger MVP (Track B) | Python team | B |
