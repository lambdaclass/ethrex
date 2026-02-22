# #10 Continuous Benchmarking Client

**Volkov Score: 7.5/10 — PROCEED**

## What

매 커밋마다 자동으로 Geth/Reth 대비 성능을 측정하고,
결과를 공개 대시보드로 발행하는 내장 벤치마킹 시스템.

## Why This Matters

### Core Problem
- "우리 클라이언트가 Geth보다 나은 점이 무엇인가?"
- 이 질문에 **숫자로** 답할 수 없으면 아무도 전환하지 않는다
- 현재 클라이언트 간 비교는 수동적이고 비체계적

### Strategic Alignment
- **Sahil의 differential testing 전략과 직결**:
  벤치마킹 중 결과가 다른 트랜잭션을 발견하면 → 잠재적 버그
  → responsible disclosure → All Core Devs 진입 → 신뢰 구축
- **Harvey의 메트릭 공개 제안을 자동화**:
  sync time, memory usage, crash rate를 수동이 아닌 CI/CD로

### Differentiation
- 어떤 이더리움 클라이언트도 이것을 내장하고 있지 않다
- Ethereum Hive가 외부 테스트 프레임워크로 존재하지만
  클라이언트 내부에서 자동화된 것은 없다

## Scope Definition

### MVP (Phase 1 — 3주)

```
[매 커밋/PR]
    │
    ▼
┌─ Benchmark Suite ──────────────────┐
│                                    │
│  1. Sync Performance               │
│     - Full sync 시작 → N블록 동기화│
│     - 시간, 메모리, 디스크 I/O 측정│
│                                    │
│  2. Transaction Execution          │
│     - 표준 벤치마크 트랜잭션 세트  │
│     - ERC-20 transfer, Uniswap     │
│       swap, complex DeFi 등        │
│     - gas/sec, latency 측정        │
│                                    │
│  3. State Access                   │
│     - 랜덤 계정 조회 latency       │
│     - Storage slot 읽기/쓰기 속도  │
│                                    │
│  4. Memory Profile                 │
│     - Peak RSS, steady-state RSS   │
│     - GC pause time (Python)       │
│                                    │
└────────────────────────────────────┘
    │
    ▼
[비교 대상: Geth latest, Reth latest]
    │
    ▼
[결과 → GitHub Pages 대시보드]
```

### Phase 2 (추가 3주)
- **Differential Testing 통합**:
  동일 트랜잭션을 우리 클라이언트 + Geth + Reth에서 실행
  → 결과가 다르면 자동 알림
  → 잠재적 합의 버그 후보 목록 생성
- **성능 회귀 감지**: PR이 성능을 N% 이상 저하시키면 자동 블록

### Phase 3 (선택)
- 공개 리더보드: clients.tokamak.network
- 커뮤니티 기여 벤치마크 시나리오 제출
- 히스토리컬 트렌드 차트 (클라이언트별 성능 추이)

## Technical Approach

### Benchmark Runner
```python
class BenchmarkSuite:
    """Ethereum Hive 호환 벤치마크 러너"""

    def __init__(self, clients: list[ClientConfig]):
        self.clients = clients  # [our_client, geth, reth]
        self.scenarios = self.load_scenarios()

    def run_comparison(self, scenario: Scenario) -> ComparisonResult:
        results = {}
        for client in self.clients:
            results[client.name] = {
                "execution_time": self.measure_execution(client, scenario),
                "memory_peak": self.measure_memory(client, scenario),
                "state_root": self.get_state_root(client, scenario),
            }

        # Differential check
        state_roots = {r["state_root"] for r in results.values()}
        if len(state_roots) > 1:
            return ComparisonResult(
                status="DIVERGENCE_DETECTED",
                details=results
            )

        return ComparisonResult(status="OK", details=results)
```

### CI Integration
```yaml
# .github/workflows/benchmark.yml
on:
  push:
    branches: [main]
  pull_request:

jobs:
  benchmark:
    runs-on: ubuntu-latest-16core
    steps:
      - name: Run benchmark suite
        run: python -m benchmark.runner --compare geth,reth
      - name: Check for regressions
        run: python -m benchmark.regression_check --threshold 5%
      - name: Publish results
        run: python -m benchmark.publish --output gh-pages
```

### Dashboard
- GitHub Pages 기반 정적 사이트
- Chart.js로 성능 추이 시각화
- 매 커밋마다 자동 업데이트

## Competitive Analysis

| Feature | Hive | Our Built-in | Manual Testing |
|---------|------|-------------|----------------|
| 자동화 | Partial | Full CI/CD | No |
| 클라이언트 비교 | Yes | Yes + differential | Manual |
| 회귀 감지 | No | Automatic | No |
| 공개 대시보드 | No | Yes | No |
| 버그 감지 | No | Differential testing | No |

## Success Metrics

- [ ] 매 PR마다 Geth/Reth 대비 벤치마크 자동 실행
- [ ] 5% 이상 성능 회귀 시 자동 블록
- [ ] 1개 이상의 differential testing 불일치 발견
- [ ] 공개 대시보드 런칭 (clients.tokamak.network)

## Estimated Effort

| Phase | Duration | Engineers |
|-------|----------|-----------|
| MVP (벤치마크 + CI) | 3 weeks | 1 |
| Differential testing | 3 weeks | 1 |
| Dashboard | 2 weeks | 1 frontend |

## Risk

- **CI 비용**: Geth/Reth를 매번 빌드하고 실행하는 것은 비용이 큼
  - 완화: 매 PR은 quick bench, main merge 시 full bench
- **환경 차이**: CI runner의 성능이 실제 노드와 다름
  - 완화: 상대적 비교(absolute 값보다 ratio 중심)
- **Geth/Reth 버전 관리**: 비교 대상 버전을 어떻게 관리할 것인가
  - 완화: latest stable 고정, 주 1회 업데이트

## Strategic Value: Sahil's Trust-Building Path

```
Continuous Benchmarking
    │
    ├─ differential testing에서 Geth 버그 발견
    │
    ├─ responsible disclosure to Geth team
    │
    ├─ All Core Devs (ACD) 미팅 초대
    │
    ├─ 이더리움 커뮤니티 신뢰 확보
    │
    └─ "Tokamak이 이더리움 보안에 기여하는 팀"
       → 노드 운영자들의 자발적 채택 유도
```

이것이 Harvey의 "운영 이력 → 신뢰"보다 10배 빠른 신뢰 구축 경로다.
