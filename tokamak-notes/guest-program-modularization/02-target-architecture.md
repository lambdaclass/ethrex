# 목표 아키텍처 설계

이 문서는 Guest Program 모듈화의 목표 아키텍처를 설계한다. `01-current-architecture.md`에서 식별한 결합 지점(C1-C10)을 해결하는 구조를 제시한다.

## 설계 개요

```
                        ┌──────────────────────────────┐
                        │    GuestProgramRegistry       │
                        │  ┌─────────┬──────────────┐  │
                        │  │ evm-l2  │ transfer     │  │
                        │  │ (기본)  │ (레퍼런스)   │  │
                        │  └────┬────┴──────┬───────┘  │
                        └───────┼───────────┼──────────┘
                                │           │
                   ┌────────────▼───────────▼──────────┐
                   │        GuestProgram trait           │
                   │  program_id()  elf()  vk_bytes()   │
                   │  serialize_input()  encode_output() │
                   └────────────────┬───────────────────┘
                                    │ bytes
                   ┌────────────────▼───────────────────┐
                   │      ProverBackend (확장)           │
                   │  prove(elf, serialized_input)       │
                   │  ← ELF/입력 모두 bytes로 수신       │
                   ├────────┬────────┬─────────┬────────┤
                   │  SP1   │ RISC0  │  ZisK   │ OpenVM │
                   └────────┴────────┴─────────┴────────┘
                                    │
                   ┌────────────────▼───────────────────┐
                   │    L1 OnChainProposer (확장)        │
                   │  verificationKeys                   │
                   │    [commitHash][programTypeId]       │
                   │      [verifierId] → vk              │
                   └────────────────────────────────────┘
```

## A. GuestProgram 트레이트 (bytes 수준 추상화)

### 설계 결정

**핵심 선택: bytes 수준 추상화 vs 제네릭 타입**

| 방식 | 장점 | 단점 |
|------|------|------|
| 제네릭 (`trait GuestProgram<I, O>`) | 타입 안전성, 컴파일 타임 검증 | `ProverBackend`에 제네릭 전파, 타입 폭발, 동적 디스패치 복잡 |
| bytes (`trait GuestProgram`) | 프루버와 완전 분리, 간단한 인터페이스 | 런타임 직렬화 오류 가능성 |

**선택: bytes 수준 추상화**

이유:
1. `ProverBackend`는 이미 ELF(bytes)를 실행하고 bytes 결과를 반환하는 구조이다. ELF를 실행하는 zkVM은 Guest Program의 타입을 모른다.
2. 제네릭을 사용하면 `ProverBackend<G: GuestProgram>`, `Prover<B: ProverBackend<G>, G: GuestProgram>` 등으로 타입이 전파되어 기존 코드 변경이 과도해진다.
3. 각 Guest Program 크레이트 내부에서는 자체 타입 시스템으로 안전성을 보장하고, 프루버 인터페이스에서만 bytes로 통신한다.

### 트레이트 정의

```rust
// crates/guest-program/src/traits.rs (신규)

use crate::backend::BackendType;

/// 모든 Guest Program이 구현해야 하는 인터페이스.
///
/// bytes 수준에서 동작하여 ProverBackend와의 제네릭 결합을 방지한다.
/// 각 Guest Program은 독립 크레이트로 컴파일되며, 자체 Input/Output 타입을
/// 내부적으로 사용하되 이 트레이트를 통해 bytes로 노출한다.
pub trait GuestProgram: Send + Sync {
    /// 이 Guest Program의 고유 식별자.
    /// 예: "evm-l2", "transfer", "dex"
    fn program_id(&self) -> &str;

    /// 지정된 zkVM 백엔드용 컴파일된 ELF 바이너리.
    /// 없으면 None 반환 (해당 백엔드 미지원).
    fn elf(&self, backend: BackendType) -> Option<&[u8]>;

    /// 지정된 zkVM 백엔드의 verification key (bytes).
    /// L1 컨트랙트에 등록할 VK.
    fn vk_bytes(&self, backend: BackendType) -> Option<Vec<u8>>;

    /// 원본 입력 데이터를 직렬화.
    /// ProverBackend에 전달할 bytes 형태의 입력을 생성한다.
    /// `raw_input`은 ProverInputData의 직렬화된 형태.
    fn serialize_input(&self, raw_input: &[u8]) -> Result<Vec<u8>, GuestProgramError>;

    /// Guest Program 출력을 public values bytes로 인코딩.
    /// L1 _getPublicInputsFromCommitment()과 일치해야 한다.
    /// `raw_output`은 zkVM이 반환한 public values.
    fn encode_output(&self, raw_output: &[u8]) -> Result<Vec<u8>, GuestProgramError>;

    /// L1 컨트랙트에서 이 프로그램을 식별하는 정수 ID.
    /// verificationKeys[commitHash][programTypeId][verifierId]의 두 번째 키.
    fn program_type_id(&self) -> u8;
}

#[derive(Debug, thiserror::Error)]
pub enum GuestProgramError {
    #[error("Serialization error: {0}")]
    Serialization(String),
    #[error("Unsupported backend: {0:?}")]
    UnsupportedBackend(BackendType),
    #[error("Internal error: {0}")]
    Internal(String),
}
```

### EvmL2GuestProgram 구현

```rust
// crates/guest-program/src/programs/evm_l2.rs (신규)

use crate::traits::{GuestProgram, GuestProgramError};
use crate::backend::BackendType;

/// 기존 EVM-L2 Guest Program을 GuestProgram 트레이트로 래핑.
/// 기존 코드의 동작을 100% 유지하면서 새 인터페이스를 제공한다.
pub struct EvmL2GuestProgram;

impl GuestProgram for EvmL2GuestProgram {
    fn program_id(&self) -> &str {
        "evm-l2"
    }

    fn elf(&self, backend: BackendType) -> Option<&[u8]> {
        match backend {
            #[cfg(feature = "sp1")]
            BackendType::SP1 => Some(crate::ZKVM_SP1_PROGRAM_ELF),
            #[cfg(feature = "risc0")]
            BackendType::RISC0 => {
                // RISC0는 methods 모듈에서 ELF 제공
                Some(crate::methods::ETHREX_GUEST_RISC0_ELF)
            }
            #[cfg(feature = "zisk")]
            BackendType::ZisK => Some(crate::ZKVM_ZISK_PROGRAM_ELF),
            _ => None,
        }
    }

    fn vk_bytes(&self, backend: BackendType) -> Option<Vec<u8>> {
        match backend {
            #[cfg(feature = "sp1")]
            BackendType::SP1 => {
                // SP1 VK는 셋업 시 동적으로 생성되므로 여기서는 None.
                // 추후 빌드 시 생성된 VK 파일을 읽도록 확장 가능.
                None
            }
            #[cfg(feature = "risc0")]
            BackendType::RISC0 => {
                Some(crate::ZKVM_RISC0_PROGRAM_VK.as_bytes().to_vec())
            }
            _ => None,
        }
    }

    fn serialize_input(&self, raw_input: &[u8]) -> Result<Vec<u8>, GuestProgramError> {
        // EVM-L2는 기존과 동일하게 rkyv 직렬화를 사용.
        // raw_input이 이미 ProverInputData의 직렬화 형태이므로 그대로 전달.
        Ok(raw_input.to_vec())
    }

    fn encode_output(&self, raw_output: &[u8]) -> Result<Vec<u8>, GuestProgramError> {
        // EVM-L2 출력은 기존 ProgramOutput.encode()와 동일.
        // raw_output은 zkVM의 public values bytes.
        Ok(raw_output.to_vec())
    }

    fn program_type_id(&self) -> u8 {
        1 // EVM-L2
    }
}
```

> **해결하는 결합 지점**: C1 (ELF 정적 임베딩), C4 (feature flag 다형성)

## B. ProverBackend 변경

### 설계 결정

**핵심 선택: 트레이트 시그니처 변경 전략**

| 방식 | 설명 | 장점 | 단점 |
|------|------|------|------|
| 기존 트레이트 수정 | `prove(ProgramInput)` → `prove(elf, bytes)` | 깔끔한 인터페이스 | 기존 코드 전부 수정 필요 |
| 새 메서드 추가 | 기존 유지 + `prove_with_elf(elf, bytes)` 추가 | 점진적 마이그레이션 | 임시 중복 |
| 래퍼 레이어 | 기존 트레이트 위에 `ModularProver` 래퍼 | 기존 코드 무변경 | 추가 레이어 복잡성 |

**선택: 새 메서드 추가 → 점진적 기존 메서드 제거**

이유:
1. Phase 2.1에서 새 메서드를 추가하면서 기존 테스트를 통과시킨다.
2. Phase 2.2에서 모든 호출자를 새 메서드로 마이그레이션한 후 기존 메서드를 `#[deprecated]`로 표시한다.
3. 기존 `ExecBackend`는 ELF 없이 직접 실행하므로 특수 처리가 필요하다.

### 변경된 ProverBackend

```rust
// crates/l2/prover/src/backend/mod.rs (수정)

pub trait ProverBackend {
    type ProofOutput;
    type SerializedInput;

    fn prover_type(&self) -> ProverType;
    fn backend_type(&self) -> BackendType;

    // ---- 기존 메서드 (Phase 2.1에서 유지, Phase 2.2에서 deprecated) ----

    fn serialize_input(&self, input: &ProgramInput) -> Result<Self::SerializedInput, BackendError>;
    fn execute(&self, input: ProgramInput) -> Result<(), BackendError>;
    fn prove(&self, input: ProgramInput, format: ProofFormat) -> Result<Self::ProofOutput, BackendError>;
    fn verify(&self, proof: &Self::ProofOutput) -> Result<(), BackendError>;
    fn to_batch_proof(&self, proof: Self::ProofOutput, format: ProofFormat) -> Result<BatchProof, BackendError>;

    // ---- 새 메서드 (bytes 기반) ----

    /// ELF와 직렬화된 입력 bytes로 실행.
    /// Guest Program에 무관하게 동작한다.
    fn execute_with_elf(
        &self,
        elf: &[u8],
        serialized_input: &[u8],
    ) -> Result<(), BackendError> {
        // 기본 구현: 지원하지 않음 (기존 백엔드는 점진적으로 구현)
        Err(BackendError::not_implemented("execute_with_elf"))
    }

    /// ELF와 직렬화된 입력 bytes로 증명 생성.
    fn prove_with_elf(
        &self,
        elf: &[u8],
        serialized_input: &[u8],
        format: ProofFormat,
    ) -> Result<Self::ProofOutput, BackendError> {
        Err(BackendError::not_implemented("prove_with_elf"))
    }

    /// VK bytes와 증명을 사용하여 검증.
    fn verify_with_vk(
        &self,
        proof: &Self::ProofOutput,
        vk: &[u8],
    ) -> Result<(), BackendError> {
        Err(BackendError::not_implemented("verify_with_vk"))
    }

    // ... timed 버전도 동일하게 추가
}
```

### SP1 백엔드 변경 예시

```rust
// crates/l2/prover/src/backend/sp1.rs (수정)

impl Sp1Backend {
    /// ELF에서 proving key와 verifying key를 셋업.
    /// 기존 PROVER_SETUP 대신 ELF별 캐시 사용.
    fn setup_for_elf(&self, elf: &[u8]) -> Result<(SP1ProvingKey, SP1VerifyingKey), BackendError> {
        // 캐시: ELF hash → (pk, vk)
        // 동일 ELF에 대해 반복 셋업 방지
        let client = self.get_client();
        let (pk, vk) = client.setup(elf);
        Ok((pk, vk))
    }
}

impl ProverBackend for Sp1Backend {
    // ... 기존 메서드 유지 ...

    fn execute_with_elf(&self, elf: &[u8], serialized_input: &[u8], ) -> Result<(), BackendError> {
        let mut stdin = SP1Stdin::new();
        stdin.write_slice(serialized_input);

        let (_, _) = self.setup_for_elf(elf)?;
        let client = self.get_client();
        client.execute(elf, &stdin).map_err(BackendError::execution)?;
        Ok(())
    }

    fn prove_with_elf(
        &self,
        elf: &[u8],
        serialized_input: &[u8],
        format: ProofFormat,
    ) -> Result<Self::ProofOutput, BackendError> {
        let mut stdin = SP1Stdin::new();
        stdin.write_slice(serialized_input);

        let (pk, vk) = self.setup_for_elf(elf)?;
        let client = self.get_client();
        let sp1_format = Self::convert_format(format);
        let proof = client.prove(&pk, &stdin, sp1_format).map_err(BackendError::proving)?;
        Ok(Sp1ProveOutput::new(proof, vk))
    }
}
```

> **해결하는 결합 지점**: C2 (ProgramInput 직접 참조), C3 (ELF 상수 직접 참조)

## C. GuestProgramRegistry

### 설계

```rust
// crates/l2/prover/src/registry.rs (신규)

use std::collections::HashMap;
use std::sync::Arc;
use ethrex_guest_program::traits::GuestProgram;

/// Guest Program 런타임 레지스트리.
/// program_id로 GuestProgram 구현체를 조회한다.
pub struct GuestProgramRegistry {
    programs: HashMap<String, Arc<dyn GuestProgram>>,
    default_program_id: String,
}

impl GuestProgramRegistry {
    pub fn new(default_program_id: &str) -> Self {
        Self {
            programs: HashMap::new(),
            default_program_id: default_program_id.to_string(),
        }
    }

    /// Guest Program 등록.
    pub fn register(&mut self, program: Arc<dyn GuestProgram>) {
        self.programs.insert(program.program_id().to_string(), program);
    }

    /// program_id로 Guest Program 조회.
    pub fn get(&self, program_id: &str) -> Option<&Arc<dyn GuestProgram>> {
        self.programs.get(program_id)
    }

    /// 기본 Guest Program 조회 (program_id가 지정되지 않은 배치용).
    pub fn default_program(&self) -> Option<&Arc<dyn GuestProgram>> {
        self.programs.get(&self.default_program_id)
    }

    /// 등록된 모든 program_id 목록.
    pub fn program_ids(&self) -> Vec<&str> {
        self.programs.keys().map(|s| s.as_str()).collect()
    }
}

/// 기본 레지스트리 생성: EVM-L2만 등록.
pub fn default_registry() -> GuestProgramRegistry {
    let mut registry = GuestProgramRegistry::new("evm-l2");
    registry.register(Arc::new(EvmL2GuestProgram));
    registry
}
```

### 프루버에서의 사용

```rust
// crates/l2/prover/src/prover.rs (수정)

struct Prover<B: ProverBackend> {
    backend: B,
    registry: GuestProgramRegistry,  // 추가
    // ...
}

impl<B: ProverBackend> Prover<B> {
    async fn prove_batch(&self, prover_data: ProverData) -> Result<BatchProof, BackendError> {
        let program_id = prover_data.program_id.as_deref().unwrap_or("evm-l2");
        let program = self.registry.get(program_id)
            .ok_or(BackendError::unknown_program(program_id))?;

        let elf = program.elf(self.backend.backend_type())
            .ok_or(BackendError::unsupported_backend(program_id))?;

        let serialized = program.serialize_input(&prover_data.raw_input)?;

        let proof = self.backend.prove_with_elf(elf, &serialized, prover_data.format)?;
        self.backend.to_batch_proof(proof, prover_data.format)
    }
}
```

> **해결하는 결합 지점**: C1 (ELF 정적 임베딩 — 레지스트리에서 동적 조회)

## D. ProofData 프로토콜 확장

### 변경

```rust
// crates/l2/common/src/prover.rs (수정)

pub enum ProofData {
    BatchRequest {
        commit_hash: String,
        prover_type: ProverType,
        supported_programs: Vec<String>,  // 추가: 이 프루버가 지원하는 program_id 목록
    },

    BatchResponse {
        batch_number: Option<u64>,
        input: Option<ProverInputData>,
        format: Option<ProofFormat>,
        program_id: Option<String>,  // 추가: 이 배치에 사용할 Guest Program
    },

    ProofSubmit {
        batch_number: u64,
        batch_proof: BatchProof,
        program_id: String,  // 추가: 증명에 사용된 Guest Program
    },

    // ... 나머지 동일
}
```

### 하위 호환성

- `supported_programs`가 비어 있으면 기존 동작과 동일 (`"evm-l2"` 가정).
- `program_id`가 `None`이면 기존 동작과 동일.
- 직렬화 포맷(`serde_json`)에서 새 필드에 `#[serde(default)]`를 사용하여 이전 버전 프루버와의 호환성 유지.

```rust
#[derive(Serialize, Deserialize)]
pub enum ProofData {
    BatchRequest {
        commit_hash: String,
        prover_type: ProverType,
        #[serde(default)]
        supported_programs: Vec<String>,
    },
    // ...
}
```

> **해결하는 결합 지점**: C7 (ProofData에 program_id 없음), C8 (ProverInputData L2 전용)

## E. L1 컨트랙트 변경

### 설계 결정

**VK 매핑 확장 전략**

현재:
```solidity
mapping(bytes32 commitHash => mapping(uint8 verifierId => bytes32 vk))
    public verificationKeys;
```

목표:
```solidity
mapping(bytes32 commitHash => mapping(uint8 programTypeId => mapping(uint8 verifierId => bytes32 vk)))
    public verificationKeys;
```

| `programTypeId` | Guest Program |
|-----------------|---------------|
| 1 | EVM-L2 (기본) |
| 2 | Transfer |
| 3 | DEX |
| 4-255 | 향후 확장 |

### BatchCommitmentInfo 확장

```solidity
struct BatchCommitmentInfo {
    bytes32 newStateRoot;
    bytes32 blobKZGVersionedHash;
    bytes32 processedPrivilegedTransactionsRollingHash;
    bytes32 withdrawalsLogsMerkleRoot;
    bytes32 lastBlockHash;
    uint256 nonPrivilegedTransactions;
    ICommonBridge.BalanceDiff[] balanceDiffs;
    bytes32 commitHash;
    ICommonBridge.L2MessageRollingHash[] l2InMessageRollingHashes;
    uint8 programTypeId;  // 추가
}
```

### commitBatch() 변경

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
    ICommonBridge.L2MessageRollingHash[] calldata l2MessageRollingHashes,
    uint8 programTypeId  // 추가
) external override onlyOwner whenNotPaused {
    // VK 검증: programTypeId 포함
    if (REQUIRE_SP1_PROOF &&
        verificationKeys[commitHash][programTypeId][SP1_VERIFIER_ID] == bytes32(0)) {
        revert("013");
    }

    // BatchCommitmentInfo에 programTypeId 저장
    batchCommitments[batchNumber] = BatchCommitmentInfo(
        newStateRoot, blobVersionedHash, ...,
        programTypeId  // 추가
    );
}
```

### verifyBatch() 변경

```solidity
function verifyBatch(uint256 batchNumber, bytes memory risc0Proof, bytes memory sp1Proof, bytes memory tdxSig)
    external override onlyOwner whenNotPaused
{
    uint8 progTypeId = batchCommitments[batchNumber].programTypeId;

    // public inputs 재구성: 프로그램 타입에 따라 분기
    bytes memory publicInputs = _getPublicInputsFromCommitment(batchNumber, progTypeId);

    if (REQUIRE_SP1_PROOF) {
        bytes32 sp1Vk = verificationKeys[batchCommitHash][progTypeId][SP1_VERIFIER_ID];
        ISP1Verifier(SP1_VERIFIER_ADDRESS).verifyProof(sp1Vk, publicInputs, sp1Proof);
    }
    // RISC0, TDX 동일 패턴
}
```

### _getPublicInputsFromCommitment() 확장

```solidity
function _getPublicInputsFromCommitment(
    uint256 batchNumber,
    uint8 programTypeId
) internal view returns (bytes memory) {
    if (programTypeId == 1) {
        // EVM-L2: 기존 인코딩 유지
        return _getEvmL2PublicInputs(batchNumber);
    } else if (programTypeId == 2) {
        // Transfer: 간소화된 인코딩
        return _getTransferPublicInputs(batchNumber);
    } else {
        revert("Unknown program type");
    }
}
```

### 하위 호환성

컨트랙트는 UUPS Proxy 패턴(`UUPSUpgradeable`)을 이미 사용하고 있으므로:
1. 새 구현을 배포한다.
2. Timelock을 통해 `upgradeTo()`를 호출한다.
3. 기존 배치(programTypeId 없음)는 기본값 `1` (EVM-L2)로 처리한다.

스토리지 레이아웃 변경:
- `verificationKeys` 매핑의 차원이 2 → 3으로 바뀌므로 **스토리지 슬롯이 변경**된다.
- 마이그레이션 함수에서 기존 `verificationKeys[hash][verifierId]` 데이터를 `verificationKeys[hash][1][verifierId]`로 이전해야 한다.

> **해결하는 결합 지점**: C5 (출력 인코딩 하드코딩), C6 (VK 매핑 2차원), C9 (verifyBatch 고정 시그니처)

## F. 멀티 ELF 빌드 시스템

### 디렉토리 구조

```
crates/guest-program/
├── programs/                    # Guest Program 크레이트 디렉토리 (신규)
│   ├── evm-l2/                  # 기존 L2 Guest Program
│   │   ├── Cargo.toml
│   │   └── src/
│   │       └── main.rs          # 기존 l2/program.rs의 엔트리포인트
│   └── transfer/                # Transfer Circuit (레퍼런스)
│       ├── Cargo.toml
│       └── src/
│           └── main.rs
├── bin/                         # zkVM별 빌드 출력 (구조 변경)
│   ├── sp1/
│   │   ├── evm-l2/out/          # EVM-L2 SP1 ELF
│   │   └── transfer/out/        # Transfer SP1 ELF
│   └── risc0/
│       ├── evm-l2/out/
│       └── transfer/out/
├── build.rs                     # 확장: 멀티 프로그램 빌드
└── src/
    ├── lib.rs                   # 리팩토링: 트레이트 + 레지스트리 re-export
    ├── traits.rs                # GuestProgram 트레이트 (신규)
    └── programs/                # GuestProgram 구현체 (신규)
        ├── mod.rs
        └── evm_l2.rs            # EvmL2GuestProgram
```

### build.rs 확장

```rust
// crates/guest-program/build.rs (수정)

/// 빌드할 Guest Program 목록.
/// 환경변수 GUEST_PROGRAMS로 지정하거나, 기본값은 "evm-l2".
fn get_programs_to_build() -> Vec<String> {
    match std::env::var("GUEST_PROGRAMS") {
        Ok(programs) => programs.split(',').map(|s| s.trim().to_string()).collect(),
        Err(_) => vec!["evm-l2".to_string()],
    }
}

fn main() {
    let programs = get_programs_to_build();

    for program_name in &programs {
        let program_dir = format!("./programs/{}", program_name);

        #[cfg(all(not(clippy), feature = "sp1"))]
        build_sp1_program_for(&program_dir, program_name);

        #[cfg(all(not(clippy), feature = "risc0"))]
        build_risc0_program_for(&program_dir, program_name);

        // ... zisk, openvm
    }
}

#[cfg(all(not(clippy), feature = "sp1"))]
fn build_sp1_program_for(program_dir: &str, program_name: &str) {
    let output_dir = format!("./bin/sp1/{}/out", program_name);
    let elf_name = format!("{}-sp1-elf", program_name);

    sp1_build::build_program_with_args(
        program_dir,
        sp1_build::BuildArgs {
            output_directory: Some(output_dir),
            elf_name: Some(elf_name),
            // ... 기존 설정 유지
            ..Default::default()
        },
    );
}
```

> **해결하는 결합 지점**: C10 (빌드 스크립트 단일 프로그램)

## G. 전체 데이터 흐름

### 현재 흐름

```
1. Proof Coordinator
   └─ ProverInputData (L2 전용 필드)
       └─ TCP 전송
           └─ Prover
               └─ ProverInputData → ProgramInput 변환 (#[cfg(feature = "l2")])
                   └─ ProverBackend.prove(ProgramInput)
                       └─ rkyv 직렬화 → ELF 실행 (정적 ELF)
                           └─ BatchProof
                               └─ TCP 제출
                                   └─ L1 ProofSender
                                       └─ verifyBatch(batchNumber, risc0, sp1, tdx)
```

### 목표 흐름

```
1. Proof Coordinator
   └─ BatchResponse { input, program_id: "evm-l2" }
       └─ TCP 전송
           └─ Prover
               └─ GuestProgramRegistry.get("evm-l2")
                   └─ program.elf(SP1) → ELF bytes
                   └─ program.serialize_input(raw_input) → serialized bytes
                       └─ ProverBackend.prove_with_elf(elf, serialized, format)
                           └─ BatchProof
                               └─ ProofSubmit { batch_proof, program_id: "evm-l2" }
                                   └─ TCP 제출
                                       └─ L1 ProofSender
                                           └─ verifyBatch(batchNumber, risc0, sp1, tdx)
                                               └─ OnChainProposer._getPublicInputsFromCommitment(batch, programTypeId)
```

### 변경 최소화 원칙

1. **Proof Coordinator**: `program_id`를 `BatchResponse`에 추가하되, 기본값은 `"evm-l2"`. 기존 배치는 변경 없음.
2. **Prover**: `GuestProgramRegistry`를 통해 ELF를 조회하되, 레지스트리에 `EvmL2GuestProgram`만 등록하면 기존과 동일하게 동작.
3. **L1 ProofSender**: `program_id` → `programTypeId` 변환 후 `commitBatch()`에 전달.
4. **OnChainProposer**: 마이그레이션 함수에서 기존 VK를 새 매핑으로 이전.

## H. 설계 결정 요약

| # | 결정 | 선택 | 대안 | 이유 |
|---|------|------|------|------|
| D1 | 트레이트 추상화 수준 | bytes 수준 | 제네릭 타입 | 타입 폭발 방지, ProverBackend 변경 최소화 |
| D2 | ProverBackend 변경 방식 | 새 메서드 추가 | 시그니처 수정 | 점진적 마이그레이션, 기존 테스트 유지 |
| D3 | 레지스트리 구현 | HashMap 기반 | Enum dispatch | 런타임 확장성, 플러그인 지원 |
| D4 | L1 VK 매핑 | 3차원 매핑 | 복합 키 해싱 | 명시적 구조, 가스 예측 가능 |
| D5 | 빌드 시스템 | 환경변수 기반 선택 | 설정 파일 | 기존 빌드 인프라와의 호환성 |
| D6 | 하위 호환성 | serde default + 기본값 | 프로토콜 버전 | 점진적 마이그레이션 |
| D7 | ELF 로딩 | 레지스트리에서 bytes 제공 | 파일 시스템 동적 로딩 | 보안 (임의 ELF 로딩 방지) |
