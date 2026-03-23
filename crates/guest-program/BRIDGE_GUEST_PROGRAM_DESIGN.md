# Bridge Guest Program 설계

> 작성일: 2026-03-23

## 목표

**evm-l2의 범용 EVM 증명 대신, bridge 전용 경량 ZK 증명을 생성하는 게스트 프로그램**

- evm-l2: 2200만+ 사이클, SP1 compress 15분+
- zk-dex: 특화 회로, ~5분 이내
- **bridge: deposit/withdraw만 처리, 5분 이내 목표**

## 핵심 차이: execution_program vs execute_app_circuit

```
evm-l2 (느림)                      bridge (빠름)
─────────────                      ─────────────
execution_program(ProgramInput)    execute_app_circuit(BridgeCircuit, AppProgramInput)
 └── 전체 EVM 실행                  └── 공통 핸들러만 사용
     ├── 모든 opcode 처리               ├── deposit (L1→L2)
     ├── 전체 state trie                ├── withdrawal (L2→L1)
     └── 2200만 사이클                  ├── ETH transfer
                                        └── gas fee 분배
```

## 데이터 흐름

```
Prover
 ├── ProgramInput (전체 witness 포함)
 │
 ▼
BridgeGuestProgram::serialize_input()
 ├── ProgramInput 역직렬화
 ├── analyze_bridge_transactions() → 필요 계정만 추출
 ├── convert_to_app_input() → AppProgramInput 생성
 │   └── 계정별 Merkle proof 추출
 └── AppProgramInput 직렬화 (rkyv)
 │
 ▼
SP1 zkVM (sp1-bridge ELF)
 ├── AppProgramInput 읽기
 ├── BridgeCircuit 생성
 ├── execute_app_circuit(&circuit, input)
 │   ├── verify_state_proofs()      ← Merkle proof 검증
 │   ├── For each tx:
 │   │   ├── Privileged?  → handle_privileged_tx()  [deposit]
 │   │   ├── To bridge?   → handle_withdrawal()     [withdraw]
 │   │   ├── ETH xfer?    → handle_eth_transfer()
 │   │   └── Other?       → BridgeCircuit.classify_tx() → Err (없음)
 │   ├── apply_gas_fee_distribution()
 │   ├── compute_new_state_root()   ← incremental MPT
 │   └── compute_message_digests()  ← L1/L2 메시지 해시
 └── ProgramOutput commit
```

## 구현 파일

### 1. `src/programs/bridge/circuit.rs` — BridgeCircuit

```rust
pub struct BridgeCircuit;

impl AppCircuit for BridgeCircuit {
    fn classify_tx(&self, _tx: &Transaction) -> Result<AppOperation, AppCircuitError> {
        // Bridge에는 앱 전용 트랜잭션이 없음
        // 모든 TX는 공통 핸들러(deposit/withdraw/transfer)로 처리됨
        Err(AppCircuitError::UnknownTransaction)
    }

    fn execute_operation(...) -> Result<OperationResult, AppCircuitError> {
        Err(AppCircuitError::UnknownTransaction) // 호출되지 않음
    }

    fn gas_cost(...) -> u64 { 0 }
    fn generate_logs(...) -> Vec<Log> { vec![] }
}
```

### 2. `src/programs/bridge/analyze.rs` — 트랜잭션 분석

zk-dex와의 차이:
- zk-dex: DEX 컨트랙트의 storage slot을 파싱 (주문서, 노트, 잔액)
- **bridge: storage slot 없음** — 계정 잔액만 필요

```
필요 계정:
├── 시스템: BRIDGE_L2(0xffff), MESSENGER(0xfffe), FEE_REGISTRY, FEE_RATIO
├── 블록: coinbase (블록당)
├── 트랜잭션: sender, receiver (각 TX)
└── 수수료: fee vault 주소

필요 storage:
├── MESSENGER.lastMessageId (slot 0x0) — withdrawal 시 증가
└── 없음 (bridge는 storage 없음)
```

### 3. `src/programs/bridge/mod.rs` — GuestProgram 구현

```
serialize_input(raw_input):
  1. ProgramInput 역직렬화
  2. analyze_bridge_transactions() → (accounts, storage_slots)
  3. convert_to_app_input(program_input, &accounts, &storage_slots)
  4. AppProgramInput 직렬화
```

### 4. `bin/sp1-bridge/src/main.rs` — zkVM 진입점

```rust
fn main() {
    let input = rkyv::from_bytes::<AppProgramInput>(&sp1_zkvm::io::read_vec());
    let circuit = BridgeCircuit;
    let output = execute_app_circuit(&circuit, input).unwrap();
    sp1_zkvm::io::commit_slice(&output.encode());
}
```

### 5. `bin/sp1-bridge/Cargo.toml`

evm-l2와 동일한 의존성 + **동일한 Cargo.lock** (중요!)

## 주의사항

### Cargo.lock 동기화
sp1-bridge와 sp1(evm-l2)는 같은 의존성을 사용하지만 별도 workspace.
**Cargo.lock이 달라지면 ELF가 다르게 빌드되어 SP1 proof panic 발생.**
→ sp1/Cargo.lock을 sp1-bridge/Cargo.lock에 복사해야 함

### AppCircuitError 변형
`AppCircuitError::UnknownTransaction` 사용 (NOT `UnknownOperation`)
→ 실제 enum 정의를 확인하고 올바른 variant 사용

### MESSENGER storage slot
withdrawal 처리 시 `L2_TO_L1_MESSENGER.lastMessageId` (slot 0x0)를 읽고 증가시킴
→ storage_slots에 `(MESSENGER, H256::zero())` 포함 필요

### handle_withdrawal에서 생성하는 Log 2개
1. `WithdrawalInitiated` from BRIDGE_L2 (0xffff)
2. `L1Message` from MESSENGER (0xfffe)
→ receipt root 일치를 위해 정확해야 함

## 성능 예상

| 항목 | evm-l2 | zk-dex | bridge |
|------|--------|--------|--------|
| 입력 크기 | 전체 witness | 필요 accounts + DEX storage | 필요 accounts만 |
| 사이클 수 | 2200만+ | ~500만 | ~200만 |
| SP1 compress | 15분+ | ~3분 | ~1분 |
| 총 시간 | 20분+ | ~5분 | ~2분 |

Bridge가 빠른 이유:
1. **storage proof 없음** → MPT 검증/재계산 최소화
2. **앱 로직 없음** → classify_tx/execute_operation 스킵
3. **ELF 크기 동일** (같은 코드) → SP1 setup 시간 동일

## 테스트 순서

1. `cargo check` — 컴파일 확인
2. `GUEST_PROGRAMS=bridge cargo build --features sp1,l2` — ELF 빌드
3. Docker compose up → deployer → L2 → prover
4. deposit 1 ETH → L2 반영 확인
5. withdraw 0.1 ETH → L2 잔액 확인
6. SP1 proof 생성 대기 (5분 이내)
7. L1 batch verified 확인
8. claimWithdrawal → L1 잔액 증가 확인
