# Decision: ethrex Fork as Tokamak EL Client Base

> **ethrex fork를 선택한다. ZK-native 커스텀 EVM(LEVM), 관리 가능한 코드베이스(133K줄), 네이티브 L2 아키텍처가 가장 적합하다.**

## 1. 문제 정의

Tokamak은 이더리움 실행 계층(EL) 클라이언트가 필요하다. 목적:

1. **메인넷 합의 참여** — nodewatch.io에 집계되는 프로덕션 노드
2. **Tier S 기능 구현 기반** — JIT EVM, Continuous Benchmarking, Time-Travel Debugger
3. **L2 네이티브 통합** — `--tokamak-l2` 플래그로 동일 바이너리에서 L2 운영

이 세 가지를 동시에 만족하려면 EVM 실행 루프에 대한 완전한 제어권, ZK 증명과의 호환성, 그리고 L2 Hook 시스템이 필요하다.

## 2. 평가된 옵션

실질적인 후보는 **ethrex fork**와 **Reth fork** 두 가지다.

| Option | 설명 |
|--------|------|
| **A. ethrex Fork** | LambdaClass의 Rust EL 클라이언트. 자체 EVM(LEVM), 네이티브 L2/ZK 지원 |
| **B. Reth Fork** | Paradigm의 Rust EL 클라이언트. revm 기반, 모듈러 아키텍처, ExEx 프레임워크 |

> **제외된 옵션**: "처음부터 구축"(12-24개월 소요, 비현실적)과 "revm 단독"(노드 인프라 전무)은 Tokamak의 6개월 목표와 양립 불가하여 본문에서 제외한다. 상세 비교는 [부록 A](#부록-a-제외된-옵션) 참조.

## 3. 결정 매트릭스 — ethrex vs Reth

| 기준 | 가중치 | ethrex | Reth | 차이 |
|------|--------|--------|------|------|
| 메인넷 동기화 시간 | 25% | 4 | 4 | 0 |
| EVM 수정 가능성 | 25% | 5 | 2 | +3 |
| ZK 호환성 | 20% | 5 | 2 | +3 |
| 코드베이스 관리성 | 15% | 4 | 3 | +1 |
| L2 아키텍처 정합성 | 15% | 5 | 3 | +2 |
| **가중 합계** | | **4.60** | **2.80** | **+1.80** |

### 기준별 근거

**메인넷 동기화 시간 (25%)**
- ethrex: 메인넷 싱크 성공 이력 있음. <1% 점유율로 실전 검증은 Reth보다 적음
- Reth: ~5% 점유율으로 더 많은 실전 검증. 그러나 코드 복잡도로 fork 유지 비용 높음
- **양쪽 모두 4점**: 싱크 능력 자체는 동등하다. ethrex가 실전 이력이 짧은 대신 fork 관리 비용이 낮고, Reth가 실전 이력이 긴 대신 fork 복잡도가 높아 상쇄됨

**EVM 수정 가능성 (25%)**
- ethrex **(5)**: LEVM은 자체 EVM. opcode 루프(`vm.rs:528-663`)를 직접 수정 가능. JIT 삽입, opcode 추가, 실행 흐름 변경이 단일 코드베이스 내에서 완결
- Reth **(2)**: revm은 외부 의존성. EVM 실행 루프를 수정하려면 revm 자체를 fork해야 함 → 이중 유지보수 부담. ExEx(Execution Extensions)는 **블록 실행 후** 상태 변경을 수신하는 post-execution hook이며, EVM 실행 자체를 수정하는 메커니즘이 아님

**ZK 호환성 (20%)**
- ethrex **(5)**: SP1, RISC0, ZisK, OpenVM 4개 프루버가 네이티브로 통합 (`crates/l2/prover/src/backend/`). ZK 증명이 핵심 아키텍처
- Reth **(2)**: Zeth(risc0/zeth)가 Reth의 stateless execution을 zkVM 내에서 사용하여 블록 증명 가능. 그러나 Zeth는 Reth에 내장된 것이 아니라 **RISC Zero가 관리하는 별도 프로젝트**(439 stars)이며, RISC Zero 프루버만 지원. ethrex의 네이티브 4-프루버 지원과는 통합 깊이가 다름. 1점에서 2점으로 상향 조정

**코드베이스 관리성 (15%)**
- ethrex **(4)**: 133K줄 Rust. 전체 구조 파악 가능하나 upstream rebase 비용 존재
- Reth **(3)**: 200K+ 줄이지만 모듈러 아키텍처(ExEx, reth-primitives 등)와 Paradigm의 지속적 투자로 문서화/생태계가 우수. 2점에서 3점으로 상향 조정

**L2 아키텍처 정합성 (15%)**
- ethrex **(5)**: `VMType::L2(FeeConfig)` enum + `Hook` trait + L2Hook 구현 완료. `prepare_execution()` / `finalize_execution()`으로 트랜잭션 실행 전후를 제어
- Reth **(3)**: op-reth(OP Stack 통합)으로 L2 지원. ExEx로 파생 상태 계산 가능. 그러나 Tokamak 고유의 fee 구조/Hook이 필요하면 revm 레벨 수정 불가피

## 4. 핵심 근거 — 5가지 주요 요인

### 4.1 LEVM 커스텀 EVM → JIT 삽입 가능

ethrex는 revm을 사용하지 않는다. 자체 EVM인 LEVM을 보유:

```
crates/vm/levm/src/vm.rs:528-663 — run_execution() 메인 루프
```

이 루프는 직접적인 `match opcode` 패턴으로 구현되어 있어, JIT 컴파일러 삽입 포인트가 식별 가능하다:

- **Tier 0** (해석): 현재 `run_execution()` 그대로 사용
- **Tier 1** (Baseline JIT): `opcode_table[opcode]` 호출 시점에 JIT 컴파일된 코드로 분기
- **Tier 2** (Optimizing JIT): `build_opcode_table()` (`opcodes.rs:385`)의 fork별 테이블을 JIT 캐시로 대체

Reth의 revm은 외부 크레이트이므로 이 수준의 수정은 revm 자체를 fork해야 한다.

**기술적 장벽**: EVM의 동적 점프(`JUMP`, `JUMPI`)는 JIT 컴파일의 근본적 난제다. 점프 대상이 런타임에 결정되므로 사전에 기본 블록(basic block) 경계를 확정할 수 없다. revmc(revm JIT 프로젝트)가 이 문제에 대한 선행 연구를 진행 중이며, Tokamak JIT 설계 시 참조해야 한다. "삽입 가능"은 "구현이 쉽다"를 의미하지 않는다.

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

### 4.4 133K줄 = AI Agent 기반 개발에 최적

```
ethrex: ~133,000줄 Rust (target 제외)
Reth:   ~200,000줄+ Rust
Geth:   ~500,000줄 Go
```

ethrex의 코드베이스는 Reth의 2/3, Geth의 1/4 수준이다. AI Agent(Claude Code 등)가 전체 코드베이스를 컨텍스트 내에서 파악하고 수정할 수 있는 규모이며, 200K줄 이상인 Reth는 Agent의 컨텍스트 윈도우 한계에 더 빨리 도달한다.

**개발 모델**: AI Agent가 코드 작성·리뷰·테스트를 수행하고, Jason이 의사결정·방향 설정·최종 승인을 담당한다. 개발 완료 후 팀(Kevin, Harvey, Jake, Sahil 등)과 결과물 기반 토론을 진행한다.

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
| **Agent 한계** — AI Agent가 복잡한 아키텍처 결정이나 저수준 최적화에서 한계 노출 | Medium | Medium | 단계별 검증(Hive 테스트, differential testing)으로 Agent 출력물 품질 보장. 난이도 높은 결정은 팀 토론으로 보완 |
| **Bus factor** — 의사결정자(Jason)가 1명. 부재 시 프로젝트 정지 | High | Low | Jason 2주 이상 부재 시 현재 Phase 동결. Kevin이 임시 의사결정권을 갖고 긴급 이슈(upstream breaking change, 보안 취약점)에 한해 대응. Phase 전환 결정은 Jason 복귀까지 보류 |

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

## 8. EXIT 기준

프로젝트 중단 또는 방향 전환의 명확한 조건:

| 수치 | 기한 | 미달 시 행동 | 의사결정자 |
|------|------|-------------|-----------|
| 메인넷 풀 싱크 완료 | 4개월 | ethrex upstream에 버그 리포트 + 1회 재시도. 재시도 실패 시 Reth fork 전환 평가 | Tech leads |
| Hive 테스트 95%+ 통과 | 6개월 | 실패 테스트 분석 → ethrex upstream 기여로 해결 시도. 80% 미만이면 프로젝트 중단 검토 | Tech leads + Kevin |
| 내부 노드 30일 연속 업타임 | 6개월 | 아키텍처 재검토. crash 원인이 LEVM 성숙도이면 revm 병행 검토 | Full team |

**핵심 원칙**: "재평가"가 아니라 구체적 행동을 정의한다. 각 기한에서 Go/No-Go를 결정하고, No-Go 시의 대안 경로가 명시되어 있다.

---

## 9. Tier S PoC: `perf_opcode_timings` 벤치마크

### 빌드 검증

```
$ cargo build --features perf_opcode_timings
  Finished `dev` profile [unoptimized + debuginfo] target(s) in 3m 44s
```

`perf_opcode_timings` feature flag로 ethrex가 정상 빌드됨을 확인. 이 feature를 활성화하면 `run_execution()` 루프 내에서 모든 opcode의 실행 시간이 자동 측정된다.

### 동작 원리 확인

빌드된 바이너리에서 블록 실행 시 다음 코드 경로가 활성화된다:

```rust
// crates/vm/levm/src/vm.rs:551-646
#[cfg(feature = "perf_opcode_timings")]
let mut timings = crate::timings::OPCODE_TIMINGS.lock().expect("poison");

loop {
    let opcode = self.current_call_frame.next_opcode();
    // ...
    #[cfg(feature = "perf_opcode_timings")]
    let opcode_time_start = std::time::Instant::now();

    let op_result = match opcode { /* ... */ };

    #[cfg(feature = "perf_opcode_timings")]
    {
        let time = opcode_time_start.elapsed();
        timings.update(opcode, time);
    }
}

// crates/vm/backends/levm/mod.rs:261-268
#[cfg(feature = "perf_opcode_timings")]
{
    let mut timings = OPCODE_TIMINGS.lock().expect("poison");
    timings.inc_tx_count(receipts.len());
    timings.inc_block_count();
    tracing::info!("{}", timings.info_pretty());
}
```

블록 실행 완료 후 `info_pretty()`가 opcode별 평균/누적 시간, 호출 횟수를 로깅한다. 출력 형식:

```
[PERF] opcode timings avg per block (blocks=N, txs=N, total=Ns, sorted desc):
SSTORE               12.345µs          1.234s (    100000 calls)
SLOAD                 8.901µs          0.890s (    100000 calls)
CALL                  5.678µs          0.567s (    100000 calls)
...
```

### PoC 결론

1. **Feature flag가 동작한다**: `--features perf_opcode_timings`로 빌드 성공, 코드 경로 확인 완료
2. **opcode별 측정이 자동화되어 있다**: 별도 instrumentation 없이 모든 opcode의 실행 시간이 측정됨
3. **CI 연결이 직관적이다**: `RUST_LOG=info` 환경에서 블록 실행 시 자동 출력 → CI에서 파싱하여 대시보드로 전송 가능
4. **Continuous Benchmarking MVP의 기반으로 충분하다**: 추가 개발 없이 기존 인프라만으로 opcode 성능 기준선(baseline)을 수립할 수 있음

> 메인넷 싱크 후 실제 블록에서의 타이밍 데이터 수집은 Phase 1.2에서 수행한다. 현 단계에서는 인프라의 존재와 동작을 확인하는 것이 PoC의 범위다.

---

## Volkov PROCEED 기준 대응

| PROCEED 기준 | 충족 여부 | 근거 |
|-------------|-----------|------|
| #1. Q1-Q4 의사결정 완료 | **충족** | Q1: 프로덕션 노드(Track A). Q2: Rust. Q3: 노드 점유율 + L2 통합. Q4: 아래 참조 |
| #2. 6개월 로드맵 | **충족** | Phase 1-4 (섹션 7) |
| #3. 인력/예산 배분 | **충족** | AI Agent 기반 개발. Jason이 의사결정, Agent가 구현·리뷰·테스트 수행. 팀과 결과물 기반 토론 |
| #4. 경쟁사 차별점 3가지 | **충족** | (1) ZK-native 4-프루버 EVM (2) 자동 증명 벤치마크 (3) 내장 Time-Travel 디버거 |
| #5. EXIT 기준 | **충족** | 4개 수치 × 기한 × 미달 시 행동 × 의사결정자 (섹션 8) |
| #6. Tier S PoC | **충족** | `perf_opcode_timings` 빌드 검증 + 동작 원리 확인 (섹션 9) |

### 6개월 성공 기준 (Q4 답변)

- [ ] ethrex fork 후 메인넷 풀 싱크 완료
- [ ] Ethereum Hive 테스트 95%+ 통과
- [ ] 자동 벤치마크 대시보드 공개 (clients.tokamak.network)
- [ ] Differential testing에서 Geth/Reth 불일치 1건+ 발견
- [ ] 내부 노드 3개 이상 안정 운영 (30일+ 업타임)

---

## 부록 A: 제외된 옵션

| Option | 설명 | 제외 사유 |
|--------|------|-----------|
| **C. 처음부터 구축** | 새로운 Rust EL 클라이언트를 처음부터 개발 | P2P, 상태관리, 동기화 전부 구현 필요. 12-24개월. 6개월 목표와 양립 불가 |
| **D. revm 단독** | revm 라이브러리만 사용하여 최소 실행 엔진 구축 | 노드 인프라(P2P, RPC, 동기화) 전무. 사실상 "처음부터 구축"의 변형 |

---

*Decision date: 2026-02-22*
*Author: Jason (with analysis from Phase 0-1/0-2 agents)*
*Status: **FINAL** — 2026-02-22 확정*
