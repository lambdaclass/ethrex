# Combined Vision: Tokamak Ethereum Client

> **"Performance you can see, verify, and debug."**
>
> 이더리움에서 가장 빠르고, 스스로 그것을 증명하며, 왜 빠른지 보여주는 클라이언트.

## Language: Rust

JIT 컴파일러(Cranelift), 프로덕션 메인넷 성능, 프로덕션 노드 내장이 모두
Rust를 요구한다. 이 비전은 Track A(Rust Production Node)에 속한다.

## Core Identity

기존 클라이언트는 "우리가 빠르다"고 **주장**한다.
Tokamak 클라이언트는 매 커밋마다 자동으로 **증명**하고,
차이가 나면 **왜 다른지 보여준다.**

```
┌──────────────────────────────────────────────────────┐
│                                                      │
│   #9  JIT-Compiled EVM    → 가장 빠르고             │
│   #10 Continuous Benchmark → 스스로 증명하며         │
│   #21 Time-Travel Debugger → 왜 빠른지 보여준다     │
│                                                      │
└──────────────────────────────────────────────────────┘
```

## The Self-Reinforcing Loop

세 기능이 따로 노는 것이 아니라 하나의 피드백 루프를 형성한다:

```
        ┌─────────────────────────┐
        │   #9 JIT-Compiled EVM   │
        │                         │
        │   EVM 바이트코드를      │
        │   네이티브 코드로       │
        │   JIT 컴파일            │
        │   → Geth 대비 3-5x     │
        └───────────┬─────────────┘
                    │
       "얼마나 빠른가?"
                    │
                    ▼
        ┌─────────────────────────┐
        │   #10 Continuous        │
        │   Benchmarking          │
        │                         │
        │   매 커밋마다 자동      │
        │   Geth/Reth 대비       │
        │   성능 측정 + 공개      │
        │                         │
        │   + differential        │
        │     testing으로         │
        │     Geth 버그 발견      │
        └───────────┬─────────────┘
                    │
       "왜 다른 결과가 나왔는가?"
                    │
                    ▼
        ┌─────────────────────────┐
        │   #21 Time-Travel       │
        │   Debugger              │
        │                         │
        │   불일치 트랜잭션을     │
        │   opcode 단위로         │
        │   리플레이하며          │
        │   정확한 원인 추적      │
        └───────────┬─────────────┘
                    │
       "이 결과로 JIT를 더 개선"
                    │
                    ▼
              ┌───────────┐
              │ #9로 복귀 │
              └───────────┘
```

### Loop in Action: 구체적 예시

**JIT 최적화 루프:**
```
1. Benchmarking: "Aave liquidation이 Reth보다 느림"
2. Time-Travel: 해당 tx 리플레이 → JUMPDEST 패턴에서 병목 발견
3. JIT: 해당 opcode 패턴에 JIT 최적화 추가
4. Benchmarking: 다음 커밋에서 자동 확인 → 2.8x → 3.4x 개선
5. (반복)
```

**Geth 버그 발견 루프:**
```
1. Benchmarking: "블록 #19,847,231에서 Geth와 state root 불일치"
2. Time-Travel: 해당 tx를 opcode 단위로 리플레이
              → SSTORE에서 Geth의 gas 계산 오류 확인
3. Responsible disclosure → Geth 팀 보고
4. All Core Devs 미팅 초대 → 이더리움 커뮤니티 신뢰 확보
5. "Tokamak이 이더리움 보안에 기여하는 팀" 포지셔닝
```

## Competitive Positioning

| Capability | Geth | Reth | Nethermind | **Tokamak** |
|-----------|:----:|:----:|:---------:|:-----------:|
| EVM Performance | Baseline | 1.5-2x | ~1x | **3-5x (JIT)** |
| Auto Benchmark | No | No | No | **Every commit** |
| Public Dashboard | No | No | No | **clients.tokamak.network** |
| Differential Testing | No | No | No | **Built-in** |
| Time-Travel Debug | Raw trace | Raw trace | Raw trace | **Interactive** |
| Proves its own speed | No | No | No | **Yes** |

**핵심 차별점: 어떤 이더리움 클라이언트도 이 세 가지를 조합하지 않았다.**

## Usage Scenarios

### Scenario 1: 노드 운영자 설득
```
운영자: "왜 Geth 대신 이걸 써야 하죠?"
우리:    clients.tokamak.network 접속하세요.
         매일 자동 업데이트되는 Geth/Reth 대비 벤치마크입니다.
         Uniswap swap 3.2x, ERC-20 transfer 2.1x 빠릅니다.
         수치를 직접 확인하세요.
```

### Scenario 2: Geth 버그 발견 → 커뮤니티 신뢰 구축
```
Benchmarking: "블록 #19,847,231에서 Geth와 결과 불일치"
Time-Travel:  해당 tx opcode 단위 리플레이
              → SSTORE에서 Geth gas 계산 오류 확인
Disclosure:   Geth 팀에 responsible disclosure
Result:       ACD 미팅 초대 → 커뮤니티 신뢰 확보
```

### Scenario 3: EF 그랜트 신청
```
신청서:  "우리는 클라이언트 다양성에 기여합니다.
         우리 클라이언트는 Geth보다 3x 빠르며,
         이를 자동 벤치마크로 투명하게 증명합니다.
         이미 differential testing으로 Geth 버그 N건을
         발견하여 이더리움 보안에 기여했습니다."
```

### Scenario 4: L2 확장 (Phase 2)
```
기반 확보 후:
  tokamak-node --tokamak-l2

→ 동일 바이너리로 L1 노드 + Tokamak L2 동시 운영
→ etherX 모델: L1 코드 90% 공유, 플래그 하나로 L2
→ 노드 운영자에게 L2 수수료 일부 공유 (Harvey 제안)
→ "이미 돌리고 있는 노드에 플래그 하나 추가"
```

## Technical Architecture

### Base: ethrex Fork (Rust)
```
ethrex (LambdaClass)
├── EVM execution     ← #9 JIT 컴파일러 교체
├── P2P networking    ← 그대로 사용
├── State management  ← 그대로 사용
├── JSON-RPC          ← #21 Time-Travel RPC 추가
├── Consensus         ← 그대로 사용
└── Sync              ← 그대로 사용

추가 모듈:
├── jit/              ← Cranelift 기반 JIT 컴파일러
│   ├── compiler.rs       바이트코드 → 네이티브 코드
│   ├── cache.rs          컴파일된 코드 캐시 (LRU)
│   ├── optimizer.rs      opcode fusion, constant folding
│   └── profiler.rs       실행 빈도 카운터 (tiered)
│
├── benchmark/        ← Continuous Benchmarking
│   ├── runner.rs         벤치마크 시나리오 실행
│   ├── comparator.rs     Geth/Reth 대비 비교
│   ├── differential.rs   state root 불일치 감지
│   └── publisher.rs      결과 → dashboard 발행
│
├── debugger/         ← Time-Travel Debugger
│   ├── replay.rs         트랜잭션 상태 재구성
│   ├── snapshot.rs       opcode별 state snapshot (CoW)
│   ├── inspector.rs      revm Inspector trait 구현
│   └── rpc.rs            debug_timeTravel RPC endpoint
│
└── tokamak-l2/       ← Phase 2: L2 통합
    ├── bridge.rs
    ├── prover.rs
    └── sequencer.rs
```

### JIT Tiered Execution
```
모든 EVM 바이트코드
    │
    ├─ 실행 횟수 < 10회 ──→ Tier 0: Interpreter (revm)
    │                       빠른 시작, 오버헤드 없음
    │
    ├─ 실행 횟수 10-100회 → Tier 1: Baseline JIT (Cranelift)
    │                       빠른 컴파일, 기본 최적화
    │
    └─ 실행 횟수 > 100회 ─→ Tier 2: Optimizing JIT
                            프로파일 기반 최적화
                            opcode fusion, register alloc
```

핵심: Uniswap Router처럼 초당 수천 회 호출되는 컨트랙트에서 최대 효과.

### Benchmark CI Pipeline
```yaml
# 매 커밋마다
on: [push, pull_request]

jobs:
  benchmark:
    steps:
      - run: tokamak-bench --compare geth:latest,reth:latest
      - run: tokamak-bench --differential  # state root 비교
      - run: tokamak-bench --regression-check --threshold 5%
      - run: tokamak-bench --publish  # → clients.tokamak.network
```

### Time-Travel RPC Extension
```
기존 RPC:
  debug_traceTransaction(txHash)  → raw opcode trace

Tokamak 추가:
  debug_timeTravel(txHash, {
    stepForward: true,
    stepBack: true,
    breakpoints: ["SSTORE", "CALL"],
    inspectSlot: "0x..."
  })
  → interactive state at each opcode step
```

## Implementation Roadmap

### Phase 1: Foundation (Month 1-2)
```
Week 1-2: ethrex fork + 빌드 환경 구축
Week 3-4: 메인넷 풀 싱크 확인
Week 5-6: Continuous Benchmarking MVP
          (Geth/Reth 대비 자동 비교)
Week 7-8: Differential testing 통합
          (state root 불일치 감지)
```
Deliverable: 메인넷 싱크 + 자동 벤치마크 대시보드

### Phase 2: Debugging (Month 3-4)
```
Week 9-10:  Time-Travel Debugger core
            (tx replay + state snapshot)
Week 11-12: Interactive CLI debugger
            (step, breakpoint, inspect)
Week 13-14: debug_timeTravel RPC endpoint
Week 15-16: Web UI (optional)
```
Deliverable: 로컬에서 과거 트랜잭션 인터랙티브 디버깅

### Phase 3: Performance (Month 5-7)
```
Week 17-18: JIT Tier 0+1 (Cranelift baseline)
Week 19-20: Ethereum test suite 100% 통과 검증
Week 21-22: JIT Tier 2 (opcode fusion, optimization)
Week 23-24: Fuzzing + security audit
Week 25-28: 성능 튜닝 + 벤치마크 공개
```
Deliverable: Geth 대비 2-3x+ 성능, 자동 증명 대시보드

### Phase 4: L2 Integration (Month 8-10)
```
Week 29-32: --tokamak-l2 플래그
Week 33-36: 브릿지, 증명 검증, 시퀀서
Week 37-40: L2 수수료 공유 메커니즘
```
Deliverable: 동일 바이너리로 L1 + Tokamak L2 운영

## Resource Requirements

| Phase | Duration | Rust Engineers | Other |
|-------|----------|---------------|-------|
| 1. Foundation | 2 months | 2 | 1 DevOps |
| 2. Debugging | 2 months | 2 | 1 Frontend (UI) |
| 3. Performance | 3 months | 2-3 (JIT = compiler exp.) | 1 Security |
| 4. L2 Integration | 3 months | 2 | 1 ZK (from existing team) |

**최소 인력: Senior Rust 2명 + JIT/컴파일러 경험자 1명**

ZK 회로 팀의 Rust 경험을 Phase 4에서 활용 가능.

## Risk Matrix

| Risk | Impact | Probability | Mitigation |
|------|--------|-------------|------------|
| JIT consensus 위반 | Critical | Medium | 모든 JIT 결과를 interpreter와 비교하는 validation mode |
| ethrex upstream 변경 | High | High | 정기적 rebase + upstream 기여로 관계 유지 |
| Senior Rust 채용 실패 | High | Medium | ethrex/Reth 커뮤니티에서 기여자 영입 |
| 메인넷 싱크 실패 | High | Low | ethrex가 이미 성공, fork이므로 리스크 낮음 |
| Geth 버그 발견 못함 | Medium | Medium | 벤치마크 자체로도 가치 있음, 버그는 보너스 |

## Success Metrics

### 6개월 (Phase 1+2 완료)
- [ ] 메인넷 풀 싱크 + 30일 연속 운영
- [ ] Ethereum Hive 테스트 95%+ 통과
- [ ] 자동 벤치마크 대시보드 공개 (clients.tokamak.network)
- [ ] Differential testing에서 불일치 1건+ 발견
- [ ] Time-Travel Debugger로 과거 tx 리플레이 작동

### 12개월 (Phase 3 완료)
- [ ] JIT EVM으로 Geth 대비 2x+ 성능 달성
- [ ] Geth/Reth 버그 responsible disclosure 1건+
- [ ] 외부 노드 운영자 10명+ 채택
- [ ] EF 클라이언트 다양성 그랜트 수령

### 18개월 (Phase 4 완료)
- [ ] --tokamak-l2 플래그로 L2 동시 운영
- [ ] 노드 50개+ (nodewatch.io 집계)
- [ ] All Core Devs 미팅 정기 참석

## One-Liner

> **Tokamak Client: The Ethereum execution client that's fastest,
> proves it automatically, and shows you exactly why.**
