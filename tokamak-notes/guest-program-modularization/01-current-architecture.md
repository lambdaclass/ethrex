# 현재 아키텍처 분석

이 문서는 Guest Program 모듈화를 위해 현재 아키텍처의 결합 지점(coupling points)을 분석한다.

## 1. Guest Program 구조

### 1.1 디렉토리 레이아웃

```
crates/guest-program/
├── build.rs                    # ELF 빌드 (SP1, RISC0, ZisK, OpenVM)
├── bin/
│   ├── sp1/                    # SP1 게스트 엔트리포인트
│   │   └── out/                # 컴파일된 SP1 ELF
│   ├── risc0/                  # RISC0 게스트 엔트리포인트
│   │   └── out/                # 컴파일된 RISC0 ELF + VK
│   ├── zisk/                   # ZisK 게스트 엔트리포인트
│   └── openvm/                 # OpenVM 게스트 엔트리포인트
└── src/
    ├── lib.rs                  # 루트: feature flag 분기 + ELF 정적 상수
    ├── methods.rs              # RISC0 메서드 ID
    ├── common/
    │   ├── mod.rs
    │   ├── execution.rs        # execute_blocks() — 핵심 실행 로직
    │   └── error.rs
    ├── l1/                     # L1 게스트 프로그램
    │   ├── input.rs
    │   ├── output.rs
    │   └── program.rs
    └── l2/                     # L2 게스트 프로그램
        ├── input.rs            # ProgramInput 정의
        ├── output.rs           # ProgramOutput 정의 + encode()
        ├── program.rs          # execution_program() 엔트리포인트
        ├── blobs.rs            # blob 검증
        ├── messages.rs         # 메시지 처리
        └── error.rs
```

### 1.2 Feature Flag 다형성

**파일**: `crates/guest-program/src/lib.rs:10-35`

L1/L2 구분을 컴파일 타임 feature flag(`l2`)로 처리한다:

```rust
#[cfg(feature = "l2")]
pub mod input {
    pub use crate::l2::ProgramInput;
}
#[cfg(not(feature = "l2"))]
pub mod input {
    pub use crate::l1::ProgramInput;
}

#[cfg(feature = "l2")]
pub mod output {
    pub use crate::l2::ProgramOutput;
}
// ... execution 모듈도 동일 패턴
```

**결합 지점**: `input`, `output`, `execution` 모듈이 feature flag에 의해 **정적으로 결정**된다. 런타임에 다른 Guest Program을 선택할 수 없다.

### 1.3 ELF 바이너리 정적 임베딩

**파일**: `crates/guest-program/src/lib.rs:39-55`

```rust
#[cfg(all(not(clippy), feature = "sp1"))]
pub static ZKVM_SP1_PROGRAM_ELF: &[u8] =
    include_bytes!("../bin/sp1/out/riscv32im-succinct-zkvm-elf");

#[cfg(all(not(clippy), feature = "risc0"))]
pub static ZKVM_RISC0_PROGRAM_VK: &str =
    include_str!(concat!("../bin/risc0/out/riscv32im-risc0-vk"));

#[cfg(all(not(clippy), feature = "zisk"))]
pub static ZKVM_ZISK_PROGRAM_ELF: &[u8] =
    include_bytes!("../bin/zisk/target/riscv64ima-zisk-zkvm-elf/release/ethrex-guest-zisk");
```

**결합 지점**: ELF 바이너리가 `include_bytes!()`로 **컴파일 타임에 바이너리에 고정**된다. 다른 Guest Program의 ELF를 사용하려면 전체를 다시 컴파일해야 한다.

### 1.4 `execute_blocks()` — 공통 실행 로직

**파일**: `crates/guest-program/src/common/execution.rs:38-186`

```rust
pub fn execute_blocks<F>(
    blocks: &[Block],
    execution_witness: ExecutionWitness,
    elasticity_multiplier: u64,
    vm_factory: F,
) -> Result<BatchExecutionResult, ExecutionError>
where
    F: Fn(&GuestProgramStateWrapper, usize) -> Result<Evm, ExecutionError>,
```

이 함수는 L1/L2 공통으로 사용되는 블록 실행 로직이다:
- `vm_factory` 클로저로 EVM 인스턴스 생성을 추상화한다 (L1은 `Evm::new()`, L2는 `Evm::new_for_l2(fee_config)`).
- 블록 검증 → EVM 실행 → 상태 전이 적용 → 최종 상태 검증 순서로 진행한다.
- `report_cycles()`로 zkVM 내부 사이클 측정을 지원한다.

**재사용 가능**: VM factory 패턴이 이미 추상화되어 있으므로, 모듈화 시 이 구조를 확장할 수 있다.

### 1.5 L2 `execution_program()` — L2 엔트리포인트

**파일**: `crates/guest-program/src/l2/program.rs:15-68`

```rust
pub fn execution_program(input: ProgramInput) -> Result<ProgramOutput, L2ExecutionError> {
    let ProgramInput {
        blocks, execution_witness, elasticity_multiplier,
        fee_configs, blob_commitment, blob_proof,
    } = input;

    let BatchExecutionResult { receipts, initial_state_hash, final_state_hash, ... } =
        execute_blocks(&blocks, execution_witness, elasticity_multiplier, |db, i| {
            // L2 VM factory
            Evm::new_for_l2(db.clone(), fee_config)
        })?;

    // L2 전용: 메시지 추출, blob 검증
    let batch_messages = get_batch_messages(&blocks, &receipts, chain_id);
    let message_digests = compute_message_digests(&batch_messages)?;
    let blob_versioned_hash = verify_blob(...)?;

    Ok(ProgramOutput { initial_state_hash, final_state_hash, ... })
}
```

**결합 지점**: 이 함수가 **L2 전용 로직 전부를 포함**한다 — 메시지 처리, blob 검증, 출력 구성. 다른 Guest Program은 이 중 일부만 필요하거나 완전히 다른 로직이 필요할 수 있다.

### 1.6 `ProgramOutput.encode()` — L1 검증과의 결합

**파일**: `crates/guest-program/src/l2/output.rs:30-68`

```rust
impl ProgramOutput {
    pub fn encode(&self) -> Vec<u8> {
        let mut encoded = [
            self.initial_state_hash.to_fixed_bytes(),     // 32 bytes
            self.final_state_hash.to_fixed_bytes(),       // 32 bytes
            self.l1_out_messages_merkle_root.to_fixed_bytes(),
            self.l1_in_messages_rolling_hash.to_fixed_bytes(),
            self.blob_versioned_hash.to_fixed_bytes(),
            self.last_block_hash.to_fixed_bytes(),
            self.chain_id.to_big_endian(),                // 32 bytes
            self.non_privileged_count.to_big_endian(),    // 32 bytes
        ].concat();
        // + balance_diffs (가변 길이)
        // + l2_in_message_rolling_hashes (가변 길이)
    }
}
```

**결합 지점**: 이 인코딩은 L1 `OnChainProposer.sol`의 `_getPublicInputsFromCommitment()` (라인 621-710)과 **바이트 단위로 일치**해야 한다. 다른 Guest Program이 다른 출력 구조를 갖는다면, L1에서도 해당 프로그램의 인코딩을 재구성할 수 있어야 한다.

## 2. ProverBackend 트레이트

### 2.1 트레이트 정의

**파일**: `crates/l2/prover/src/backend/mod.rs:81-147`

```rust
pub trait ProverBackend {
    type ProofOutput;
    type SerializedInput;

    fn prover_type(&self) -> ProverType;
    fn serialize_input(&self, input: &ProgramInput) -> Result<Self::SerializedInput, BackendError>;
    fn execute(&self, input: ProgramInput) -> Result<(), BackendError>;
    fn prove(&self, input: ProgramInput, format: ProofFormat) -> Result<Self::ProofOutput, BackendError>;
    fn verify(&self, proof: &Self::ProofOutput) -> Result<(), BackendError>;
    fn to_batch_proof(&self, proof: Self::ProofOutput, format: ProofFormat) -> Result<BatchProof, BackendError>;
}
```

**결합 지점 1**: `ProgramInput`을 **직접 참조**한다. `ProgramInput`은 `crates/guest-program/src/l2/input.rs`에서 정의된 L2 전용 타입이다. 백엔드가 Guest Program의 입력 타입을 알아야 한다.

**결합 지점 2**: `serialize_input()`이 백엔드마다 `ProgramInput` → 백엔드 전용 포맷 변환을 수행한다. 이 직렬화 로직은 본질적으로 Guest Program의 책임이다.

### 2.2 SP1 백엔드 — ELF 직접 참조

**파일**: `crates/l2/prover/src/backend/sp1.rs`

```rust
use ethrex_guest_program::{ZKVM_SP1_PROGRAM_ELF, input::ProgramInput};

// 셋업 시 ELF 정적 상수 직접 사용
pub fn init_prover_setup(_endpoint: Option<Url>) -> ProverSetup {
    let (pk, vk) = client.setup(ZKVM_SP1_PROGRAM_ELF);  // 라인 54
    // ...
}

// 실행 시에도 ELF 정적 상수 직접 사용
fn execute_with_stdin(&self, stdin: &SP1Stdin) -> Result<(), BackendError> {
    setup.client.execute(ZKVM_SP1_PROGRAM_ELF, stdin)    // 라인 121
        .map_err(BackendError::execution)?;
}

// 직렬화: ProgramInput → SP1Stdin (rkyv)
fn serialize_input(&self, input: &ProgramInput) -> Result<SP1Stdin, BackendError> {
    let mut stdin = SP1Stdin::new();
    let bytes = rkyv::to_bytes::<Error>(input)?;         // 라인 152
    stdin.write_slice(bytes.as_slice());
    Ok(stdin)
}
```

**결합 지점**:
- `ZKVM_SP1_PROGRAM_ELF` 정적 상수를 3곳에서 직접 참조 (`setup`, `execute`, `prove`)
- `ProgramInput`을 직접 `rkyv` 직렬화

### 2.3 RISC0 백엔드 — ELF 및 Image ID 직접 참조

**파일**: `crates/l2/prover/src/backend/risc0.rs`

```rust
use ethrex_guest_program::{
    input::ProgramInput,
    methods::{ETHREX_GUEST_RISC0_ELF, ETHREX_GUEST_RISC0_ID},
};

// 실행 시 ELF 직접 사용
fn execute_with_env(&self, env: ExecutorEnv<'_>) -> Result<(), BackendError> {
    executor.execute(env, ETHREX_GUEST_RISC0_ELF)        // 라인 68
}

// 증명 시 ELF 직접 사용
fn prove_with_env(&self, env: ExecutorEnv<'_>, format: ProofFormat) -> Result<Receipt, BackendError> {
    prover.prove_with_opts(env, ETHREX_GUEST_RISC0_ELF, &prover_opts)  // 라인 82
}

// 검증 시 Image ID 직접 사용
fn verify(&self, proof: &Receipt) -> Result<(), BackendError> {
    proof.verify(ETHREX_GUEST_RISC0_ID)                  // 라인 120
}
```

**결합 지점**: SP1과 동일 패턴 — `ETHREX_GUEST_RISC0_ELF`와 `ETHREX_GUEST_RISC0_ID`를 직접 참조.

### 2.4 Exec 백엔드 — 실행 함수 직접 호출

**파일**: `crates/l2/prover/src/backend/exec.rs`

```rust
use ethrex_guest_program::{input::ProgramInput, output::ProgramOutput};

fn execute_core(input: ProgramInput) -> Result<ProgramOutput, BackendError> {
    ethrex_guest_program::execution::execution_program(input)  // 라인 27
        .map_err(BackendError::execution)
}
```

**결합 지점**: `execution_program()`을 직접 호출한다. 이 함수는 feature flag에 의해 L1 또는 L2 실행 함수로 결정된다.

## 3. ELF 빌드 파이프라인

### 3.1 `build.rs` 구조

**파일**: `crates/guest-program/build.rs`

```rust
fn main() {
    #[cfg(all(not(clippy), feature = "risc0"))]
    build_risc0_program();

    #[cfg(all(not(clippy), feature = "sp1"))]
    build_sp1_program();

    #[cfg(all(not(clippy), feature = "zisk"))]
    build_zisk_program();

    #[cfg(all(not(clippy), feature = "openvm"))]
    build_openvm_program();
}
```

각 빌드 함수:
- **SP1** (`build_sp1_program()`, 라인 66-106): `sp1_build::build_program_with_args()`로 `bin/sp1/` 소스를 RISC-V ELF로 컴파일. `l2` feature를 전달. VK를 `bin/sp1/out/`에 저장.
- **RISC0** (`build_risc0_program()`, 라인 18-63): `risc0_build::embed_methods_with_options()`로 `ethrex-guest-risc0`를 빌드. Image ID(VK)를 `bin/risc0/out/`에 저장.
- **ZisK** (`build_zisk_program()`, 라인 108-187): `cargo +zisk build`로 RISC-V 64비트 ELF 빌드. `cargo-zisk rom-setup` 후처리.
- **OpenVM** (`build_openvm_program()`, 라인 189-220): `cargo openvm build`로 ELF 빌드.

**결합 지점**: 빌드 스크립트가 **단일 Guest Program**만 빌드한다. 여러 Guest Program을 빌드하려면 빌드 스크립트를 확장해야 한다. 또한 `l2` feature가 빌드 시에도 전달되어 L1/L2를 결정한다.

## 4. L1 검증 파이프라인

### 4.1 `OnChainProposer.sol` — VK 매핑

**파일**: `crates/l2/contracts/src/l1/OnChainProposer.sol`

```solidity
uint8 internal constant SP1_VERIFIER_ID = 1;    // 라인 46
uint8 internal constant RISC0_VERIFIER_ID = 2;   // 라인 47

mapping(bytes32 commitHash => mapping(uint8 verifierId => bytes32 vk))
    public verificationKeys;                      // 라인 114
```

VK는 `(commitHash, verifierId)` 2차원으로 저장된다:
- `commitHash`: git 커밋 해시의 keccak256 — 코드 버전별 VK 관리
- `verifierId`: 1=SP1, 2=RISC0 — zkVM 백엔드별 VK

**결합 지점**: `verifierId`가 **zkVM 백엔드 종류**만 구분한다. 동일 커밋 + 동일 백엔드에서 **다른 Guest Program**의 VK를 구분할 차원이 없다.

### 4.2 `commitBatch()` — 배치 커밋

**파일**: `crates/l2/contracts/src/l1/OnChainProposer.sol:256-355`

```solidity
function commitBatch(
    uint256 batchNumber,
    bytes32 newStateRoot,
    bytes32 withdrawalsLogsMerkleRoot,
    bytes32 processedPrivilegedTransactionsRollingHash,
    bytes32 lastBlockHash,
    uint256 nonPrivilegedTransactions,
    bytes32 commitHash,
    ICommonBridge.BalanceDiff[] calldata balanceDiffs,
    ICommonBridge.L2MessageRollingHash[] calldata l2MessageRollingHashes
) external override onlyOwner whenNotPaused {
```

배치 커밋 시 `commitHash`와 함께 VK 존재 여부를 검증한다 (라인 329-339):
```solidity
if (REQUIRE_SP1_PROOF && verificationKeys[commitHash][SP1_VERIFIER_ID] == bytes32(0)) {
    revert("013"); // missing verification key
}
```

**결합 지점**: `commitBatch()`에 `programTypeId`가 없으므로, 모든 배치가 동일한 Guest Program(EVM-L2)을 사용한다고 가정한다.

### 4.3 `verifyBatch()` — 배치 검증

**파일**: `crates/l2/contracts/src/l1/OnChainProposer.sol:363-480`

```solidity
function verifyBatch(
    uint256 batchNumber,
    bytes memory risc0BlockProof,
    bytes memory sp1ProofBytes,
    bytes memory tdxSignature
) external override onlyOwner whenNotPaused {
    // public inputs 재구성
    bytes memory publicInputs = _getPublicInputsFromCommitment(batchNumber);

    // RISC0 검증
    if (REQUIRE_RISC0_PROOF) {
        bytes32 risc0Vk = verificationKeys[batchCommitHash][RISC0_VERIFIER_ID];
        IRiscZeroVerifier(RISC0_VERIFIER_ADDRESS).verify(
            risc0BlockProof, risc0Vk, sha256(publicInputs)
        );
    }

    // SP1 검증
    if (REQUIRE_SP1_PROOF) {
        bytes32 sp1Vk = verificationKeys[batchCommitHash][SP1_VERIFIER_ID];
        ISP1Verifier(SP1_VERIFIER_ADDRESS).verifyProof(
            sp1Vk, publicInputs, sp1ProofBytes
        );
    }
}
```

**결합 지점**: `_getPublicInputsFromCommitment()`이 **단일 인코딩 형식**만 지원한다. 다른 Guest Program이 다른 public inputs 구조를 갖는다면, 이 함수도 프로그램 타입별로 분기해야 한다.

### 4.4 `_getPublicInputsFromCommitment()` — Public Inputs 재구성

**파일**: `crates/l2/contracts/src/l1/OnChainProposer.sol:621-710`

고정 크기 필드 256 bytes:
```
bytes 0-32:   initialStateRoot       (이전 배치의 newStateRoot)
bytes 32-64:  finalStateRoot         (현재 배치의 newStateRoot)
bytes 64-96:  withdrawalsMerkleRoot
bytes 96-128: l1InMessagesRollingHash
bytes 128-160: blobVersionedHash
bytes 160-192: lastBlockHash
bytes 192-224: chainId
bytes 224-256: nonPrivilegedCount
```

가변 크기 필드:
```
balanceDiffs:              chainId(32) + value(32) + [tokenL1(20)+tokenL2(20)+destTokenL2(20)+value(32)]* + [messageHash(32)]*
l2InMessageRollingHashes:  [chainId(32) + rollingHash(32)]*
```

이 인코딩은 `ProgramOutput.encode()` (`l2/output.rs:32-68`)와 **바이트 단위로 동일**해야 한다.

**결합 지점**: Guest Program 출력 인코딩 ↔ L1 public inputs 재구성이 **하드코딩**으로 결합되어 있다.

## 5. Prover ↔ Proof Coordinator 프로토콜

### 5.1 `ProofData` 통신 프로토콜

**파일**: `crates/l2/common/src/prover.rs:162-221`

```rust
pub enum ProofData {
    BatchRequest { commit_hash: String, prover_type: ProverType },
    BatchResponse { batch_number: Option<u64>, input: Option<ProverInputData>, format: Option<ProofFormat> },
    ProofSubmit { batch_number: u64, batch_proof: BatchProof },
    ProofSubmitACK { batch_number: u64 },
    // ...
}
```

**결합 지점**: `BatchRequest`에 `program_id`가 없다. 프루버가 어떤 Guest Program을 실행할 수 있는지 코디네이터에 알릴 방법이 없다. 코디네이터도 배치에 어떤 Guest Program이 필요한지 지정할 수 없다.

### 5.2 `ProverInputData` — 코디네이터 → 프루버 입력

**파일**: `crates/l2/common/src/prover.rs:13-23`

```rust
pub struct ProverInputData {
    pub blocks: Vec<Block>,
    pub execution_witness: ExecutionWitness,
    pub elasticity_multiplier: u64,
    pub blob_commitment: blobs_bundle::Commitment,
    pub blob_proof: blobs_bundle::Proof,
    pub fee_configs: Vec<FeeConfig>,
}
```

**결합 지점**: `ProverInputData`가 L2 EVM Guest Program 전용 필드를 갖고 있다. 다른 Guest Program은 다른 입력 구조가 필요하다.

### 5.3 Proof Coordinator — 배치 할당

**파일**: `crates/l2/sequencer/proof_coordinator.rs:140-232`

```rust
async fn handle_request(&self, stream: &mut TcpStream, commit_hash: String, prover_type: ProverType) {
    // 1. prover_type이 needed_proof_types에 있는지 확인
    // 2. 다음 증명할 배치 번호 결정
    // 3. 해당 배치의 ProverInputData 조회
    // 4. BatchResponse 전송
}
```

**결합 지점**: 코디네이터가 `prover_type`(zkVM 백엔드)만 고려하고, **Guest Program 종류**는 고려하지 않는다. 모든 배치가 동일한 Guest Program을 사용한다고 가정한다.

### 5.4 L1 Proof Sender — 증명 제출

**파일**: `crates/l2/sequencer/l1_proof_sender.rs:184-260`

```rust
async fn verify_and_send_proof(&self) -> Result<(), ProofSenderError> {
    // 1. 다음 검증할 배치 결정
    // 2. 필요한 모든 proof type의 증명이 있는지 확인
    // 3. 있으면 L1에 verifyBatch() 트랜잭션 전송
}
```

`send_proof_to_contract()` (라인 398-482):
```rust
let calldata_values = [
    &[Value::Uint(U256::from(batch_number))],
    proofs.get(&ProverType::RISC0).map(|p| p.calldata()).unwrap_or(ProverType::RISC0.empty_calldata()).as_slice(),
    proofs.get(&ProverType::SP1).map(|p| p.calldata()).unwrap_or(ProverType::SP1.empty_calldata()).as_slice(),
    proofs.get(&ProverType::TDX).map(|p| p.calldata()).unwrap_or(ProverType::TDX.empty_calldata()).as_slice(),
].concat();
```

**결합 지점**: `verifyBatch()` 호출 시그니처가 `(uint256, bytes, bytes, bytes)` — RISC0, SP1, TDX 증명을 **고정된 순서로** 전달. Guest Program 타입별 분기가 없다.

## 6. Prover 메인 루프

**파일**: `crates/l2/prover/src/prover.rs`

```rust
struct Prover<B: ProverBackend> {
    backend: B,
    // ...
}

impl<B: ProverBackend> Prover<B> {
    pub async fn start(&self) {
        loop {
            // 1. Proof Coordinator에서 배치 요청
            // 2. ProverInputData → ProgramInput 변환
            // 3. backend.prove(input, format) 호출
            // 4. 증명 제출
        }
    }
}
```

`request_new_input()` (라인 161-215)에서 `ProverInputData` → `ProgramInput` 변환:
```rust
#[cfg(feature = "l2")]
let input = ProgramInput {
    blocks: input.blocks,
    execution_witness: input.execution_witness,
    elasticity_multiplier: input.elasticity_multiplier,
    blob_commitment: input.blob_commitment,
    blob_proof: input.blob_proof,
    fee_configs: input.fee_configs,
};
```

**결합 지점**: `ProgramInput` 생성이 feature flag(`l2`)로 결정된다. 프루버는 단일 Guest Program만 실행할 수 있다.

## 7. 결합 지점 요약

모듈화를 위해 해결해야 할 결합 지점을 정리한다:

| # | 결합 지점 | 위치 | 문제 | 해결 방향 |
|---|----------|------|------|----------|
| C1 | ELF 정적 임베딩 | `lib.rs:39-55` | ELF가 컴파일 타임에 고정 | ELF를 GuestProgram 트레이트에서 제공 |
| C2 | `ProverBackend`가 `ProgramInput` 직접 참조 | `backend/mod.rs:92,107,110` | 백엔드가 입력 타입을 알아야 함 | bytes 기반 인터페이스로 변경 |
| C3 | 백엔드에서 ELF 상수 직접 참조 | `sp1.rs:54,121`, `risc0.rs:68,82,120` | 단일 ELF만 사용 가능 | ELF를 파라미터로 전달 |
| C4 | Feature flag 다형성 | `lib.rs:10-35` | 런타임 선택 불가 | GuestProgram 트레이트로 대체 |
| C5 | `ProgramOutput.encode()` ↔ L1 하드코딩 | `output.rs:32-68` ↔ `OnChainProposer.sol:621-710` | 단일 인코딩만 지원 | 프로그램 타입별 인코딩 디스패치 |
| C6 | VK 매핑 2차원 | `OnChainProposer.sol:114` | Guest Program 구분 불가 | `programTypeId` 차원 추가 |
| C7 | `ProofData`에 `program_id` 없음 | `prover.rs:165,184` | 프루버-코디네이터 간 프로그램 식별 불가 | `program_id` 필드 추가 |
| C8 | `ProverInputData` L2 전용 | `prover.rs:13-23` | 다른 입력 구조 불가 | bytes 기반 또는 범용 래퍼 |
| C9 | `verifyBatch()` 고정 시그니처 | `OnChainProposer.sol:363-371` | 프로그램 타입 무관 | 프로그램 타입별 검증 디스패치 |
| C10 | 빌드 스크립트 단일 프로그램 | `build.rs` | 하나의 Guest Program만 빌드 | 멀티 프로그램 빌드 지원 |

다음 문서 (`02-target-architecture.md`)에서 각 결합 지점의 해결 방안을 설계한다.
