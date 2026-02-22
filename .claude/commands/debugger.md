# Time-Travel Debugger Developer

Time-Travel Debugger 전문 개발자 모드. opcode별 state snapshot, 트랜잭션 리플레이, RPC endpoint에 특화.

## 역할

ethrex의 LevmCallTracer를 확장하여 opcode 단위 Time-Travel Debugging을 구현한다.

## 기존 인프라

```rust
// crates/vm/levm/src/tracing.rs
pub struct LevmCallTracer {
    // 현재: call-level 트레이싱
    // 확장: opcode-level state snapshot 추가
}
```

## 구현 설계

### 1. State Snapshot 구조

```rust
pub struct OpcodeSnapshot {
    pub pc: usize,
    pub opcode: Opcode,
    pub stack: Vec<U256>,     // 스택 상태
    pub memory: Vec<u8>,      // 메모리 상태 (선택적, 큰 데이터)
    pub storage_changes: Vec<(Address, U256, U256)>,  // (addr, key, value)
    pub gas_remaining: u64,
    pub gas_used: u64,
}

pub struct TxTimeline {
    pub tx_hash: B256,
    pub snapshots: Vec<OpcodeSnapshot>,
    pub total_opcodes: usize,
}
```

### 2. 확장 포인트

```rust
// vm.rs — run_execution() 루프 내
loop {
    let opcode = self.current_call_frame.next_opcode();
    // ← snapshot 캡처 포인트
    let op_result = match opcode { ... };
    // ← post-execution snapshot
}
```

### 3. RPC Endpoint

```
debug_timeTravel(tx_hash, opcode_index) → OpcodeSnapshot
debug_timeTravelRange(tx_hash, start, end) → Vec<OpcodeSnapshot>
debug_timeTravelSearch(tx_hash, condition) → Vec<OpcodeSnapshot>
```

## 작업 흐름

1. LevmCallTracer 분석 → 확장 포인트 식별
2. OpcodeSnapshot 구조체 구현
3. run_execution() 루프에 snapshot 캡처 통합
4. RPC endpoint 구현
5. CLI 디버거 인터페이스
6. 메모리 사용량 최적화 (lazy snapshot, COW)

## 주의사항

- Phase 2 (Month 3-4)에 착수
- snapshot 캡처는 성능 오버헤드 → feature flag로 격리
- 메모리 사용량 주의: 대형 트랜잭션은 수천 개 opcode → snapshot 압축 필요
- 기존 `debug_traceTransaction` RPC와 호환성 유지

$ARGUMENTS
