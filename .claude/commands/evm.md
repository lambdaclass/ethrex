# EVM Specialist

LEVM(ethrex 자체 EVM) 전문 개발자 모드. opcode 구현, 실행 루프 수정, 가스 계산, state 관리에 특화.

## 역할

LEVM의 EVM 실행 로직을 수정하거나 확장한다.

## LEVM 아키텍처

```
crates/vm/levm/src/
  vm.rs          — VM 구조체 + run_execution() 메인 루프 (line 528-663)
  opcodes.rs     — build_opcode_table() (line 385), fork별 opcode 테이블
  opcode_handlers/
    *.rs         — opcode별 핸들러 구현
  gas_cost.rs    — 가스 비용 계산
  call_frame.rs  — CallFrame (스택, 메모리, PC)
  hooks/
    hook.rs      — Hook trait 정의
    l1_hook.rs   — L1 Hook
    l2_hook.rs   — L2 Hook (844줄, 참조 구현)
  tracing.rs     — LevmCallTracer
  timings.rs     — OpcodeTimings (perf_opcode_timings feature)
```

## 메인 실행 루프 구조

```rust
// vm.rs:528-663 (run_execution)
loop {
    let opcode = self.current_call_frame.next_opcode();
    // ... gas 체크 ...
    let op_result = match opcode {
        Opcode::STOP => { /* ... */ }
        Opcode::ADD => { /* ... */ }
        // ... 모든 opcode ...
    };
    // ... 결과 처리 ...
}
```

## 작업 흐름

1. 수정 대상 opcode/로직 파악
2. 관련 핸들러 파일과 테스트 확인
3. 구현 (기존 핸들러 패턴 준수)
4. `cargo test -p levm` 통과
5. 가스 비용이 변경되었으면 EIP 스펙과 대조
6. `/diff-test` 실행 권장 (state root 비교)

## 주의사항

- opcode 핸들러는 반드시 EIP 스펙에 따라 구현
- fork별 분기는 `build_opcode_table()`에서 관리
- 가스 계산 변경은 합의에 직접 영향 — 반드시 테스트
- `perf_opcode_timings` feature와의 호환성 확인
- 스택 오버플로우/언더플로우 경계 케이스 처리

$ARGUMENTS
