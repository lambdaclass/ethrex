# Decision: ethrex Fork as Tokamak EL Client Base

> **ethrex fork를 선택한다. ZK-native 커스텀 EVM(LEVM), 관리 가능한 코드베이스(133K줄), 네이티브 L2 아키텍처가 결정적이다.**

## 1. 문제 정의

Tokamak은 이더리움 실행 계층(EL) 클라이언트가 필요하다. 목적:

1. **메인넷 합의 참여** — nodewatch.io에 집계되는 프로덕션 노드
2. **Tier S 기능 구현 기반** — JIT EVM, Continuous Benchmarking, Time-Travel Debugger
3. **L2 네이티브 통합** — `--tokamak-l2` 플래그로 동일 바이너리에서 L2 운영

이 세 가지를 동시에 만족하려면 EVM 실행 루프에 대한 완전한 제어권, ZK 증명과의 호환성, 그리고 L2 Hook 시스템이 필요하다.

## 2. 평가된 옵션

| Option | 설명 |
|--------|------|
| **A. ethrex Fork** | LambdaClass의 Rust EL 클라이언트. 자체 EVM(LEVM), 네이티브 L2/ZK 지원 |
| **B. Reth Fork** | Paradigm의 Rust EL 클라이언트. revm 기반, 모듈러 아키텍처 |
| **C. 처음부터 구축** | 새로운 Rust EL 클라이언트를 처음부터 개발 |
| **D. revm 단독** | revm 라이브러리만 사용하여 최소 실행 엔진 구축 |

## 3. 결정 매트릭스

| 기준 | 가중치 | ethrex | Reth | 처음부터 | revm |
|------|--------|--------|------|---------|------|
| 메인넷 동기화 시간 | 25% | 5 | 4 | 1 | 1 |
| EVM 수정 가능성 | 25% | 5 | 2 | 4 | 3 |
| ZK 호환성 | 20% | 5 | 1 | 2 | 1 |
| 코드베이스 관리성 | 15% | 4 | 2 | 5 | 3 |
| L2 아키텍처 정합성 | 15% | 5 | 3 | 3 | 1 |
| **가중 합계** | | **4.85** | **2.45** | **2.65** | **1.60** |

### 기준별 근거

**메인넷 동기화 시간 (25%)**
- ethrex: 이미 메인넷 싱크 성공 이력. Fork 후 3-6개월 내 가능
- Reth: 동일하게 성공 이력 있으나 코드 복잡도로 fork 관리 비용 높음
- 처음부터/revm: P2P, 상태관리, 동기화 전부 구현 필요. 12-24개월

**EVM 수정 가능성 (25%)**
- ethrex: LEVM은 자체 EVM. opcode 루프(`vm.rs:528-663`)를 직접 수정 가능
- Reth: revm은 외부 의존성. EVM 내부 수정 시 revm fork 필요 → 이중 유지보수
- 처음부터: 완전 제어이나 구현 비용 과대
- revm: opcode 단위 접근은 가능하나 노드 인프라 전무

**ZK 호환성 (20%)**
- ethrex: SP1, RISC0, ZisK, OpenVM 4개 프루버 네이티브 지원. ZK 증명이 핵심 아키텍처
- Reth: ZK 지원 없음. 별도 통합 필요
- 처음부터: ZK 통합을 직접 설계 가능하나 시간 소요
- revm: ZK 관련 인프라 없음

**코드베이스 관리성 (15%)**
- ethrex: 133K줄 Rust. 2-3명 팀으로 전체 이해/관리 가능
- Reth: 200K+ 줄. Paradigm 규모 팀 전제. 모듈러이나 복잡
- 처음부터: 코드량 최소화 가능하나 비현실적 시간
- revm: 라이브러리 자체는 작으나 노드 구축 시 코드 폭발

**L2 아키텍처 정합성 (15%)**
- ethrex: `VMType::L2(FeeConfig)` enum + `Hook` trait + L2Hook 이미 구현
- Reth: L2 지원은 OP Stack 통합(op-reth) 경로이나 아키텍처 방향 상이
- 처음부터: L2 설계 자유이나 시간
- revm: L2 인프라 없음

## 4. 핵심 근거 — 5가지 결정적 요인

### 4.1 LEVM 커스텀 EVM → JIT 삽입 포인트 명확

ethrex는 revm을 사용하지 않는다. 자체 EVM인 LEVM을 보유:

```
crates/vm/levm/src/vm.rs:528-663 — run_execution() 메인 루프
```

이 루프는 직접적인 `match opcode` 패턴으로 구현되어 있어, JIT 컴파일러 삽입이 명확하다:

- **Tier 0** (해석): 현재 `run_execution()` 그대로 사용
- **Tier 1** (Baseline JIT): `opcode_table[opcode]` 호출 시점에 JIT 컴파일된 코드로 분기
- **Tier 2** (Optimizing JIT): `build_opcode_table()` (`opcodes.rs:385`)의 fork별 테이블을 JIT 캐시로 대체

Reth의 revm은 외부 크레이트이므로 이 수준의 수정은 revm 자체를 fork해야 한다.

### 4.2 Hook 시스템 → `VMType::TokamakL2` 추가 용이

ethrex의 Hook 시스템은 이미 L1/L2 분기를 지원한다:

```rust
// crates/vm/levm/src/vm.rs:38-44
pub enum VMType {
    L1,
    L2(FeeConfig),
}

// crates/vm/levm/src/hooks/hook.rs:19-24
pub fn get_hooks(vm_type: &VMType) -> Vec<Rc<RefCell<dyn Hook + 'static>>> {
    match vm_type {
        VMType::L1 => l1_hooks(),
        VMType::L2(fee_config) => l2_hooks(*fee_config),
    }
}
```

Tokamak L2를 추가하려면:
1. `VMType` enum에 `TokamakL2(TokamakFeeConfig)` 변형 추가
2. `get_hooks()`에 `tokamak_l2_hooks()` 매핑 추가
3. `TokamakL2Hook`을 `Hook` trait으로 구현 (L2Hook 패턴 참조)

기존 L2Hook (`l2_hook.rs`, 844줄)이 완전한 참조 구현 역할을 한다.

### 4.3 멀티 프루버 ZK 네이티브 지원

ethrex는 SP1, RISC0, ZisK, OpenVM 4개의 ZK 프루버를 네이티브로 지원한다. Tokamak의 ZK MIPS 회로 팀 경험과 직접 연결되며, proven execution 아키텍처의 기반이 된다.

### 4.4 133K줄 = 2-3명 팀으로 관리 가능

```
ethrex: ~133,000줄 Rust (target 제외)
Reth:   ~200,000줄+ Rust
Geth:   ~500,000줄 Go
```

ethrex의 코드베이스는 Reth의 2/3, Geth의 1/4 수준이다. Senior Rust 엔지니어 2-3명이면 전체 코드베이스를 이해하고 유지보수할 수 있다. 이는 Tokamak 팀 규모(Rust 전담 2-3명 예상)에 적합하다.

### 4.5 `perf_opcode_timings` 기존 인프라 활용

ethrex는 이미 opcode 단위 성능 측정 인프라를 보유:

```rust
// crates/vm/levm/src/timings.rs
pub struct OpcodeTimings {
    totals: HashMap<Opcode, Duration>,
    counts: HashMap<Opcode, u64>,
    blocks: usize,
    txs: usize,
}

pub static OPCODE_TIMINGS: LazyLock<Mutex<OpcodeTimings>> = ...;
```

`#[cfg(feature = "perf_opcode_timings")]`로 활성화되며, `run_execution()` 루프에서 각 opcode의 실행 시간을 자동 측정한다. Continuous Benchmarking의 핵심 데이터 소스로 직접 활용 가능하다.

## 5. Tokamak 기능 → ethrex 아키텍처 매핑

| Tokamak 기능 | ethrex 컴포넌트 | 파일 | 통합 방법 |
|-------------|----------------|------|-----------|
| **JIT Compiler** | `VM::run_execution()` opcode 루프 | `crates/vm/levm/src/vm.rs:528-663` | Tier 1/2에서 opcode_table을 JIT 캐시로 대체 |
| **Time-Travel Debugger** | `LevmCallTracer` + `Substate` 백업 | `crates/vm/levm/src/tracing.rs` | LevmCallTracer 확장: opcode별 state snapshot 추가 |
| **Continuous Benchmarking** | `perf_opcode_timings` feature | `crates/vm/levm/src/timings.rs` | OpcodeTimings를 CI 파이프라인에 연결 |
| **Tokamak L2** | `VMType` enum + `Hook` trait | `crates/vm/levm/src/hooks/` | VMType::TokamakL2 + TokamakL2Hook 추가 |
| **Differential Testing** | `build_opcode_table()` fork 분기 | `crates/vm/levm/src/opcodes.rs:385` | 동일 트랜잭션을 Geth/ethrex 양쪽에서 실행, 결과 비교 |

## 6. 리스크 평가

| 리스크 | 영향 | 확률 | 완화 전략 |
|--------|------|------|-----------|
| **Upstream 분기** — ethrex가 호환 불가능한 방향으로 진화 | High | High | 정기적 rebase + upstream 기여로 관계 유지. 핵심 수정은 별도 레이어에 격리 |
| **JIT 합의 위반** — JIT 컴파일된 코드가 인터프리터와 다른 결과 생성 | Critical | Medium | 모든 JIT 결과를 인터프리터와 비교하는 validation mode. 불일치 시 인터프리터 결과 사용 |
| **LEVM 성숙도** — ethrex의 EVM이 Geth/revm보다 테스트 이력 짧음 | Medium | Medium | Ethereum Hive 테스트 통과율 모니터링. 초기에는 Hive 95%+ 달성이 선행 조건 |
| **인력 부족** — Senior Rust 엔지니어 + JIT/컴파일러 경험자 확보 어려움 | High | Medium | ethrex/Reth 오픈소스 커뮤니티에서 기여자 영입. ZK 회로 팀의 Rust 경험 활용 |
| **LambdaClass 관계** — Fork 시 협력 관계 유지 필요 | Medium | Low | 적극적 upstream 기여. Tokamak 전용 기능은 별도 크레이트로 분리 |

## 7. 다음 단계 — Phase별 로드맵

### Phase 1.1: Fork & 환경 구축 (Week 1-2)
- ethrex fork → `tokamak-client` 레포
- 메인넷/Holesky 빌드 검증
- CI 파이프라인 설정

### Phase 1.2: 메인넷 동기화 (Week 3-6)
- 메인넷 풀 싱크 시도
- Hive 테스트 프레임워크 통합
- 95%+ 통과율 달성

### Phase 1.3: Continuous Benchmarking MVP (Week 7-10)
- `perf_opcode_timings` 기반 벤치마크 러너
- Geth 대비 자동 비교 CI 파이프라인
- Differential testing (state root 비교)

### Phase 2: Time-Travel Debugger (Month 3-4)
- LevmCallTracer 확장 (opcode별 state snapshot)
- `debug_timeTravel` RPC endpoint
- Interactive CLI debugger

### Phase 3: JIT EVM (Month 5-7)
- Tier 0+1 (Cranelift baseline JIT)
- Ethereum 테스트 스위트 100% 통과 검증
- Tier 2 (opcode fusion, 최적화)

### Phase 4: Tokamak L2 통합 (Month 8-10)
- `VMType::TokamakL2` + Hook 구현
- `--tokamak-l2` CLI 플래그
- 브릿지, 시퀀서, 증명 검증

---

## Volkov PROCEED 기준 대응

| PROCEED 기준 | 충족 여부 | 근거 |
|-------------|-----------|------|
| #1. Q1-Q4 의사결정 완료 | **충족** | Q1: 프로덕션 노드(Track A). Q2: Rust. Q3: 노드 점유율 + L2 통합. Q4: 아래 참조 |
| #2. 6개월 로드맵 | **충족** | Phase 1-2 (위 섹션) |
| #3. 인력/예산 배분 | **부분** | Senior Rust 2명 + JIT 경험자 1명 필요. 구체 배정은 팀 결정 |
| #4. 경쟁사 차별점 3가지 | **충족** | (1) ZK-native EVM (2) 자동 증명 벤치마크 (3) 내장 Time-Travel 디버거 |
| #5. EXIT 기준 | **필요** | 6개월 내 Hive 95% 미달 시 재평가 |
| #6. Tier S 2주 PoC | **필요** | Phase 1.1 착수 후 `perf_opcode_timings` 기반 벤치마크 PoC |

### 6개월 성공 기준 (Q4 답변)

- [ ] ethrex fork 후 메인넷 풀 싱크 완료
- [ ] Ethereum Hive 테스트 95%+ 통과
- [ ] 자동 벤치마크 대시보드 공개 (clients.tokamak.network)
- [ ] Differential testing에서 Geth/Reth 불일치 1건+ 발견
- [ ] 내부 노드 3개 이상 안정 운영 (30일+ 업타임)

---

*Decision date: 2026-02-22*
*Author: Jason (with analysis from Phase 0-1/0-2 agents)*
*Status: **DRAFT** — 팀 리뷰 후 확정*
