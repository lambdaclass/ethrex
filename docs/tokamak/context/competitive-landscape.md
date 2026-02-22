# Competitive Landscape

## Ethereum Execution Layer Clients

### Production Clients (Mainnet Ready)

| Client | Language | Share | Backing | Strength | Weakness |
|--------|----------|-------|---------|----------|----------|
| **Geth** | Go | ~55% | EF | 10년 battle-tested, 최대 생태계 | 슈퍼다수 리스크, 레거시 코드 |
| **Nethermind** | C#/.NET | ~18% | Nethermind | 안정적 대안 | .NET 생태계 한계 |
| **Besu** | Java | ~12% | ConsenSys/HL | 엔터프라이즈 친화 | 성능 열위 |
| **Erigon** | Go | ~8% | Erigon team | 아카이브 노드 특화 | UX 열위 |
| **Reth** | Rust | ~5% | Paradigm | 최고 성능, 모듈러 | 아직 성숙도 낮음 |

### Emerging Clients

| Client | Language | Share | Backing | Focus | Relevance |
|--------|----------|-------|---------|-------|-----------|
| **ethrex** | Rust | <1% | LambdaClass | L2 통합, 경량 | 직접 경쟁자/협력 후보 |
| **EtherX** | ? | ~0.4% | EF 지원 | zkVM L2 기반 제공 | 전략 모델 |
| **EELS** | Python | 0% | EF | 공식 스펙 참조 구현 | Python 선택 시 기반 |
| **py-evm** | Python | 0% | EF/Trinity | 연구용 | Python 선택 시 참조 |

### Developer Tools (Not Clients, But Competing for Same Users)

| Tool | Type | Use Case | Relevance |
|------|------|----------|-----------|
| **Foundry/Anvil** | 로컬 테스트넷 | 개발/테스트 | 연구 도구로 가면 직접 경쟁 |
| **Hardhat** | 개발 프레임워크 | 개발/테스트 | JS/TS 생태계 |
| **Tenderly** | SaaS 디버거 | 디버깅/시뮬레이션 | Time-Travel 기능 경쟁 |
| **tevm** | 브라우저 EVM | 브라우저 내 실행 | Hammer 참조 |

## Key Strategic Observations

### 1. Geth Supermajority Problem
- Geth ~55%는 이더리움 생태계의 가장 큰 리스크
- Geth 버그 → 네트워크 분할 가능
- EF가 클라이언트 다양성에 적극적으로 펀딩
- **기회**: 이 내러티브를 타면 EF 그랜트 접근 가능

### 2. Reth의 급부상
- Paradigm 자금력 + Rust 성능 → 가장 빠르게 성장
- 모듈러 아키텍처 → 생태계 확장 중
- **위협**: "Rust EL 클라이언트" 니치를 이미 선점

### 3. Python 공백
- 프로덕션 Python EL 클라이언트: 0개
- EF의 EELS: 스펙 참조용이지 실행용이 아님
- py-evm/Trinity: 사실상 중단
- AI 에이전트 생태계: 99% Python
- **기회**: 명확한 빈 공간

### 4. L1 Client → L2 Adoption 인과관계

| Client | L1 Share | Related L2 | L2 Rank | Causal? |
|--------|----------|------------|---------|---------|
| Geth | 55% | - | - | N/A |
| Besu | 12% | Linea | ~10th | No |
| Reth | 5% | Base (#1 통해) | #1 | Reverse (L2→L1) |
| Nethermind | 18% | - | - | No evidence |

**결론**: L1 share가 L2 adoption을 유발한다는 증거 없음.
오히려 Base의 성공이 Reth 채택을 끌어올린 역방향 인과.

### 5. etherX Model
- L1과 L2가 코드베이스 90% 공유
- `--l2` 플래그 하나로 L2 배포
- 내장 브릿지, watcher, verifier
- **모방 가치**: 높음. 이 아키텍처를 차용하면
  "노드 설치 = L2 배포"가 가능

## Build vs Fork vs Contribute Decision Matrix

| Option | Cost | Time | Risk | Control | Community Credit |
|--------|------|------|------|---------|-----------------|
| **A. ethrex Fork** | Low | 2-3mo | Medium (divergence) | High | Low |
| **B. ethrex Contribute** | Low | Ongoing | Low | Low | High |
| **C. Reth Fork** | Medium | 3-6mo | High (complexity) | High | Low |
| **D. New from Scratch** | Very High | 12-24mo | Very High | Full | High if successful |
| **E. EELS/py-evm Fork** | Low | 1-2mo | Medium | High | Medium |

### Recommendation by Strategy

- **Python 전략** → Option E (EELS/py-evm Fork) + Tokamak L2 통합
- **Rust 전략** → Option A (ethrex Fork) or B (ethrex Contribute)
- **From scratch** → Option D — 비추천 (리소스 대비 리스크 과대)
