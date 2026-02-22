# #9 JIT-Compiled EVM

**Volkov Score: 7.0/10 — PROCEED (Conditional: Rust only)**

## What

EVM 바이트코드를 런타임에 네이티브 머신 코드로 JIT 컴파일하여
인터프리터 대비 2-5x 실행 성능 향상을 달성.

## Critical Decision

> **이 기능은 Rust 전략을 선택해야만 가능하다.**
> Python으로는 JIT 컴파일러를 구현할 수 없다.
> 이 기능을 선택하면 Python 전략(AI/연구자 친화)과 양립 불가.

| Strategy | JIT Available | Trade-off |
|----------|:---:|-----------|
| Python | No | AI/연구자 생태계 접근성 |
| Rust | Yes | 성능 차별화 가능, zk-VM 호환 |

## Why This Matters

### Current State of EVM Execution
- **Geth**: Go 인터프리터 (비교 기준)
- **Reth/revm**: Rust 인터프리터 (Geth 대비 ~1.5-2x 빠름)
- **evmone**: C++ 인터프리터 (가장 빠른 인터프리터)
- **JIT EVM**: 아무도 프로덕션에 배포하지 않았다 ← 빈 공간

### Why JIT Wins
```
인터프리터:  opcode 하나 → dispatch → 실행 → 다음 opcode → dispatch → ...
JIT:        opcode 패턴 → 네이티브 코드 블록 → 한번에 실행

핫 패스(자주 호출되는 컨트랙트):
- Uniswap Router: 초당 수천 회 호출 → JIT 효과 극대화
- ERC-20 transfer: 단순하지만 빈도 높음 → JIT 이점 큼
```

### Performance Expectation

| Execution Type | Interpreter | JIT (Expected) | Improvement |
|---------------|-------------|-----------------|-------------|
| Simple transfer | 1x | 1.2-1.5x | Minimal (JIT overhead) |
| Complex DeFi | 1x | 2-3x | Significant |
| Loop-heavy | 1x | 3-5x | Maximum |
| Cold (first call) | 1x | 0.8x | Slower (compilation cost) |

핵심: JIT는 반복 실행에서 이점. 한 번만 실행되는 코드에서는 오히려 느림.

## Technical Approach

### Architecture
```
EVM Bytecode
    │
    ▼
┌─ Tiered Execution ─────────────────┐
│                                    │
│  Tier 0: Interpreter               │
│  - 모든 바이트코드의 기본 실행 경로│
│  - 실행 횟수 카운터 수집            │
│                                    │
│  Tier 1: Baseline JIT              │
│  - 실행 횟수 > threshold인 코드    │
│  - 빠른 컴파일, 기본 최적화        │
│  - Cranelift backend               │
│                                    │
│  Tier 2: Optimizing JIT            │
│  - 매우 자주 실행되는 핫 코드       │
│  - 프로파일 기반 최적화            │
│  - LLVM backend (선택)             │
│                                    │
└────────────────────────────────────┘
```

### Key Optimizations
1. **Opcode Fusion**: `PUSH1 + ADD` → 단일 네이티브 명령
2. **Stack → Register Mapping**: EVM 스택을 CPU 레지스터에 매핑
3. **Dead Code Elimination**: 도달 불가 opcode 제거
4. **Constant Folding**: 컴파일 타임에 상수 연산 해결
5. **Inline Caching**: 자주 접근하는 storage slot 캐싱

### Implementation Plan (Rust)

```rust
use cranelift::prelude::*;

pub struct JitCompiler {
    /// Cranelift JIT builder
    builder: JITBuilder,
    /// Compiled code cache: bytecode hash -> native code
    cache: HashMap<B256, CompiledCode>,
    /// Execution counter per contract
    counters: HashMap<Address, u64>,
    /// JIT compilation threshold
    threshold: u64,
}

impl JitCompiler {
    pub fn execute(&mut self, bytecode: &[u8], context: &mut EvmContext) -> ExecutionResult {
        let hash = keccak256(bytecode);

        // Check if already compiled
        if let Some(compiled) = self.cache.get(&hash) {
            return compiled.execute(context);
        }

        // Increment counter
        let count = self.counters.entry(context.address).or_insert(0);
        *count += 1;

        if *count >= self.threshold {
            // JIT compile
            let compiled = self.compile(bytecode)?;
            self.cache.insert(hash, compiled);
            return self.cache[&hash].execute(context);
        }

        // Fall back to interpreter
        self.interpret(bytecode, context)
    }
}
```

### Cranelift vs LLVM

| Aspect | Cranelift | LLVM |
|--------|-----------|------|
| Compilation speed | Fast (ms) | Slow (100ms+) |
| Code quality | Good (80%) | Best (100%) |
| Binary size | Small | Large |
| Rust integration | Native | FFI |
| **Recommendation** | **Tier 1** | Tier 2 (optional) |

## Scope Definition

### Phase 1 — Baseline JIT (8주)
- Cranelift 기반 Tier 0 + Tier 1 구현
- 기본 opcode 지원 (arithmetic, stack, memory, storage)
- 벤치마크: Geth/Reth interpreter 대비 성능 측정
- 정확성: Ethereum test suite 100% 통과

### Phase 2 — Optimization (6주)
- Opcode fusion, constant folding
- Stack → register mapping
- Hot path profiling + adaptive compilation threshold
- 목표: complex DeFi 트랜잭션 2x 이상 개선

### Phase 3 — Production Hardening (4주)
- Memory limit per compiled code
- Cache eviction policy (LRU)
- Security audit: JIT가 consensus에 영향을 주지 않는지 검증
- Fuzzing: 악의적 바이트코드에 대한 방어

## Competitive Analysis

| Client | EVM Execution | JIT | Performance |
|--------|--------------|:---:|-------------|
| Geth | Go interpreter | No | Baseline |
| Reth | revm (Rust interpreter) | No | ~1.5-2x Geth |
| evmone | C++ interpreter | No | ~2x Geth |
| **Ours** | **Rust JIT** | **Yes** | **Target: 3-5x Geth** |

## Success Metrics

- [ ] Ethereum consensus test suite 100% 통과
- [ ] Uniswap V3 swap 트랜잭션 Geth 대비 2x+ 빠름
- [ ] JIT compilation overhead < 10ms per contract
- [ ] 메모리 사용량 Reth 대비 20% 이내 증가
- [ ] 5,000시간 fuzzing 후 crash 0건

## Estimated Effort

| Phase | Duration | Engineers | Skill Required |
|-------|----------|-----------|----------------|
| Baseline JIT | 8 weeks | 2 senior Rust | Compiler, EVM internals |
| Optimization | 6 weeks | 2 | Profiling, Cranelift |
| Hardening | 4 weeks | 1 + 1 security | Fuzzing, audit |

**Total: ~18 weeks, 2-3 senior Rust engineers**

## Risk

- **Consensus 위반**: JIT 결과가 interpreter와 1bit라도 다르면 포크 발생
  - 완화: 모든 JIT 결과를 interpreter와 비교하는 validation mode
  - fuzzing + ethereum test suite 필수 통과
- **인력 확보**: senior Rust + compiler 경험자는 시장에서 희소
  - 완화: Cranelift 팀(Bytecode Alliance)에 자문 요청
- **JIT 공격 벡터**: 악의적 바이트코드로 JIT compiler exploit
  - 완화: WASM sandbox 내 JIT 실행, 코드 크기 제한
- **Python 전략 포기**: 이 기능을 선택하면 Sahil의 "AI/Python 네이티브" 전략과 양립 불가
  - 이 trade-off를 팀이 명시적으로 결정해야 함

## Verdict

JIT EVM은 **유일하게 측정 가능한 기술적 우위**를 제공한다.
"우리가 Geth보다 3배 빠르다"는 마케팅 문구는 강력하다.

하지만 비용이 크다:
- Rust 필수 → Python 생태계 포기
- Senior Rust engineer 2-3명 × 18주
- Consensus 안전성 검증에 추가 시간

**팀의 Q2 결정(Python vs Rust)이 이 기능의 생사를 결정한다.**
