# #21 Time-Travel Debugger

**Volkov Score: 7.5/10 — PROCEED**

## What

과거 트랜잭션을 해당 시점의 상태와 함께 리플레이하고,
단계별로 EVM 실행을 인터랙티브하게 디버깅할 수 있는 기능.

## Why This Matters

### Developer Pain Point
- 현재: 트랜잭션이 revert되면 "왜?"를 알기 어렵다
- Etherscan의 trace는 읽기 어렵고 맥락이 없다
- Tenderly가 SaaS로 제공하지만, 로컬/오프라인 불가
- Foundry의 `cast run --debug`는 제한적

### Differentiation
- 로컬 클라이언트에 내장된 time-travel debugger는 **없다**
- Geth: `debug_traceTransaction`은 raw trace만 제공
- Reth: 동일한 수준
- 연구자/개발자에게 가장 직접적인 가치

## Scope Definition

### MVP (Phase 1 — 4주)
```
입력: transaction hash + block number
처리: 해당 블록의 상태를 재구성 → 트랜잭션 리플레이
출력: opcode별 실행 trace + 스택/메모리/스토리지 스냅샷
```

- 최근 N블록 내 트랜잭션만 지원 (전체 히스토리는 비현실적)
- CLI 인터페이스: step forward, step back, breakpoint, inspect
- JSON-RPC 확장: `debug_timeTravel(txHash, options)`

### Phase 2 (추가 4주)
- Web UI (React): 시각적 실행 흐름 표시
- 상태 diff 하이라이팅 (변경된 storage slot 강조)
- 조건부 브레이크포인트 (특정 storage slot 변경 시 정지)

### Phase 3 (선택)
- "What-if" 모드: 트랜잭션 파라미터를 변경하여 리플레이
- AI 기반 실행 요약: "이 트랜잭션이 revert된 이유는..."

## Technical Approach

### Python 구현 시
```python
# py-evm 기반 상태 재구성
class TimeTravelDebugger:
    def replay_transaction(self, tx_hash: str, block_number: int):
        # 1. 해당 블록 직전 상태 로드
        state = self.load_state_at(block_number - 1)
        # 2. 블록 내 해당 tx 이전 tx들을 순서대로 실행
        state = self.apply_preceding_txs(state, block_number, tx_hash)
        # 3. 대상 tx를 opcode 단위로 실행하며 trace 기록
        trace = self.trace_execution(state, tx_hash)
        return trace
```

- py-evm의 EVM을 instrumented mode로 실행
- 각 opcode 실행 후 스택/메모리/스토리지 스냅샷 저장
- StateDB를 copy-on-write로 구현하여 "step back" 지원

### Rust 구현 시
- revm의 `Inspector` trait 활용
- 각 opcode 실행을 intercept하여 state snapshot 저장
- zero-copy 기법으로 메모리 효율 극대화

## Competitive Analysis

| Tool | Type | Local | Free | Interactive | State Replay |
|------|------|-------|------|-------------|-------------|
| Tenderly | SaaS | No | Limited | Yes | Yes |
| Foundry debug | CLI | Yes | Yes | Limited | Partial |
| Geth debug_trace | RPC | Yes | Yes | No | No |
| **Ours** | **Built-in** | **Yes** | **Yes** | **Yes** | **Yes** |

## Success Metrics

- [ ] 임의의 메인넷 트랜잭션을 5초 이내에 리플레이
- [ ] Step forward/backward가 50ms 이내 응답
- [ ] Tenderly 무료 티어와 동등한 정보량 제공
- [ ] 이더리움 연구자 5명에게 피드백 수집

## Estimated Effort

| Phase | Duration | Engineers |
|-------|----------|-----------|
| MVP | 4 weeks | 2 |
| Web UI | 4 weeks | 1 frontend + 1 backend |
| What-if | 4 weeks | 1 |

## Risk

- **상태 저장 용량**: 각 opcode마다 전체 state를 저장하면 메모리 폭발
  - 완화: copy-on-write + diff-based snapshot
- **성능**: 상태 재구성이 느릴 수 있음
  - 완화: 최근 N블록 캐시 + 체크포인트
