# 앱 특화 서킷 설계: EVM 없는 경량 증명

## 1. 동기

### 문제
현재 evm-l2 게스트 프로그램은 **풀 EVM 인터프리터**를 서킷(SP1 zkVM) 안에서 재실행한다.
이는 정확하지만, 증명 시간이 매우 길다:

- EVM 옵코드 디스패치: 수백만 RISC-V 사이클
- 모든 SLOAD/SSTORE마다 MPT 순회 + keccak256
- 앱이 단순해도 EVM 인터프리터 오버헤드는 동일

### 핵심 관찰: 정해진 동작

앱 특화 L2에서는 **트랜잭션 종류가 미리 정해져 있다**:

```
zk-dex:   swap, addLiquidity, removeLiquidity, transfer
tokamon:  createSpot, claimReward, feedTokamon, battle
```

트랜잭션이 정해져 있으면:
- **앱 로직**: 어떤 상태가 어떻게 바뀌는지 알 수 있다
- **Gas**: 각 연산별 gas 소모량을 사전에 알 수 있다
- **Receipts**: logs/events 패턴이 고정이므로 미리 알 수 있다
- **Nonce**: 단순 증가

→ **EVM을 서킷에서 돌릴 필요가 없다.**

### 증명 시간 비교 (예상)

```
풀 EVM 서킷:                ~수백만 사이클  (EVM 인터프리터 + MPT + keccak)
앱 특화 서킷 (네이티브 Rust):  ~수만 사이클   (앱 로직 + 증분 MPT 업데이트)
```

서명 검증(secp256k1)을 제외하면 **수십~수백 배 개선** 가능.

---

## 2. 아키텍처 개요

### 기존 (evm-l2)

```
시퀀서:  EVM으로 블록 실행 → state_root 계산
서킷:   동일한 EVM을 재실행 → state_root 검증 (느림)
```

### 앱 특화

```
시퀀서:  EVM으로 블록 실행 → state_root 계산 (변경 없음)
서킷:   앱 로직을 네이티브 Rust로 직접 계산 → state_root 검증 (빠름)
```

**시퀀서는 변경하지 않는다.** 기존 EVM + Solidity 컨트랙트 그대로 유지.
서킷만 앱에 맞게 경량화한다.

### 서킷이 증명하는 것

1. **트랜잭션 유효성**: 서명, 논스, 잔액 검증
2. **앱 로직 정확성**: 정해진 연산(swap 등)의 상태 전이가 수학적으로 맞음
3. **Gas 차감**: 연산별 고정 gas 차감이 정확함
4. **상태 루트**: 증분 MPT 업데이트로 새 state_root 계산

---

## 3. 서킷 입력/출력 설계

### 3.1 서킷 입력 (AppProgramInput)

시퀀서가 기존 `ProverInputData`에 추가로 **앱 특화 witness**를 제공한다.

```rust
/// 앱 특화 서킷의 입력.
/// 시퀀서가 EVM 실행 후 생성하는 witness를 포함한다.
pub struct AppProgramInput {
    // === 블록/트랜잭션 데이터 (기존과 동일) ===
    pub blocks: Vec<Block>,

    // === 상태 witness ===
    /// 이전 state root (이미 검증된 상태)
    pub prev_state_root: H256,
    /// 변경될 스토리지 슬롯들의 Merkle proof
    pub storage_proofs: Vec<StorageProof>,

    // === 앱 특화 데이터 ===
    /// 각 트랜잭션의 타입별 분류 (시퀀서가 EVM 실행 후 제공)
    pub classified_txs: Vec<ClassifiedTransaction>,

    // === L2 메타데이터 ===
    pub elasticity_multiplier: u64,
    pub fee_configs: Vec<FeeConfig>,
    pub blob_commitment: [u8; 48],
    pub blob_proof: [u8; 48],
}
```

### 3.2 Storage Proof (증분 상태 검증)

```rust
/// 특정 스토리지 슬롯의 Merkle proof.
/// 서킷은 이 proof를 검증하고, 변경 후 새 root를 계산한다.
pub struct StorageProof {
    /// 대상 계정 주소
    pub address: Address,
    /// 스토리지 슬롯 키
    pub slot: H256,
    /// 현재 값
    pub value: U256,
    /// state trie에서 이 계정까지의 Merkle 경로
    pub account_proof: Vec<Vec<u8>>,
    /// storage trie에서 이 슬롯까지의 Merkle 경로
    pub storage_proof: Vec<Vec<u8>>,
}
```

### 3.3 Classified Transaction (트랜잭션 분류)

```rust
/// 시퀀서가 EVM 실행 결과를 바탕으로 분류한 트랜잭션.
/// 서킷은 이 분류에 따라 앱 로직을 적용한다.
pub enum ClassifiedTransaction {
    /// L1→L2 입금 (privileged transaction)
    Deposit {
        tx_index: usize,
        to: Address,
        value: U256,
    },

    /// ETH 전송
    EthTransfer {
        tx_index: usize,
        from: Address,
        to: Address,
        value: U256,
        gas_used: u64,
    },

    /// 앱 특화 연산 (zk-dex: swap, addLiquidity 등)
    AppOperation {
        tx_index: usize,
        op_type: u8,         // 연산 종류 (앱별로 정의)
        from: Address,
        gas_used: u64,
        params: Vec<u8>,     // ABI 인코딩된 파라미터
    },

    /// 시스템 컨트랙트 호출 (bridge, messenger 등)
    SystemCall {
        tx_index: usize,
        target: Address,
        gas_used: u64,
    },
}
```

### 3.4 서킷 출력 (ProgramOutput)

**기존 `ProgramOutput`과 동일한 포맷**을 유지한다.
L1 OnChainProposer 컨트랙트의 검증 로직과 호환되어야 하기 때문.

```rust
pub struct ProgramOutput {
    pub initial_state_hash: H256,
    pub final_state_hash: H256,
    pub l1_out_messages_merkle_root: H256,
    pub l1_in_messages_rolling_hash: H256,
    pub l2_in_message_rolling_hashes: Vec<(u64, H256)>,
    pub blob_versioned_hash: H256,
    pub last_block_hash: H256,
    pub chain_id: U256,
    pub non_privileged_count: U256,
    pub balance_diffs: Vec<BalanceDiff>,
}
```

---

## 4. 서킷 실행 로직

### 4.1 전체 흐름

```
1. 입력 역직렬화
2. prev_state_root 검증 (storage proofs와 대조)
3. 각 블록에 대해:
   a. 블록 헤더 검증 (parent hash, timestamp, gas limit 등)
   b. 각 트랜잭션에 대해:
      - 서명 검증 (secp256k1)
      - 논스 검증 및 증가
      - 트랜잭션 분류에 따라 앱 로직 실행
      - Gas 차감 (고정값)
      - Receipt 생성 (고정 패턴)
   c. 상태 업데이트 (증분 MPT)
4. 최종 state_root 계산
5. 메시지 digest 계산 (입출금)
6. ProgramOutput 생성 및 커밋
```

### 4.2 앱 로직 실행 (zk-dex 예시)

```rust
/// zk-dex swap 연산의 서킷 내 실행.
/// EVM 없이 네이티브 Rust로 상태 전이를 계산한다.
fn execute_swap(
    state: &mut AppState,
    from: Address,
    token_in: Address,
    token_out: Address,
    amount_in: U256,
) -> Result<SwapResult, AppError> {
    // 1. 풀 상태 읽기 (storage proof로 검증됨)
    let reserve_in = state.get_reserve(token_in)?;
    let reserve_out = state.get_reserve(token_out)?;

    // 2. Constant product formula (x * y = k)
    // EVM에서 Solidity로 계산하든, 여기서 Rust로 계산하든 수학은 동일
    let amount_out = (reserve_out * amount_in) / (reserve_in + amount_in);

    // 3. 상태 업데이트
    state.set_reserve(token_in, reserve_in + amount_in)?;
    state.set_reserve(token_out, reserve_out - amount_out)?;
    state.transfer_token(token_out, from, amount_out)?;

    Ok(SwapResult { amount_out })
}
```

### 4.3 Gas 처리

```rust
/// 연산별 고정 gas 비용.
/// 시퀀서의 EVM 실행 결과와 일치해야 한다.
/// (Solidity 컨트랙트의 gas 소모량을 사전에 측정하여 결정)
fn get_fixed_gas(op_type: AppOperation) -> u64 {
    match op_type {
        AppOperation::Swap => 150_000,
        AppOperation::AddLiquidity => 200_000,
        AppOperation::RemoveLiquidity => 180_000,
        AppOperation::Transfer => 65_000,
        // ...
    }
}

/// Gas 차감을 상태에 반영.
fn apply_gas_deduction(
    state: &mut AppState,
    from: Address,
    gas_used: u64,
    gas_price: u64,
) {
    let fee = U256::from(gas_used) * U256::from(gas_price);
    state.deduct_balance(from, fee);
    // EIP-1559: base_fee는 burn, priority_fee는 coinbase로
    // 앱 특화 L2에서는 단순화할 수 있음
}
```

### 4.4 Receipt 생성

```rust
/// 앱 연산별 고정 로그 패턴.
/// EVM의 LOG 옵코드가 생성하는 것과 동일한 결과를 직접 구성한다.
fn generate_swap_logs(
    from: Address,
    token_in: Address,
    token_out: Address,
    amount_in: U256,
    amount_out: U256,
    pool_address: Address,
) -> Vec<Log> {
    vec![
        // Transfer(from, pool, amountIn) — ERC20 Transfer event
        Log {
            address: token_in,
            topics: vec![
                TRANSFER_EVENT_TOPIC,  // keccak256("Transfer(address,address,uint256)")
                H256::from(from),
                H256::from(pool_address),
            ],
            data: amount_in.to_be_bytes().into(),
        },
        // Transfer(pool, from, amountOut)
        Log {
            address: token_out,
            topics: vec![
                TRANSFER_EVENT_TOPIC,
                H256::from(pool_address),
                H256::from(from),
            ],
            data: amount_out.to_be_bytes().into(),
        },
        // Swap event
        Log {
            address: pool_address,
            topics: vec![SWAP_EVENT_TOPIC, H256::from(from)],
            data: encode_swap_data(amount_in, amount_out),
        },
    ]
}
```

### 4.5 증분 MPT 업데이트

```rust
/// 서킷 내에서 상태를 증분 업데이트한다.
/// 전체 MPT를 재구축하지 않고, 변경된 슬롯의 경로만 재해싱한다.
fn incremental_state_update(
    prev_root: H256,
    storage_proofs: &[StorageProof],
    updates: &[(Address, H256, U256)],  // (account, slot, new_value)
) -> H256 {
    // 1. 각 storage proof를 prev_root에 대해 검증
    for proof in storage_proofs {
        verify_merkle_proof(prev_root, proof).unwrap();
    }

    // 2. 변경된 값으로 경로 재해싱
    //    MPT 깊이 ~15 → keccak256 ~15회 per slot
    let new_root = apply_updates_to_trie(prev_root, storage_proofs, updates);

    new_root
}
```

---

## 5. 시퀀서 측 변경

시퀀서는 기존 EVM 실행을 유지하면서, **앱 특화 witness를 추가로 생성**한다.

### 5.1 Witness 생성기

```rust
/// 시퀀서가 EVM 실행 후, 서킷에 필요한 witness를 생성한다.
pub struct AppWitnessGenerator {
    app_config: AppConfig,
}

impl AppWitnessGenerator {
    /// EVM 실행 결과로부터 앱 특화 witness를 생성.
    pub fn generate(
        &self,
        blocks: &[Block],
        execution_result: &ExecutionResult,
        state_db: &dyn VmDatabase,
    ) -> AppWitness {
        let classified_txs = self.classify_transactions(blocks);
        let storage_proofs = self.collect_storage_proofs(
            &classified_txs,
            state_db,
        );

        AppWitness {
            classified_txs,
            storage_proofs,
        }
    }

    /// 트랜잭션을 앱 연산 타입으로 분류.
    /// function selector (calldata[:4])로 판별.
    fn classify_transactions(&self, blocks: &[Block]) -> Vec<ClassifiedTransaction> {
        // ...
    }

    /// 변경되는 스토리지 슬롯의 Merkle proof 수집.
    fn collect_storage_proofs(
        &self,
        txs: &[ClassifiedTransaction],
        state_db: &dyn VmDatabase,
    ) -> Vec<StorageProof> {
        // 앱 연산 타입별로 어떤 슬롯이 변경되는지 미리 알 수 있음
        // 예: swap → reserves[tokenA], reserves[tokenB], balances[user]
        // ...
    }
}
```

### 5.2 Gas 일치 보장

정해진 동작이므로 연산별 gas 소모량은 고정이다. Solidity 컨트랙트를 gas-deterministic하게 설계한다:
- 조건 분기를 최소화하여 gas 소모가 항상 동일하도록 작성
- 배포 전에 각 연산의 gas 소모량을 측정하여 서킷의 고정값과 일치시킴

### 5.3 Gas 일치 검증 도구

```rust
/// 배포 전에 각 앱 연산의 gas 소모량을 측정하는 테스트 유틸리티.
/// 고정 gas 값이 EVM 실행과 일치하는지 검증한다.
#[cfg(test)]
fn verify_gas_consistency() {
    let evm_gas = execute_swap_on_evm(params);
    let circuit_gas = get_fixed_gas(AppOperation::Swap);
    assert_eq!(evm_gas, circuit_gas,
        "Gas mismatch: EVM used {} but circuit expects {}", evm_gas, circuit_gas);
}
```

---

## 6. 시퀀서-서킷 일치성 보장

### 6.1 핵심 원칙

서킷이 EVM 없이 계산한 결과가 시퀀서의 EVM 실행 결과와 **동일한 state_root**를 만들어야 한다.

```
시퀀서 (EVM):   state_root_A → [EVM 실행] → state_root_B
서킷 (앱 로직): state_root_A → [앱 로직]  → state_root_B'

state_root_B == state_root_B' 여야 함
```

### 6.2 일치해야 하는 항목

| 항목 | 일치 방법 |
|------|-----------|
| 계정 잔액 (balance) | 앱 로직이 동일한 금액을 이동 |
| 계정 논스 (nonce) | 트랜잭션마다 +1 |
| 스토리지 슬롯 | 앱 로직이 동일한 값을 계산 (같은 수학) |
| 코드 해시 (code_hash) | 컨트랙트 생성 없으므로 변경 없음 |
| 스토리지 루트 | 동일한 슬롯 변경 → 동일한 MPT → 동일한 루트 |
| 가스 차감 | 고정 gas × gas_price = 동일한 잔액 변화 |

### 6.3 일치하기 어려운 항목과 대응

| 항목 | 어려운 이유 | 대응 |
|------|-------------|------|
| Receipts root | EVM 실행 없이 정확한 logs 재현 필요 | 앱 연산별 로그 패턴 하드코딩 |
| Gas refund | EVM의 refund 메커니즘이 복잡 | 앱 특화 L2에서는 refund 없이 설계 가능 |
| Cold/warm 스토리지 가격 | EVM 실행 순서에 의존 | 고정 gas로 대체 |
| L2 fee 분배 | base/operator/L1 fee vault 분배 로직 | 시퀀서 로직과 동일하게 구현 |

---

## 7. 보안 모델

### 7.1 서킷이 보장하는 것 (ZK proof)

- 모든 트랜잭션의 서명이 유효하다
- 각 트랜잭션의 상태 전이가 앱 규칙에 따라 정확하다
- 최종 state_root가 상태 변경을 정확히 반영한다
- 입출금 메시지가 정확하게 처리되었다

### 7.2 서킷이 보장하지 않는 것

- EVM 바이트코드 실행의 정확성 (EVM을 안 돌리므로)
- 임의의 스마트 컨트랙트 로직 (정해진 앱 연산만)
- EVM 가스 규칙과의 100% 호환 (고정 gas 모델 사용)

### 7.3 보안 가정

- **앱 컨트랙트가 변경되지 않음**: 서킷의 앱 로직이 Solidity 컨트랙트와 일치해야 하므로, 컨트랙트 업그레이드 시 서킷도 업데이트 필요
- **허용된 연산만 실행됨**: 시퀀서가 정해진 연산 외의 트랜잭션을 포함시키면 서킷이 거부
- **고정 gas가 정확함**: 배포 전 테스트로 EVM gas와 일치 검증 필수

---

## 8. 구현 구조

### 8.1 크레이트 구조

```
crates/guest-program/
├── src/
│   ├── common/
│   │   ├── mod.rs
│   │   ├── execution.rs         # 기존 (evm-l2용, 변경 없음)
│   │   └── app_execution.rs     # NEW: 앱 특화 실행 엔진
│   ├── l2/
│   │   ├── program.rs           # 기존 execution_program() (변경 없음)
│   │   └── input.rs             # 기존 ProgramInput (변경 없음)
│   └── programs/
│       ├── evm_l2.rs            # 기존 (변경 없음)
│       ├── zk_dex/
│       │   ├── mod.rs           # GuestProgram trait 구현
│       │   ├── circuit.rs       # NEW: 서킷 실행 로직 (swap, liquidity 등)
│       │   ├── gas.rs           # NEW: 연산별 고정 gas
│       │   └── receipts.rs      # NEW: 연산별 receipt 생성
│       └── tokamon/
│           ├── mod.rs
│           ├── circuit.rs       # NEW: 게임 로직
│           ├── gas.rs
│           └── receipts.rs
├── bin/
│   ├── sp1/src/main.rs          # 기존 evm-l2 (변경 없음)
│   ├── sp1-zk-dex/src/main.rs   # REWRITE: 앱 특화 서킷
│   └── sp1-tokamon/src/main.rs  # REWRITE: 앱 특화 서킷
```

### 8.2 앱 특화 서킷 바이너리 (SP1)

```rust
// crates/guest-program/bin/sp1-zk-dex/src/main.rs

#![no_main]
sp1_zkvm::entrypoint!(main);

pub fn main() {
    // 1. 입력 읽기
    let input = sp1_zkvm::io::read_vec();
    let input = rkyv::from_bytes::<AppProgramInput>(&input).unwrap();

    // 2. 이전 상태 검증 (storage proofs)
    verify_storage_proofs(&input.prev_state_root, &input.storage_proofs);

    // 3. 앱 로직 실행 (EVM 없이)
    let mut state = AppState::from_proofs(&input.storage_proofs);

    for tx in &input.classified_txs {
        match tx {
            ClassifiedTransaction::Deposit { to, value, .. } => {
                state.credit_balance(*to, *value);
            }
            ClassifiedTransaction::EthTransfer { from, to, value, gas_used, .. } => {
                state.transfer_eth(*from, *to, *value);
                state.deduct_gas(*from, *gas_used, gas_price);
            }
            ClassifiedTransaction::AppOperation { op_type, from, params, gas_used, .. } => {
                execute_app_operation(&mut state, *op_type, *from, params);
                state.deduct_gas(*from, *gas_used, gas_price);
            }
            // ...
        }
    }

    // 4. 새 state_root 계산 (증분 MPT 업데이트)
    let new_state_root = state.compute_new_root();

    // 5. 출력 커밋
    let output = ProgramOutput {
        initial_state_hash: input.prev_state_root,
        final_state_hash: new_state_root,
        // ... 나머지 필드
    };
    sp1_zkvm::io::commit_slice(&output.encode());
}
```

### 8.3 앱별 연산 정의 (zk-dex)

```rust
// crates/guest-program/src/programs/zk_dex/circuit.rs

/// zk-dex에서 지원하는 연산 타입.
pub enum DexOperation {
    /// Swap: token_in → token_out (constant product)
    Swap {
        token_in: Address,
        token_out: Address,
        amount_in: U256,
        min_amount_out: U256,
    },
    /// 유동성 추가
    AddLiquidity {
        token_a: Address,
        token_b: Address,
        amount_a: U256,
        amount_b: U256,
    },
    /// 유동성 제거
    RemoveLiquidity {
        token_a: Address,
        token_b: Address,
        lp_amount: U256,
    },
    /// ERC20 토큰 전송
    TokenTransfer {
        token: Address,
        to: Address,
        amount: U256,
    },
}

/// 각 연산을 서킷 내에서 실행.
pub fn execute_dex_operation(
    state: &mut AppState,
    from: Address,
    op: DexOperation,
) -> Result<(), AppError> {
    match op {
        DexOperation::Swap { token_in, token_out, amount_in, min_amount_out } => {
            // Constant product AMM: x * y = k
            let reserve_in = state.get_storage(POOL_ADDRESS, reserve_slot(token_in))?;
            let reserve_out = state.get_storage(POOL_ADDRESS, reserve_slot(token_out))?;

            // fee = 0.3% (Uniswap V2 style)
            let amount_in_with_fee = amount_in * 997;
            let numerator = amount_in_with_fee * reserve_out;
            let denominator = reserve_in * 1000 + amount_in_with_fee;
            let amount_out = numerator / denominator;

            assert!(amount_out >= min_amount_out, "Slippage exceeded");

            // 상태 업데이트
            state.set_storage(POOL_ADDRESS, reserve_slot(token_in), reserve_in + amount_in)?;
            state.set_storage(POOL_ADDRESS, reserve_slot(token_out), reserve_out - amount_out)?;
            state.transfer_erc20(token_in, from, POOL_ADDRESS, amount_in)?;
            state.transfer_erc20(token_out, POOL_ADDRESS, from, amount_out)?;

            Ok(())
        }
        // ... 나머지 연산들
    }
}
```

---

## 9. 구현 단계

### Phase 1: 인프라 (서킷 입출력 타입 정의)

1. `AppProgramInput`, `StorageProof`, `ClassifiedTransaction` 타입 정의
2. `AppState` 구현 (storage proof 기반 상태 관리)
3. 증분 MPT 업데이트 로직 구현
4. 기존 `ProgramOutput`과의 호환성 확인

### Phase 2: zk-dex 서킷 구현

1. DEX 연산별 실행 로직 (swap, addLiquidity, removeLiquidity, transfer)
2. 연산별 고정 gas 매핑
3. 연산별 receipt/log 생성
4. SP1 바이너리 작성
5. 유닛 테스트 (연산 정확성, gas 일치)

### Phase 3: 시퀀서 witness 생성

1. `AppWitnessGenerator` 구현
2. 트랜잭션 분류 로직 (function selector 기반)
3. Storage proof 수집 로직
4. ProverInputData 확장

### Phase 4: 통합 테스트

1. 시퀀서(EVM) 실행 결과와 서킷(앱 로직) 실행 결과의 state_root 일치 검증
2. Gas 일치 검증
3. Receipt 일치 검증
4. E2E: 시퀀서 → witness → 서킷 → proof → L1 검증

### Phase 5: tokamon 서킷 구현

1. Phase 2와 동일한 패턴으로 tokamon 게임 로직 구현

---

## 10. 트레이드오프 정리

| 장점 | 단점 |
|------|------|
| 증명 시간 수십~수백 배 감소 | 앱 컨트랙트 변경 시 서킷도 업데이트 필요 |
| 서킷 코드가 단순하고 감사하기 쉬움 | 앱별로 서킷을 작성해야 함 |
| EVM 인터프리터 버그의 영향 없음 | Gas/receipt 일치 검증 필요 (배포 전) |
| 시퀀서 변경 없음 | 시퀀서에 witness 생성 로직 추가 필요 |

---

## 11. 결정된 사항

1. **Gas 모델**: 고정 gas. 정해진 동작이므로 연산별 gas가 예측 가능하다.
2. **Receipts**: 고정 패턴. 정해진 동작이므로 logs/events가 예측 가능하다.
3. **시퀀서**: EVM 유지. 서킷만 앱 특화로 경량화한다.

## 12. 미해결 질문

1. **Solidity 컨트랙트 설계**: gas-deterministic하게 작성할 것인지? (조건 분기에 따라 gas가 미세하게 달라질 수 있음)
2. **컨트랙트 업그레이드 시**: 서킷 업데이트 절차와 전환 기간 처리?
3. **앱별 첫 번째 대상**: zk-dex부터 구현할 것인지?
