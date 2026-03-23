# 게스트 프로그램 추가 가이드

새로운 ZK 게스트 프로그램을 ethrex에 추가할 때 필요한 모든 단계를 정리합니다.

## 두 가지 유형

| 유형 | 입력 | 실행 엔진 | 사이클 | 예시 |
|------|------|----------|--------|------|
| **Full EVM** | `ProgramInput` | `execution_program()` | 22M+ | evm-l2 |
| **App Circuit** | `AppProgramInput` | `execute_app_circuit()` | ~1M | bridge, zk-dex |

**App Circuit 권장** — 특정 트랜잭션만 증명하면 되는 경우 20배+ 빠름.

## App Circuit 구조 (권장)

```
execute_app_circuit(circuit, input)
  ├── 공통 핸들러 (자동)
  │   ├── deposit (L1→L2) — handle_privileged_tx()
  │   ├── withdrawal (L2→L1) — handle_withdrawal()
  │   ├── ETH transfer — handle_eth_transfer()
  │   ├── system call — handle_system_call()
  │   └── gas fee — apply_gas_fee_distribution()
  └── 앱 전용 (circuit 구현)
      ├── classify_tx() — TX 분류
      ├── execute_operation() — 상태 전이
      ├── gas_cost() — 가스 비용
      └── generate_logs() — 이벤트 로그
```

## 체크리스트

### 1. 게스트 프로그램 바이너리 (`bin/sp1-{name}/`)

- [ ] `bin/sp1-{name}/Cargo.toml` 생성 (sp1/Cargo.toml 복사 후 수정)
- [ ] **Cargo.lock 동기화**: `cp bin/sp1/Cargo.lock bin/sp1-{name}/Cargo.lock`
  > **중요**: Cargo.lock이 다르면 동일 코드라도 다른 바이너리가 생성되어 SP1 proof panic 발생
- [ ] `bin/sp1-{name}/src/main.rs` 작성:

```rust
#![no_main]
use ethrex_guest_program::common::app_execution::execute_app_circuit;
use ethrex_guest_program::common::app_types::AppProgramInput;
use ethrex_guest_program::programs::{name}::circuit::{Name}Circuit;
use rkyv::rancor::Error;

sp1_zkvm::entrypoint!(main);

pub fn main() {
    let input = sp1_zkvm::io::read_vec();
    let input = rkyv::from_bytes::<AppProgramInput, Error>(&input).unwrap();
    let circuit = {Name}Circuit;
    let output = execute_app_circuit(&circuit, input).unwrap();
    sp1_zkvm::io::commit_slice(&output.encode());
}
```

### 2. 프로그램 모듈 (`src/programs/{name}/`)

- [ ] `src/programs/{name}/mod.rs` — `GuestProgram` trait 구현
- [ ] `src/programs/{name}/circuit.rs` — `AppCircuit` trait 구현
- [ ] `src/programs/{name}/analyze.rs` — 트랜잭션 분석 (필요 계정/슬롯 수집)
- [ ] `src/programs/mod.rs`에 모듈 + `pub use` 추가

#### circuit.rs

```rust
pub struct {Name}Circuit;

impl AppCircuit for {Name}Circuit {
    fn classify_tx(&self, tx: &Transaction) -> Result<AppOperation, AppCircuitError> {
        // 앱 전용 TX가 없으면: Err(AppCircuitError::UnknownTransaction)
        // 앱 전용 TX가 있으면: calldata selector로 분류
    }
    fn execute_operation(&self, state: &mut AppState, from: Address, op: &AppOperation)
        -> Result<OperationResult, AppCircuitError> { ... }
    fn gas_cost(&self, op: &AppOperation) -> u64 { ... }
    fn generate_logs(&self, from: Address, op: &AppOperation, result: &OperationResult) -> Vec<Log> { ... }
}
```

#### analyze.rs — 핵심 주의사항

**witness에 존재하는 계정만 요청해야 합니다.** witness에 없는 계정의 proof를 요청하면 Trie error 발생.

규칙:
- coinbase: 항상 포함 (gas fee)
- TX sender: `tx.sender()`로 복구 후 포함
- TX recipient: `tx.to()` 포함
- 시스템 컨트랙트: **실제로 TX가 호출할 때만** 포함
  - `COMMON_BRIDGE_L2_ADDRESS` — withdrawal TX가 있을 때만
  - `BURN_ADDRESS`, `L2_TO_L1_MESSENGER_ADDRESS` — withdrawal 있을 때만
  - `FEE_TOKEN_REGISTRY_ADDRESS`, `FEE_TOKEN_RATIO_ADDRESS` — TX가 호출할 때만
- MESSENGER storage slot — withdrawal 있을 때만
- fee vault — `fee_config`에 설정되어 있을 때만

**zk-dex의 analyze 패턴을 따르세요.** bridge analyze가 정확한 참고 예시입니다.

#### mod.rs — serialize_input

```rust
fn serialize_input(&self, raw_input: &[u8]) -> Result<Vec<u8>, GuestProgramError> {
    let program_input: ProgramInput = rkyv::from_bytes(raw_input)?;
    let (accounts, storage_slots) = analyze::analyze_{name}_transactions(
        &program_input.blocks, &program_input.fee_configs, &program_input.execution_witness,
    )?;
    let app_input = convert_to_app_input(program_input, &accounts, &storage_slots)?;
    Ok(rkyv::to_bytes(&app_input)?.to_vec())
}
```

### 3. ELF 포함 (`src/lib.rs`)

```rust
#[cfg(all(not(clippy), feature = "sp1"))]
pub static ZKVM_SP1_{NAME}_ELF: &[u8] =
    include_bytes!("../bin/sp1-{name}/out/riscv32im-succinct-zkvm-elf");
#[cfg(any(clippy, not(feature = "sp1")))]
pub const ZKVM_SP1_{NAME}_ELF: &[u8] = &[];
```

### 4. 빌드 시스템 (`build.rs`)

```rust
if programs.contains(&"{name}".to_string()) {
    #[cfg(all(not(clippy), feature = "sp1"))]
    build_sp1_guest_program("sp1-{name}");
} else {
    ensure_elf_placeholder("./bin/sp1-{name}");
}
```

### 5. Prover 등록 (`crates/l2/prover/`)

```rust
// prover.rs
("{name}".to_string(), Arc::new({Name}GuestProgram)),
```

### 6. Type ID 등록 (`crates/l2/common/lib.rs`)

```rust
"{name}" => N, // 기존 IDs: evm-l2=1, zk-dex=2, tokamon=3, bridge=4
```

### 7. Dockerfile.sp1

- `ARG GUEST_PROGRAMS` 기본값에 추가
- VK 파일 COPY 추가

### 8. Deployer

- `ETHREX_REGISTER_GUEST_PROGRAMS={name}:N` 환경변수
- deployer가 GuestProgramRegistry에 등록 + VK 등록

### 9. Compose Generator

- `compose-generator.js`에서 programs.toml 파싱 시 typeId 매핑 추가

## 성능 비교 (실측)

| 프로그램 | ELF 크기 | Execution Cycles | SP1 Proof 시간 | 특징 |
|---------|---------|-----------------|---------------|------|
| evm-l2 | 4.1 MB | 22,000,000+ | ~20분 | 범용 EVM 전체 |
| zk-dex | 1.2 MB | ~5,000,000 | ~5분 | DEX 전용 8개 op |
| **bridge** | **725 KB** | **983,945** | **4분** | 공통 핸들러만 |

Bridge가 가장 빠른 이유:
1. 앱 전용 operation 없음 → classify_tx 스킵
2. 앱 전용 storage 없음 → MPT 검증 최소화
3. ELF가 작음 → SP1 setup/compress 빠름

## 주의사항

- **Cargo.lock 동기화 필수** — 불일치 시 `Invalid memory access: addr=0` panic
- **SP1 5.0.8은 32-bit RISC-V만 지원** — `sp1_build`(build.rs)로 빌드
- **`cargo-prove build`는 64-bit 생성** — 사용 불가
- **witness 계정 일치** — analyze에서 witness에 없는 계정 요청 시 Trie error
- **VK 등록 필수** — deployer가 L1 OnChainProposer에 VK를 등록해야 proof verify 통과
