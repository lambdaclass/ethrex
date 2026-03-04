# 앱 특화 서킷 개발 계획

설계 문서: `11-app-specific-circuit-design.md`

## 핵심 원칙

- **정해진 동작 → 모든 게 예측 가능 → EVM 없이 직접 계산**
- Gas: 연산별 고정값 (정해진 동작이니까)
- Receipts: 연산별 고정 패턴 (정해진 동작이니까)
- 시퀀서: EVM 유지 (변경 없음). 서킷만 경량화

---

## Phase 1: 공통 인프라

> 앱에 무관한 공통 모듈. 모든 앱 특화 서킷이 공유한다.

### 1.1 앱 서킷 입력 타입

**파일**: `crates/guest-program/src/common/app_types.rs` (NEW)

```rust
use ethrex_common::{Address, H256, U256};
use ethrex_common::types::Block;

/// 앱 특화 서킷의 입력.
#[derive(rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub struct AppProgramInput {
    /// 실행할 블록들
    pub blocks: Vec<Block>,
    /// 이전 state root (L1에서 이미 검증된 상태)
    pub prev_state_root: H256,
    /// 변경될 스토리지 슬롯들의 Merkle proof
    pub storage_proofs: Vec<StorageProof>,
    /// L2 메타데이터
    pub elasticity_multiplier: u64,
    pub fee_configs: Vec<FeeConfig>,
    pub blob_commitment: [u8; 48],
    pub blob_proof: [u8; 48],
    pub chain_id: u64,
}

/// 특정 스토리지 슬롯의 Merkle proof
#[derive(rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub struct StorageProof {
    pub address: Address,
    pub slot: H256,
    pub value: U256,
    /// state trie → account 경로
    pub account_proof: Vec<Vec<u8>>,
    /// storage trie → slot 경로
    pub storage_proof: Vec<Vec<u8>>,
    /// 계정 정보 (nonce, balance, code_hash, storage_root)
    pub account_state: AccountState,
}
```

### 1.2 앱 상태 관리

**파일**: `crates/guest-program/src/common/app_state.rs` (NEW)

```rust
/// Storage proof 기반 상태 관리.
/// 서킷 내에서 상태를 읽고 수정하며, 최종 state_root를 계산한다.
pub struct AppState {
    /// 이전 state root
    prev_state_root: H256,
    /// 계정별 상태 (nonce, balance, storage)
    accounts: BTreeMap<Address, AccountState>,
    /// 변경된 스토리지 슬롯 추적
    dirty_slots: BTreeMap<Address, BTreeMap<H256, U256>>,
    /// 원본 Merkle proof (state root 재계산용)
    proofs: Vec<StorageProof>,
}

impl AppState {
    pub fn from_proofs(prev_root: H256, proofs: Vec<StorageProof>) -> Self { ... }
    pub fn get_balance(&self, addr: Address) -> U256 { ... }
    pub fn set_balance(&mut self, addr: Address, val: U256) { ... }
    pub fn get_nonce(&self, addr: Address) -> u64 { ... }
    pub fn increment_nonce(&mut self, addr: Address) { ... }
    pub fn get_storage(&self, addr: Address, slot: H256) -> U256 { ... }
    pub fn set_storage(&mut self, addr: Address, slot: H256, val: U256) { ... }
    pub fn transfer_eth(&mut self, from: Address, to: Address, val: U256) -> Result<()> { ... }

    /// 변경된 슬롯만 반영하여 새 state root 계산 (증분 MPT 업데이트)
    pub fn compute_new_state_root(&self) -> H256 { ... }
}
```

### 1.3 증분 MPT 업데이트

**파일**: `crates/guest-program/src/common/incremental_mpt.rs` (NEW)

```rust
/// Merkle proof 검증: 이 값이 실제로 이 root에 속하는지 확인
pub fn verify_account_proof(root: H256, proof: &StorageProof) -> Result<()> { ... }

/// 변경된 값을 적용하여 새 root 계산
/// MPT 경로만 재해싱 (전체 트리 재구축 아님)
pub fn compute_updated_root(
    prev_root: H256,
    account_updates: &[(Address, AccountState)],
    storage_updates: &[(Address, Vec<(H256, U256)>)],
    proofs: &[StorageProof],
) -> H256 { ... }
```

### 1.4 앱 서킷 실행 엔진

**파일**: `crates/guest-program/src/common/app_execution.rs` (NEW)

```rust
/// 앱 특화 서킷의 공통 실행 프레임워크.
/// 각 앱은 AppCircuit 트레이트를 구현하여 자신만의 로직을 정의한다.
pub trait AppCircuit {
    /// 트랜잭션을 분류하여 앱 연산으로 변환
    fn classify_tx(&self, tx: &Transaction) -> Result<AppOperation>;

    /// 앱 연산을 실행하고 상태를 업데이트
    fn execute_operation(
        &self,
        state: &mut AppState,
        from: Address,
        op: &AppOperation,
    ) -> Result<OperationResult>;

    /// 연산별 고정 gas 반환
    fn gas_cost(&self, op: &AppOperation) -> u64;

    /// 연산별 고정 로그 패턴 생성
    fn generate_logs(&self, op: &AppOperation, result: &OperationResult) -> Vec<Log>;
}

/// 앱 연산 (앱별로 다르게 정의)
pub struct AppOperation {
    pub op_type: u8,
    pub params: Vec<u8>,  // ABI 디코딩된 파라미터
}

/// 연산 실행 결과
pub struct OperationResult {
    pub success: bool,
    pub state_changes: Vec<(Address, H256, U256)>,  // 변경된 슬롯들
}

/// 앱 특화 서킷의 메인 실행 함수.
/// 모든 앱이 공유하는 공통 로직.
pub fn execute_app_circuit<C: AppCircuit>(
    circuit: &C,
    input: AppProgramInput,
) -> Result<ProgramOutput> {
    // 1. Storage proof 검증
    let mut state = AppState::from_proofs(input.prev_state_root, input.storage_proofs);
    for proof in &state.proofs {
        verify_account_proof(input.prev_state_root, proof)?;
    }

    // 2. 각 블록/트랜잭션 실행
    let mut all_receipts = Vec::new();
    let mut non_privileged_count = 0u64;

    for block in &input.blocks {
        let mut block_receipts = Vec::new();
        let mut cumulative_gas = 0u64;

        for tx in &block.body.transactions {
            if tx.is_privileged() {
                // 입금 처리 (시스템 트랜잭션) — 공통
                handle_privileged_tx(&mut state, tx)?;
            } else {
                // 서명 검증 — 공통
                verify_signature(tx)?;

                // 논스 검증 및 증가 — 공통
                verify_and_increment_nonce(&mut state, tx)?;

                // ETH 전송 — 공통 (calldata 없고 Call인 경우)
                if tx.data().is_empty() && tx.to().is_call() {
                    let to = tx.to().address().unwrap();
                    state.transfer_eth(tx.sender(), to, tx.value())?;
                    let gas = ETH_TRANSFER_GAS; // 21,000 (고정)
                    apply_gas_deduction(&mut state, tx, gas, &block.header)?;
                    cumulative_gas += gas;
                    block_receipts.push(Receipt {
                        succeeded: true,
                        cumulative_gas_used: cumulative_gas,
                        logs: vec![], // ETH 전송은 이벤트 없음
                        ..Default::default()
                    });
                    non_privileged_count += 1;
                    continue;
                }

                // L1 출금 (L2→L1) — 공통 (CommonBridgeL2 호출)
                if tx.to().address() == Some(COMMON_BRIDGE_L2_ADDRESS) {
                    handle_withdrawal(&mut state, tx)?;
                    let gas = WITHDRAWAL_GAS; // 고정
                    apply_gas_deduction(&mut state, tx, gas, &block.header)?;
                    cumulative_gas += gas;
                    block_receipts.push(generate_withdrawal_receipt(tx, cumulative_gas));
                    non_privileged_count += 1;
                    continue;
                }

                // 기타 시스템 컨트랙트 (L1Messenger, FeeTokenRegistry 등) — 공통
                if is_system_contract(tx.to().address()) {
                    handle_system_call(&mut state, tx)?;
                    let gas = get_system_call_gas(tx.to().address());
                    apply_gas_deduction(&mut state, tx, gas, &block.header)?;
                    cumulative_gas += gas;
                    block_receipts.push(generate_system_receipt(tx, cumulative_gas));
                    non_privileged_count += 1;
                    continue;
                }

                // 앱 연산 분류 및 실행 — 앱별
                let op = circuit.classify_tx(tx)?;
                let result = circuit.execute_operation(&mut state, tx.sender(), &op)?;

                // Gas 차감 (고정값)
                let gas = circuit.gas_cost(&op);
                apply_gas_deduction(&mut state, tx, gas, &block.header)?;
                cumulative_gas += gas;

                // Receipt 생성 (고정 패턴)
                let logs = circuit.generate_logs(&op, &result);
                block_receipts.push(Receipt {
                    succeeded: result.success,
                    cumulative_gas_used: cumulative_gas,
                    logs,
                    ..Default::default()
                });

                non_privileged_count += 1;
            }
        }
        all_receipts.push(block_receipts);
    }

    // 3. 최종 state root 계산 (증분 MPT)
    let final_state_hash = state.compute_new_state_root();

    // 4. 메시지 처리 (입출금)
    let batch_messages = get_batch_messages(&input.blocks, &all_receipts, input.chain_id);
    let message_digests = compute_message_digests(&batch_messages)?;
    let balance_diffs = get_balance_diffs(&batch_messages.l2_out_messages);

    // 5. Blob 검증
    let blob_versioned_hash = verify_blob(
        &input.blocks, &input.fee_configs,
        input.blob_commitment, input.blob_proof,
    )?;

    // 6. 출력
    Ok(ProgramOutput {
        initial_state_hash: input.prev_state_root,
        final_state_hash,
        l1_out_messages_merkle_root: message_digests.l1_out_messages_merkle_root,
        l1_in_messages_rolling_hash: message_digests.l1_in_messages_rolling_hash,
        l2_in_message_rolling_hashes: message_digests.l2_in_message_rolling_hashes,
        blob_versioned_hash,
        last_block_hash: input.blocks.last().unwrap().header.hash(),
        chain_id: input.chain_id.into(),
        non_privileged_count: non_privileged_count.into(),
        balance_diffs,
    })
}
```

### 1.5 common/mod.rs 업데이트

**파일**: `crates/guest-program/src/common/mod.rs` (EDIT)

```rust
mod error;
mod execution;
pub mod app_types;        // NEW
pub mod app_state;        // NEW
pub mod app_execution;    // NEW
pub mod incremental_mpt;  // NEW

pub use error::ExecutionError;
pub use execution::{BatchExecutionResult, execute_blocks};
```

### Phase 1 파일 요약

| # | 파일 | 작업 |
|---|------|------|
| 1 | `src/common/app_types.rs` | CREATE — 입력 타입 |
| 2 | `src/common/app_state.rs` | CREATE — 상태 관리 |
| 3 | `src/common/incremental_mpt.rs` | CREATE — 증분 MPT |
| 4 | `src/common/app_execution.rs` | CREATE — 실행 엔진 + AppCircuit 트레이트 |
| 5 | `src/common/mod.rs` | EDIT — 모듈 추가 |

### Phase 1 검증

```bash
cargo check -p ethrex-guest-program
cargo test -p ethrex-guest-program  # 기존 테스트 통과 확인
```

---

## Phase 2: zk-dex 서킷 구현

### 2.1 DEX 연산 정의

**파일**: `crates/guest-program/src/programs/zk_dex/operations.rs` (NEW, 기존 types.rs 대체)

```rust
/// zk-dex 연산 타입 (function selector로 판별)
pub enum DexOperationType {
    Swap = 0,
    AddLiquidity = 1,
    RemoveLiquidity = 2,
    TokenTransfer = 3,
}

/// zk-dex 연산 파라미터 (ABI 디코딩)
pub enum DexParams {
    Swap {
        token_in: Address,
        token_out: Address,
        amount_in: U256,
        min_amount_out: U256,
    },
    AddLiquidity {
        token_a: Address,
        token_b: Address,
        amount_a: U256,
        amount_b: U256,
    },
    RemoveLiquidity {
        token_a: Address,
        token_b: Address,
        lp_amount: U256,
    },
    TokenTransfer {
        token: Address,
        to: Address,
        amount: U256,
    },
}
```

### 2.2 DEX 서킷 구현

**파일**: `crates/guest-program/src/programs/zk_dex/circuit.rs` (NEW, 기존 execution.rs 대체)

```rust
use crate::common::app_execution::AppCircuit;

pub struct ZkDexCircuit {
    /// DEX 컨트랙트 주소 (고정)
    pub pool_address: Address,
    /// 허용된 토큰 컨트랙트 주소들
    pub allowed_tokens: Vec<Address>,
}

impl AppCircuit for ZkDexCircuit {
    fn classify_tx(&self, tx: &Transaction) -> Result<AppOperation> {
        let selector = &tx.data()[..4];
        match selector {
            SWAP_SELECTOR => Ok(/* Swap 파싱 */),
            ADD_LIQUIDITY_SELECTOR => Ok(/* AddLiquidity 파싱 */),
            // ...
            _ => Err(AppError::UnknownOperation),
        }
    }

    fn execute_operation(&self, state: &mut AppState, from: Address, op: &AppOperation) -> Result<OperationResult> {
        match op.decode::<DexParams>()? {
            DexParams::Swap { token_in, token_out, amount_in, min_amount_out } => {
                // Constant product: x * y = k
                let reserve_in = state.get_storage(self.pool_address, reserve_slot(token_in))?;
                let reserve_out = state.get_storage(self.pool_address, reserve_slot(token_out))?;

                let amount_in_with_fee = amount_in * 997;
                let amount_out = (amount_in_with_fee * reserve_out) / (reserve_in * 1000 + amount_in_with_fee);

                assert!(amount_out >= min_amount_out);

                state.set_storage(self.pool_address, reserve_slot(token_in), reserve_in + amount_in)?;
                state.set_storage(self.pool_address, reserve_slot(token_out), reserve_out - amount_out)?;

                Ok(OperationResult { success: true, .. })
            }
            // ... 나머지 연산들
        }
    }

    fn gas_cost(&self, op: &AppOperation) -> u64 {
        match op.op_type {
            0 => 150_000,   // Swap
            1 => 200_000,   // AddLiquidity
            2 => 180_000,   // RemoveLiquidity
            3 => 65_000,    // TokenTransfer
            _ => unreachable!(),
        }
    }

    fn generate_logs(&self, op: &AppOperation, result: &OperationResult) -> Vec<Log> {
        // 연산별 고정 이벤트 패턴 생성
        // ...
    }
}
```

### 2.3 GuestProgram 트레이트 업데이트

**파일**: `crates/guest-program/src/programs/zk_dex/mod.rs` (EDIT)

```rust
// 기존 모듈 제거
// pub mod execution;  // 삭제 (keccak 기반)
// pub mod types;      // 삭제 (DexProgramInput/Output)

// 새 모듈
pub mod circuit;       // NEW: 앱 서킷 로직
pub mod operations;    // NEW: 연산 정의

pub struct ZkDexGuestProgram;

impl GuestProgram for ZkDexGuestProgram {
    fn program_id(&self) -> &str { "zk-dex" }
    fn program_type_id(&self) -> u8 { 2 }

    // resource_limits를 evm-l2와 동일하게 변경 (풀 EVM은 아니지만 MPT 연산 포함)
    fn resource_limits(&self) -> ResourceLimits {
        ResourceLimits {
            max_input_bytes: Some(256 * 1024 * 1024), // 256 MB
            max_proving_duration: Some(std::time::Duration::from_secs(3600)),
        }
    }
    // ...
}
```

### 2.4 SP1 바이너리 재작성

**파일**: `crates/guest-program/bin/sp1-zk-dex/src/main.rs` (REWRITE)

```rust
#![no_main]
sp1_zkvm::entrypoint!(main);

use ethrex_guest_program::common::app_types::AppProgramInput;
use ethrex_guest_program::common::app_execution::execute_app_circuit;
use ethrex_guest_program::programs::zk_dex::circuit::ZkDexCircuit;

pub fn main() {
    let input = sp1_zkvm::io::read_vec();
    let input = rkyv::from_bytes::<AppProgramInput, rkyv::rancor::Error>(&input).unwrap();

    let circuit = ZkDexCircuit {
        pool_address: DEX_POOL_ADDRESS,   // 빌드 타임 상수
        allowed_tokens: vec![/* ... */],
    };

    let output = execute_app_circuit(&circuit, input).unwrap();
    sp1_zkvm::io::commit_slice(&output.encode());
}
```

### 2.5 SP1 Cargo.toml 업데이트

**파일**: `crates/guest-program/bin/sp1-zk-dex/Cargo.toml` (EDIT)

evm-l2 바이너리 (`bin/sp1/Cargo.toml`)의 의존성을 참고하되,
`ethrex-vm` 대신 앱 특화 모듈만 사용:

```toml
[dependencies]
sp1-zkvm = "5.0.8"
rkyv = "0.8.10"
ethrex-guest-program = { path = "../../", default-features = false, features = ["sp1-cycles"] }
ethrex-common = { path = "../../../../common", default-features = false }
# ethrex-vm은 불필요 (EVM을 안 쓰니까)

[patch.crates-io]
# SP1 패치 (keccak, secp256k1 등 — 서명 검증과 MPT에 필요)
tiny-keccak = { git = "https://github.com/sp1-patches/tiny-keccak", ... }
secp256k1 = { git = "https://github.com/sp1-patches/rust-secp256k1", ... }
# sha2, sha3 등은 EVM 전용이므로 불필요할 수 있음
```

### 2.6 기존 파일 삭제

| 파일 | 이유 |
|------|------|
| `src/programs/zk_dex/execution.rs` | keccak 기반 → circuit.rs로 대체 |
| `src/programs/zk_dex/types.rs` | DexProgramInput/Output → AppProgramInput + ProgramOutput로 대체 |

### Phase 2 파일 요약

| # | 파일 | 작업 |
|---|------|------|
| 1 | `src/programs/zk_dex/operations.rs` | CREATE — 연산 타입 정의 |
| 2 | `src/programs/zk_dex/circuit.rs` | CREATE — AppCircuit 구현 |
| 3 | `src/programs/zk_dex/mod.rs` | EDIT — 모듈 교체 |
| 4 | `src/programs/zk_dex/execution.rs` | DELETE |
| 5 | `src/programs/zk_dex/types.rs` | DELETE |
| 6 | `bin/sp1-zk-dex/src/main.rs` | REWRITE |
| 7 | `bin/sp1-zk-dex/Cargo.toml` | EDIT |

### Phase 2 검증

```bash
cargo test -p ethrex-guest-program  # 새 서킷 유닛 테스트
cargo check -p ethrex-guest-program  # 기존 evm-l2 깨지지 않음 확인
```

---

## Phase 3: 시퀀서 witness 생성

> 시퀀서가 EVM 실행 후 앱 서킷에 필요한 witness를 생성.

### 3.1 Witness 생성기

**파일**: `crates/l2/sequencer/app_witness.rs` (NEW)

```rust
pub struct AppWitnessGenerator {
    program_id: String,
    contract_addresses: Vec<Address>,  // 앱 컨트랙트 주소들
}

impl AppWitnessGenerator {
    /// EVM 실행 결과 + 상태 DB로부터 AppProgramInput 생성
    pub fn generate(
        &self,
        blocks: &[Block],
        state_db: &dyn Database,
    ) -> Result<AppProgramInput> {
        // 1. 변경된 스토리지 슬롯 수집
        //    (EVM 실행 후 state transitions에서 추출)
        let accessed_slots = collect_accessed_slots(blocks, &self.contract_addresses);

        // 2. 각 슬롯의 Merkle proof 생성
        let storage_proofs = accessed_slots.iter()
            .map(|(addr, slot)| state_db.get_proof(*addr, *slot))
            .collect();

        // 3. 이전 state root
        let prev_state_root = state_db.state_root();

        Ok(AppProgramInput {
            blocks: blocks.to_vec(),
            prev_state_root,
            storage_proofs,
            // ... 나머지 필드
        })
    }
}
```

### 3.2 ProverInputData 확장

**파일**: `crates/l2/common/src/prover.rs` (EDIT)

```rust
pub struct ProverInputData {
    // 기존 필드 유지
    pub blocks: Vec<Block>,
    pub execution_witness: ExecutionWitness,
    // ...

    // NEW: 앱 특화 서킷용 입력 (옵션)
    pub app_input: Option<AppProgramInput>,
}
```

### 3.3 Proof Coordinator 업데이트

**파일**: `crates/l2/sequencer/proof_coordinator.rs` (EDIT)

- `program_id`에 따라 `ExecutionWitness` 또는 `AppProgramInput` 중 적절한 것을 생성
- evm-l2: 기존과 동일 (ExecutionWitness)
- zk-dex/tokamon: AppWitnessGenerator 사용

### Phase 3 파일 요약

| # | 파일 | 작업 |
|---|------|------|
| 1 | `crates/l2/sequencer/app_witness.rs` | CREATE |
| 2 | `crates/l2/common/src/prover.rs` | EDIT |
| 3 | `crates/l2/sequencer/proof_coordinator.rs` | EDIT |

---

## Phase 4: 통합 테스트

### 4.1 State Root 일치 테스트

**파일**: `crates/guest-program/tests/app_circuit_integration.rs` (NEW)

```rust
/// 시퀀서(EVM)와 서킷(앱 로직)이 동일한 state_root를 계산하는지 검증.
#[test]
fn evm_and_circuit_produce_same_state_root() {
    // 1. 테스트 블록 생성 (swap 트랜잭션 포함)
    // 2. EVM으로 실행 → state_root_evm
    // 3. 앱 서킷으로 실행 → state_root_circuit
    // 4. assert_eq!(state_root_evm, state_root_circuit)
}
```

### 4.2 Gas 일치 테스트

```rust
/// EVM의 gas_used와 서킷의 고정 gas가 일치하는지 검증.
#[test]
fn gas_matches_evm_execution() {
    // 각 연산 타입별로:
    // 1. EVM으로 실행 → gas_used_evm
    // 2. circuit.gas_cost() → gas_fixed
    // 3. assert_eq!(gas_used_evm, gas_fixed)
}
```

### 4.3 Receipt 일치 테스트

```rust
/// EVM의 receipts와 서킷이 생성한 receipts가 일치하는지 검증.
#[test]
fn receipts_match_evm_execution() {
    // 1. EVM으로 실행 → receipts_evm
    // 2. circuit.generate_logs() → receipts_circuit
    // 3. assert_eq!(receipts_root(receipts_evm), receipts_root(receipts_circuit))
}
```

---

## Phase 5: tokamon 서킷 구현

Phase 2와 동일한 패턴. `AppCircuit` 트레이트를 tokamon용으로 구현.

### 파일

| # | 파일 | 작업 |
|---|------|------|
| 1 | `src/programs/tokamon/operations.rs` | CREATE |
| 2 | `src/programs/tokamon/circuit.rs` | CREATE |
| 3 | `src/programs/tokamon/mod.rs` | EDIT |
| 4 | `src/programs/tokamon/execution.rs` | DELETE |
| 5 | `src/programs/tokamon/types.rs` | DELETE |
| 6 | `bin/sp1-tokamon/src/main.rs` | REWRITE |
| 7 | `bin/sp1-tokamon/Cargo.toml` | EDIT |

---

## Phase 6: 벤치마크 및 정리

### 6.1 벤치마크

**파일**: `crates/l2/prover/src/bin/sp1_benchmark.rs` (EDIT or DELETE)

기존 벤치마크는 `DexProgramInput`을 사용하므로 `AppProgramInput` 기반으로 재작성.
evm-l2 vs 앱 특화 서킷의 **사이클 수 비교** 포함.

### 6.2 programs.toml 업데이트

**파일**: `crates/l2/programs.toml` (EDIT)

```toml
[programs.zk-dex]
type = "app-specific"
pool_address = "0x..."
allowed_tokens = ["0x...", "0x..."]
gas_costs = { swap = 150000, add_liquidity = 200000, remove_liquidity = 180000, transfer = 65000 }

[programs.tokamon]
type = "app-specific"
game_address = "0x..."
gas_costs = { create_spot = 100000, claim_reward = 80000, feed = 60000, battle = 120000 }
```

---

## 전체 파일 변경 요약

### 생성 (CREATE)

| # | 파일 | 설명 |
|---|------|------|
| 1 | `src/common/app_types.rs` | 앱 서킷 입력 타입 |
| 2 | `src/common/app_state.rs` | Storage proof 기반 상태 관리 |
| 3 | `src/common/incremental_mpt.rs` | 증분 MPT 업데이트 |
| 4 | `src/common/app_execution.rs` | AppCircuit 트레이트 + 공통 실행 엔진 |
| 5 | `src/programs/zk_dex/operations.rs` | DEX 연산 타입 |
| 6 | `src/programs/zk_dex/circuit.rs` | DEX 서킷 구현 |
| 7 | `src/programs/tokamon/operations.rs` | 게임 연산 타입 |
| 8 | `src/programs/tokamon/circuit.rs` | 게임 서킷 구현 |
| 9 | `crates/l2/sequencer/app_witness.rs` | Witness 생성기 |
| 10 | `tests/app_circuit_integration.rs` | 통합 테스트 |

### 수정 (EDIT)

| # | 파일 | 변경 내용 |
|---|------|-----------|
| 1 | `src/common/mod.rs` | 새 모듈 추가 |
| 2 | `src/programs/zk_dex/mod.rs` | 모듈 교체, 테스트 업데이트 |
| 3 | `src/programs/tokamon/mod.rs` | 모듈 교체, 테스트 업데이트 |
| 4 | `bin/sp1-zk-dex/src/main.rs` | 앱 서킷으로 재작성 |
| 5 | `bin/sp1-zk-dex/Cargo.toml` | 의존성 업데이트 |
| 6 | `bin/sp1-tokamon/src/main.rs` | 앱 서킷으로 재작성 |
| 7 | `bin/sp1-tokamon/Cargo.toml` | 의존성 업데이트 |
| 8 | `crates/l2/common/src/prover.rs` | AppProgramInput 추가 |
| 9 | `crates/l2/sequencer/proof_coordinator.rs` | Witness 생성 분기 |
| 10 | `crates/l2/programs.toml` | 앱별 설정 추가 |

### 삭제 (DELETE)

| # | 파일 | 이유 |
|---|------|------|
| 1 | `src/programs/zk_dex/execution.rs` | keccak 기반 → circuit.rs 대체 |
| 2 | `src/programs/zk_dex/types.rs` | DexProgramInput → AppProgramInput 대체 |
| 3 | `src/programs/tokamon/execution.rs` | keccak 기반 → circuit.rs 대체 |
| 4 | `src/programs/tokamon/types.rs` | TokammonProgramInput → AppProgramInput 대체 |

### 변경 없음 (기존 코드 재사용)

| 파일 | 용도 |
|------|------|
| `src/l2/program.rs` | evm-l2 execution_program() 그대로 |
| `src/l2/input.rs` | ProgramInput 그대로 |
| `src/l2/output.rs` | ProgramOutput 그대로 (앱 서킷도 동일 포맷 출력) |
| `src/l2/messages.rs` | 메시지 처리 그대로 (앱 서킷에서 재사용) |
| `src/l2/blobs.rs` | Blob 검증 그대로 (앱 서킷에서 재사용) |
| `src/common/execution.rs` | execute_blocks() 그대로 (evm-l2 전용) |
| `src/programs/evm_l2.rs` | EvmL2GuestProgram 그대로 |
| `bin/sp1/src/main.rs` | evm-l2 SP1 바이너리 그대로 |

---

## 의존성 관계

```
Phase 1 (공통 인프라)
   ↓
Phase 2 (zk-dex 서킷)  ←→  Phase 3 (시퀀서 witness)
   ↓                          ↓
Phase 4 (통합 테스트) ←←←←←←←←┘
   ↓
Phase 5 (tokamon 서킷)
   ↓
Phase 6 (벤치마크/정리)
```

Phase 2와 3은 병렬 진행 가능.
Phase 4는 2+3 완료 후.
Phase 5는 Phase 2 패턴을 복제하므로 빠르게 진행 가능.

---

## 핵심 기술적 도전

1. **증분 MPT 업데이트** (`incremental_mpt.rs`)
   - 가장 복잡한 부분. 기존 `Trie` 코드를 참고하되, 전체 트리 없이 proof만으로 업데이트
   - 기존 `GuestProgramState.apply_account_updates()`의 로직을 proof 기반으로 재구현

2. **EVM-서킷 결과 일치**
   - Solidity 컨트랙트의 스토리지 레이아웃을 정확히 알아야 함
   - function selector, ABI 인코딩, 스토리지 슬롯 매핑 등

3. **L2 fee 처리**
   - base fee vault, operator fee vault, L1 fee vault 분배
   - `l2_hook.rs`의 fee 분배 로직을 서킷에서 재현
