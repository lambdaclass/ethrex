# 구현 단계 계획

이 문서는 Guest Program 모듈화 구현을 4개 하위 단계(Phase 2.1-2.4)로 나누어 상세하게 기술한다.
각 단계는 독립적으로 테스트 가능한 마일스톤을 가진다.

> **마지막 업데이트**: 2026-02-22
> Phase 2.1 ✅ | Phase 2.2 ✅ | Phase 2.3 ✅ | Phase 2.4 ✅ | Phase 3 ✅

---

## Phase 2.1: 코어 추상화 ✅ 완료

### 목표
- `GuestProgram` 트레이트를 정의한다.
- 기존 EVM-L2 코드를 `EvmL2GuestProgram`으로 래핑한다.
- `ProverBackend`에 bytes 기반 메서드를 추가한다.
- 모든 백엔드(SP1, RISC0, ZisK, OpenVM, Exec)에 새 메서드 추가.
- **모든 기존 테스트가 통과해야 한다 (zero behavior change).**

### 실제 구현 내용

#### 2.1.1 GuestProgram 트레이트 정의 ✅

**파일**: `crates/guest-program/src/traits.rs` (신규)

> **설계 결정**: `BackendType` 순환 의존성 문제는 **Option B**로 해결 — `&str` 상수를 사용하여 백엔드를 식별한다.
> `backends` 모듈에 `SP1`, `RISC0`, `ZISK`, `OPENVM`, `EXEC` 상수를 정의.

```rust
pub mod backends {
    pub const SP1: &str = "sp1";
    pub const RISC0: &str = "risc0";
    pub const ZISK: &str = "zisk";
    pub const OPENVM: &str = "openvm";
    pub const EXEC: &str = "exec";
}

pub trait GuestProgram: Send + Sync {
    fn program_id(&self) -> &str;
    fn elf(&self, backend: &str) -> Option<&[u8]>;
    fn vk_bytes(&self, backend: &str) -> Option<Vec<u8>>;
    fn program_type_id(&self) -> u8;
    fn serialize_input(&self, raw_input: &[u8]) -> Result<Vec<u8>, GuestProgramError>;
    fn encode_output(&self, raw_output: &[u8]) -> Result<Vec<u8>, GuestProgramError>;
}
```

`serialize_input`과 `encode_output`은 기본 구현(identity pass-through)을 제공.

변경 파일:
| 파일 | 변경 내용 |
|------|----------|
| `crates/guest-program/src/traits.rs` | **신규** — `backends` 모듈, `GuestProgramError`, `GuestProgram` 트레이트 |
| `crates/guest-program/src/lib.rs` | `pub mod traits;` 추가 |

#### 2.1.2 EvmL2GuestProgram 구현 ✅

**파일**: `crates/guest-program/src/programs/evm_l2.rs` (신규)

```rust
pub struct EvmL2GuestProgram;

impl GuestProgram for EvmL2GuestProgram {
    fn program_id(&self) -> &str { "evm-l2" }
    fn program_type_id(&self) -> u8 { 1 }

    fn elf(&self, backend: &str) -> Option<&[u8]> {
        match backend {
            backends::SP1 => { /* cfg(feature="sp1") 조건부 include_bytes */ },
            backends::RISC0 => { /* cfg(feature="risc0") 조건부 */ },
            backends::ZISK => { /* cfg(feature="zisk") 조건부 */ },
            _ => None,
        }
    }
    // serialize_input, encode_output: 기본 identity 사용
}
```

6개 단위 테스트 포함 (`program_id_is_evm_l2`, `program_type_id_is_one`, `unknown_backend_returns_none`, `serialize_input_is_identity`, `encode_output_is_identity`, `non_empty_filters_sentinels`).

변경 파일:
| 파일 | 변경 내용 |
|------|----------|
| `crates/guest-program/src/programs/mod.rs` | **신규** — `pub mod evm_l2;` |
| `crates/guest-program/src/programs/evm_l2.rs` | **신규** — EvmL2GuestProgram |
| `crates/guest-program/src/lib.rs` | `pub mod programs;` 추가 |

#### 2.1.3 ProverBackend에 bytes 기반 메서드 추가 ✅

**파일**: `crates/l2/prover/src/backend/mod.rs` (수정)

```rust
pub trait ProverBackend {
    // ... 기존 메서드 유지 ...
    fn backend_name(&self) -> &'static str;  // backends:: 상수 반환

    // ELF-based methods (기본 구현: NotImplemented 에러)
    fn execute_with_elf(&self, elf: &[u8], serialized_input: &[u8]) -> Result<(), BackendError>;
    fn prove_with_elf(&self, elf: &[u8], serialized_input: &[u8], format: ProofFormat)
        -> Result<Self::ProofOutput, BackendError>;
    fn execute_with_elf_timed(...) -> Result<Duration, BackendError>;
    fn prove_with_elf_timed(...) -> Result<(Self::ProofOutput, Duration), BackendError>;
}
```

`BackendType`에 `as_backend_name()` 메서드 추가하여 enum → `&str` 변환 지원.

변경 파일:
| 파일 | 변경 내용 |
|------|----------|
| `crates/l2/prover/src/backend/mod.rs` | `backend_name()`, `execute_with_elf()`, `prove_with_elf()`, timed 변형 추가. `BackendType::as_backend_name()` 추가 |
| `crates/l2/prover/src/backend/error.rs` | `BackendError::NotImplemented` 변형, `not_implemented()`, `serialization()` 생성자 추가 |

#### 2.1.4 모든 백엔드 수정 ✅

5개 백엔드 모두 `backend_name()` 구현 완료. ELF 메서드는 기본 `NotImplemented` 반환 (Exec만 실제 구현).

| 파일 | 변경 내용 |
|------|----------|
| `crates/l2/prover/src/backend/sp1.rs` | `backend_name()` → `backends::SP1` |
| `crates/l2/prover/src/backend/risc0.rs` | `backend_name()` → `backends::RISC0` |
| `crates/l2/prover/src/backend/zisk.rs` | `backend_name()` → `backends::ZISK` |
| `crates/l2/prover/src/backend/openvm.rs` | `backend_name()` → `backends::OPENVM` |
| `crates/l2/prover/src/backend/exec.rs` | `backend_name()` → `backends::EXEC`, `execute_with_elf()` 및 `prove_with_elf()` 실제 구현 |

> **계획 대비 차이점**:
> - 원래 계획은 SP1/RISC0만 수정 예정이었으나, 실제로는 5개 백엔드 전부에 `backend_name()` 추가.
> - `verify_with_vk()`는 구현하지 않음 (현재 필요 없음).
> - `backend_type()` 대신 `backend_name()` 사용 — 순환 의존성 회피.

### Phase 2.1 완료 기준

- [x] `GuestProgram` 트레이트가 정의되고 `EvmL2GuestProgram`이 구현됨
- [x] `ProverBackend`에 `prove_with_elf()` 등 새 메서드가 추가됨
- [x] 5개 백엔드 모두 `backend_name()` 구현
- [x] Exec 백엔드 `execute_with_elf()`, `prove_with_elf()` 실제 구현
- [x] **모든 기존 테스트 통과** (cargo test)
- [x] 6개 EvmL2GuestProgram 단위 테스트 추가

---

## Phase 2.2: 레지스트리 & 멀티프로그램 ✅ 완료

### 목표
- `GuestProgramRegistry` 구현
- `ProofData` 프로토콜에 `program_id` 추가
- Proof Coordinator가 프로그램별 배치 할당
- Prover 메인 루프에 dual-path proving 통합

### 실제 구현 내용

#### 2.2.1 GuestProgramRegistry 구현 ✅

**파일**: `crates/l2/prover/src/registry.rs` (신규)

```rust
pub struct GuestProgramRegistry {
    programs: HashMap<String, Arc<dyn GuestProgram>>,
    default_program_id: String,
}

impl GuestProgramRegistry {
    pub fn new(default_program_id: &str) -> Self;
    pub fn register(&mut self, program: Arc<dyn GuestProgram>);
    pub fn get(&self, program_id: &str) -> Option<&Arc<dyn GuestProgram>>;
    pub fn default_program(&self) -> Option<&Arc<dyn GuestProgram>>;
    pub fn default_program_id(&self) -> &str;
    pub fn program_ids(&self) -> Vec<&str>;
}
```

5개 단위 테스트: `register_and_lookup`, `default_program`, `default_program_missing`, `duplicate_registration_replaces`, `program_ids`.

변경 파일:
| 파일 | 변경 내용 |
|------|----------|
| `crates/l2/prover/src/registry.rs` | **신규** — GuestProgramRegistry |
| `crates/l2/prover/src/lib.rs` | `pub mod registry;` 추가 |

#### 2.2.2 ProofData 프로토콜 확장 ✅

**파일**: `crates/l2/common/src/prover.rs` (수정)

```rust
pub enum ProofData {
    BatchRequest {
        commit_hash: String,
        prover_type: ProverType,
        #[serde(default)]
        supported_programs: Vec<String>,  // 추가
    },
    BatchResponse {
        batch_number: Option<u64>,
        input: Option<ProverInputData>,
        format: Option<ProofFormat>,
        #[serde(default)]
        program_id: Option<String>,  // 추가
    },
    ProofSubmit {
        batch_number: u64,
        batch_proof: BatchProof,
        #[serde(default = "default_program_id")]
        program_id: String,  // 추가
    },
    // ...
}
```

편의 메서드 추가: `batch_request_with_programs()`, `batch_response_with_program()`, `proof_submit_with_program()`.

하위 호환성: `#[serde(default)]` 사용으로 이전 프루버와 통신 가능.

#### 2.2.3 Proof Coordinator 수정 ✅

**파일**: `crates/l2/sequencer/proof_coordinator.rs` (수정)

- `handle_connection()`: `BatchRequest`에서 `supported_programs` 파싱
- `handle_request()`: `supported_programs` 필터링 — 빈 리스트면 모든 프로그램 수용
- `determine_program_for_batch()`: 현재 하드코딩 `"evm-l2"` (향후 배치별 매핑 추가 예정)
- `ProofSubmit` 수신 시 `program_id` 포함

#### 2.2.4 Prover 메인 루프 수정 ✅

**파일**: `crates/l2/prover/src/prover.rs` (수정)

**Dual-path proving 아키텍처**:

```rust
fn prove_batch(&self, input: ProgramInput, format: ProofFormat,
               batch_number: u64, program_id: &str) -> Result<BatchProof, BackendError> {
    // 1차: 레지스트리에서 ELF 조회
    if let Some((program, elf)) = registry.get(program_id).and_then(|p| p.elf(backend_name)) {
        // ELF 경로: serialize → prove_with_elf
        let serialized = program.serialize_input(rkyv::to_bytes(&input))?;
        self.backend.prove_with_elf(elf, &serialized, format)
    } else {
        // 레거시 경로: prove(ProgramInput) 직접 호출
        self.backend.prove(input, format)
    }
}
```

`create_default_registry()`:
```rust
fn create_default_registry() -> GuestProgramRegistry {
    let mut registry = GuestProgramRegistry::new("evm-l2");
    registry.register(Arc::new(EvmL2GuestProgram));
    registry.register(Arc::new(ZkDexGuestProgram));      // Phase 2.4에서 추가
    registry.register(Arc::new(TokammonGuestProgram));    // Phase 2.4에서 추가
    registry
}
```

`request_new_input()`: `supported_programs` 리스트를 coordinator에 전송, 응답에서 `program_id` 수신.

> **계획 대비 차이점**:
> - Transfer Circuit 레퍼런스는 구현하지 않음 — 대신 ZK-DEX와 Tokamon 앱 특화 템플릿으로 대체 (06-app-specific-templates.md 참조).
> - `crates/guest-program/programs/transfer/` 디렉토리 미생성.
> - `build.rs` 수정 없음 (멀티 ELF 빌드는 향후 과제).

### Phase 2.2 완료 기준

- [x] `GuestProgramRegistry`가 동작하고 `EvmL2GuestProgram`이 등록됨
- [x] `ProofData` 프로토콜에 `program_id`가 추가됨 (하위 호환)
- [x] Proof Coordinator가 `program_id`를 포함한 응답을 전송
- [x] Prover가 레지스트리를 통해 ELF 조회 시도 후 레거시 fallback
- [x] 5개 레지스트리 단위 테스트 통과
- [ ] ~~Transfer Circuit 레퍼런스~~ → ZK-DEX/Tokamon 스텁으로 대체 (Phase 2.4)

---

## Phase 2.3: L1 컨트랙트 & 검증 ✅ 완료

### 목표
- `OnChainProposer.sol`의 VK 매핑에 `programTypeId` 추가
- `verifyBatch()` 프로그램 타입별 검증 디스패치
- `l1_committer.rs` 수정
- based 변형 컨트랙트도 동일 수정

### 실제 구현 내용

#### 2.3.1 OnChainProposer.sol 수정 ✅

**파일**: `crates/l2/contracts/src/l1/OnChainProposer.sol` (수정)

VK 매핑 2D → 3D:
```solidity
// 기존: mapping(bytes32 => mapping(uint8 => bytes32))
// 변경:
mapping(bytes32 commitHash => mapping(uint8 programTypeId => mapping(uint8 verifierId => bytes32 vk)))
    public verificationKeys;
```

`BatchCommitmentInfo`에 `programTypeId` 추가:
```solidity
struct BatchCommitmentInfo {
    // ... 기존 필드 ...
    uint8 programTypeId;  // 추가 (구조체 끝에 배치하여 스토리지 레이아웃 영향 최소화)
}
```

`DEFAULT_PROGRAM_TYPE_ID = 1` 상수 추가.

하위 호환성: `programTypeId == 0`이면 자동으로 `DEFAULT_PROGRAM_TYPE_ID (1)`로 매핑.

```solidity
function commitBatch(
    uint256 batchNumber, bytes32 newStateRoot, ...,
    bytes32 commitHash, uint8 programTypeId,  // 추가
    ...
) external override onlyOwner whenNotPaused {
    uint8 effectiveProgramTypeId = programTypeId == 0
        ? DEFAULT_PROGRAM_TYPE_ID : programTypeId;
    // VK 검증: verificationKeys[commitHash][effectiveProgramTypeId][verifierId]
    // BatchCommitmentInfo 저장 시 effectiveProgramTypeId 포함
}
```

`verifyBatch()` 및 `verifyBatchesAligned()`: 저장된 `batchCommitments[batchNumber].programTypeId`로 VK 조회.

`initialize()`: 제네시스 배치를 `DEFAULT_PROGRAM_TYPE_ID`로 초기화, VK를 3D 매핑에 저장.

#### 2.3.2 upgradeVerificationKey 범용화 ✅

```solidity
function upgradeVerificationKey(
    bytes32 commit_hash, uint8 programTypeId, uint8 verifierId, bytes32 new_vk
) public onlyOwner {
    require(commit_hash != bytes32(0));
    require(programTypeId > 0);
    require(verifierId > 0 && verifierId <= 2);
    verificationKeys[commit_hash][programTypeId][verifierId] = new_vk;
    emit VerificationKeyUpgraded(programTypeId, verifierId, commit_hash, new_vk);
}
```

기존 `upgradeSP1VerificationKey()`와 `upgradeRISC0VerificationKey()`도 3D 매핑 사용하도록 수정 (하위 호환 유지, `DEFAULT_PROGRAM_TYPE_ID` 사용).

#### 2.3.3 인터페이스 업데이트 ✅

**파일**: `crates/l2/contracts/src/l1/interfaces/IOnChainProposer.sol` (수정)

- `commitBatch` 시그니처에 `uint8 programTypeId` 추가
- `VerificationKeyUpgraded` 이벤트 오버로드 추가 (`programTypeId`, `verifierId` 포함)
- `upgradeVerificationKey` 함수 선언 추가

#### 2.3.4 Based 변형 동일 수정 ✅

**파일**: `crates/l2/contracts/src/l1/based/OnChainProposer.sol` (수정)
**파일**: `crates/l2/contracts/src/l1/based/interfaces/IOnChainProposer.sol` (수정)

메인 OnChainProposer와 동일한 변경 적용:
- 3D VK 매핑
- `BatchCommitmentInfo.programTypeId`
- `commitBatch()` 시그니처 확장
- VK 조회 로직
- `upgradeVerificationKey()` 범용 함수

#### 2.3.5 l1_committer.rs 수정 ✅

**파일**: `crates/l2/sequencer/l1_committer.rs` (수정)

ABI 함수 시그니처 업데이트 (`uint8` 추가):
```rust
const COMMIT_FUNCTION_SIGNATURE_BASED: &str =
    "commitBatch(uint256,bytes32,bytes32,bytes32,bytes32,uint256,bytes32,uint8,bytes[])";
const COMMIT_FUNCTION_SIGNATURE: &str =
    "commitBatch(uint256,bytes32,bytes32,bytes32,bytes32,uint256,bytes32,uint8,(...)[],(...)[])";
```

`send_commitment()`에 `program_type_id: u8 = 1` 하드코딩, calldata에 포함.

#### 2.3.6 l1_proof_sender.rs ✅ 수정 불필요

`verifyBatch()` 시그니처 변경 없음. `programTypeId`는 `batchCommitments`에 저장되어 있으므로
L1에서 자동으로 올바른 VK를 조회.

#### 2.3.7 배포 스크립트 ✅ 수정 불필요

`initialize()` 시그니처 변경 없음 — 내부적으로 `DEFAULT_PROGRAM_TYPE_ID` 사용.

> **계획 대비 차이점**:
> - VK 마이그레이션 함수 (`migrateVerificationKeys`)는 구현하지 않음 — feature branch에서 스토리지 레이아웃이 완전히 변경되므로 마이그레이션 불필요. 프로덕션 배포 시 별도 고려.
> - `l1_proof_sender.rs` 수정 불필요 확인 — `verifyBatch()` 시그니처 불변.
> - 배포 스크립트 (`deployer.rs`) 수정 불필요 확인 — `initialize()` 시그니처 불변.

### Phase 2.3 완료 기준

- [x] `OnChainProposer.sol`이 3차원 VK 매핑을 사용
- [x] `commitBatch()`가 `programTypeId`를 받고 저장
- [x] `verifyBatch()`가 `programTypeId`에 따라 VK를 조회
- [x] `verifyBatchesAligned()`도 동일하게 VK 조회
- [ ] ~~VK 마이그레이션 함수~~ → feature branch이므로 불필요
- [x] `l1_committer`가 `programTypeId`를 전송
- [x] based 변형 컨트랙트도 동일 수정
- [x] `cargo check -p ethrex-l2` 컴파일 통과
- [x] 기존 EVM-L2 배치가 `programTypeId=1`로 정상 동작 (하위 호환)

---

## Phase 2.4: 앱 특화 템플릿 & 개발자 도구 ✅ 완료

### 목표
- 앱 특화 Guest Program 스텁 생성 (ZK-DEX, Tokamon)
- 레지스트리에 등록
- Exec 백엔드 ELF 경로 구현
- 개발자 문서

### 실제 구현 내용

#### 2.4.1 ZkDexGuestProgram 스텁 ✅

**파일**: `crates/guest-program/src/programs/zk_dex.rs` (신규)

```rust
pub struct ZkDexGuestProgram;

impl GuestProgram for ZkDexGuestProgram {
    fn program_id(&self) -> &str { "zk-dex" }
    fn program_type_id(&self) -> u8 { 2 }
    fn elf(&self, _backend: &str) -> Option<&[u8]> { None }
    fn vk_bytes(&self, _backend: &str) -> Option<Vec<u8>> { None }
    // serialize_input, encode_output: identity pass-through
}
```

참조: [tokamak-network/zk-dex](https://github.com/tokamak-network/zk-dex/tree/circom)
상세 설계: `06-app-specific-templates.md` 섹션 2

4개 단위 테스트 포함.

#### 2.4.2 TokammonGuestProgram 스텁 ✅

**파일**: `crates/guest-program/src/programs/tokamon.rs` (신규)

```rust
pub struct TokammonGuestProgram;

impl GuestProgram for TokammonGuestProgram {
    fn program_id(&self) -> &str { "tokamon" }
    fn program_type_id(&self) -> u8 { 3 }
    fn elf(&self, _backend: &str) -> Option<&[u8]> { None }
    fn vk_bytes(&self, _backend: &str) -> Option<Vec<u8>> { None }
    // serialize_input, encode_output: identity pass-through
}
```

참조: [tokamak-network/tokamon](https://github.com/tokamak-network/tokamon/tree/deploy/thanos-sepolia)
상세 설계: `06-app-specific-templates.md` 섹션 3

4개 단위 테스트 포함.

#### 2.4.3 모듈 등록 및 레지스트리 ✅

**파일**: `crates/guest-program/src/programs/mod.rs` (수정)

```rust
mod evm_l2;
mod tokamon;
mod zk_dex;

pub use evm_l2::EvmL2GuestProgram;
pub use tokamon::TokammonGuestProgram;
pub use zk_dex::ZkDexGuestProgram;
```

**파일**: `crates/l2/prover/src/prover.rs` (수정)

`create_default_registry()`에 3개 프로그램 등록:
```rust
fn create_default_registry() -> GuestProgramRegistry {
    let mut registry = GuestProgramRegistry::new("evm-l2");
    registry.register(Arc::new(EvmL2GuestProgram));
    registry.register(Arc::new(ZkDexGuestProgram));
    registry.register(Arc::new(TokammonGuestProgram));
    registry
}
```

#### 2.4.4 Exec 백엔드 execute_with_elf / prove_with_elf ✅

**파일**: `crates/l2/prover/src/backend/exec.rs` (수정)

```rust
fn execute_with_elf(&self, _elf: &[u8], serialized_input: &[u8]) -> Result<(), BackendError> {
    let input: ProgramInput = rkyv::from_bytes::<ProgramInput, rkyv::rancor::Error>(serialized_input)
        .map_err(|e| BackendError::serialization(e.to_string()))?;
    Self::execute_core(input)?;
    Ok(())
}

fn prove_with_elf(&self, _elf: &[u8], serialized_input: &[u8], _format: ProofFormat)
    -> Result<Self::ProofOutput, BackendError>
{
    let input: ProgramInput = rkyv::from_bytes::<ProgramInput, rkyv::rancor::Error>(serialized_input)
        .map_err(|e| BackendError::serialization(e.to_string()))?;
    Self::execute_core(input)
}
```

#### 2.4.5 미완료 항목

- [ ] Guest Program 생성 스크립트 (`scripts/new-guest-program.sh`) — 향후 구현
- [ ] 멀티 ELF 빌드 도구 (`GUEST_PROGRAMS` 환경변수) — 향후 구현
- [ ] 개발자 가이드 문서 (`docs/l2/guest-program-development.md`) — 향후 구현
- [ ] ZK-DEX/Tokamon의 실제 zkVM 엔트리포인트 (ELF) 구현
- [ ] 각 프로그램의 입력/출력 타입 정의 (`ZkDexInput`, `TokammonInput` 등)

### Phase 2.4 완료 기준

- [x] ZkDexGuestProgram 스텁 구현 (program_id="zk-dex", type_id=2)
- [x] TokammonGuestProgram 스텁 구현 (program_id="tokamon", type_id=3)
- [x] `programs/mod.rs`에서 export
- [x] `create_default_registry()`에 3개 프로그램 등록
- [x] Exec 백엔드 `execute_with_elf()`, `prove_with_elf()` 구현
- [x] 14개 guest-program 테스트 + 5개 registry 테스트 통과 (총 19개)
- [ ] 생성 스크립트 / 빌드 도구
- [ ] 개발자 문서
- [ ] 실제 ELF 바이너리 구현

---

## 전체 파일 변경 요약 (실제)

| 파일 | Phase | 변경 유형 | 상태 |
|------|-------|----------|------|
| `crates/guest-program/src/traits.rs` | 2.1 | **신규** | ✅ |
| `crates/guest-program/src/programs/mod.rs` | 2.1, 2.4 | **신규** | ✅ |
| `crates/guest-program/src/programs/evm_l2.rs` | 2.1 | **신규** | ✅ |
| `crates/guest-program/src/programs/zk_dex.rs` | 2.4 | **신규** | ✅ |
| `crates/guest-program/src/programs/tokamon.rs` | 2.4 | **신규** | ✅ |
| `crates/guest-program/src/lib.rs` | 2.1 | 수정 | ✅ |
| `crates/l2/prover/src/backend/mod.rs` | 2.1 | 수정 | ✅ |
| `crates/l2/prover/src/backend/sp1.rs` | 2.1 | 수정 | ✅ |
| `crates/l2/prover/src/backend/risc0.rs` | 2.1 | 수정 | ✅ |
| `crates/l2/prover/src/backend/zisk.rs` | 2.1 | 수정 | ✅ |
| `crates/l2/prover/src/backend/openvm.rs` | 2.1 | 수정 | ✅ |
| `crates/l2/prover/src/backend/exec.rs` | 2.1, 2.4 | 수정 | ✅ |
| `crates/l2/prover/src/backend/error.rs` | 2.1 | 수정 | ✅ |
| `crates/l2/prover/src/registry.rs` | 2.2 | **신규** | ✅ |
| `crates/l2/prover/src/lib.rs` | 2.2 | 수정 | ✅ |
| `crates/l2/common/src/prover.rs` | 2.2 | 수정 | ✅ |
| `crates/l2/prover/src/prover.rs` | 2.2, 2.4 | 수정 | ✅ |
| `crates/l2/sequencer/proof_coordinator.rs` | 2.2 | 수정 | ✅ |
| `crates/l2/contracts/src/l1/OnChainProposer.sol` | 2.3 | 수정 | ✅ |
| `crates/l2/contracts/src/l1/interfaces/IOnChainProposer.sol` | 2.3 | 수정 | ✅ |
| `crates/l2/contracts/src/l1/based/OnChainProposer.sol` | 2.3 | 수정 | ✅ |
| `crates/l2/contracts/src/l1/based/interfaces/IOnChainProposer.sol` | 2.3 | 수정 | ✅ |
| `crates/l2/sequencer/l1_committer.rs` | 2.3 | 수정 | ✅ |
| `crates/l2/contracts/src/l1/GuestProgramRegistry.sol` | 3 | **신규** | ✅ |
| `crates/l2/contracts/src/l1/interfaces/IGuestProgramRegistry.sol` | 3 | **신규** | ✅ |
| `crates/l2/contracts/src/l1/OnChainProposer.sol` | 3 | 수정 | ✅ |
| `crates/l2/contracts/src/l1/based/OnChainProposer.sol` | 3 | 수정 | ✅ |
| `cmd/ethrex/build_l2.rs` | 3 | 수정 | ✅ |
| `cmd/ethrex/l2/deployer.rs` | 3 | 수정 | ✅ |
| `platform/server/*` (18 파일) | 3 | **신규** | ✅ |
| `platform/client/*` (24 파일) | 3 | **신규** | ✅ |
| `crates/guest-program/src/lib.rs` | S1 | 수정 | ✅ |
| `crates/guest-program/src/programs/evm_l2.rs` | S1 | 수정 | ✅ |
| `crates/l2/prover/src/backend/openvm.rs` | S1 | 수정 | ✅ |
| `crates/l2/prover/Cargo.toml` | S2 | 수정 | ✅ |
| `crates/l2/prover/src/backend/sp1.rs` | S2 | 수정 | ✅ |
| `crates/l2/prover/src/backend/exec.rs` | S3 | 수정 | ✅ |
| `crates/l2/storage/src/api.rs` | S4 | 수정 | ✅ |
| `crates/l2/storage/src/store.rs` | S4 | 수정 | ✅ |
| `crates/l2/storage/src/store_db/in_memory.rs` | S4 | 수정 | ✅ |
| `crates/l2/storage/src/store_db/sql.rs` | S4 | 수정 | ✅ |
| `crates/l2/sequencer/proof_coordinator.rs` | S4 | 수정 | ✅ |
| `crates/l2/sequencer/l1_committer.rs` | S4 | 수정 | ✅ |

**미변경 파일** (원래 계획에서 수정 예정이었으나 불필요):
| 파일 | 사유 |
|------|------|
| `crates/l2/sequencer/l1_proof_sender.rs` | `verifyBatch()` 시그니처 불변 |
| `crates/guest-program/build.rs` | 멀티 ELF 빌드 미구현 |

**미생성 파일** (원래 계획에서 생성 예정이었으나 스코프 변경):
| 파일 | 사유 |
|------|------|
| `crates/guest-program/programs/transfer/` | ZK-DEX/Tokamon 템플릿으로 대체 |
| `crates/guest-program/src/programs/transfer.rs` | 상동 |
| `scripts/new-guest-program.sh` | Phase 2.4 미완료 |
| `docs/l2/guest-program-development.md` | Phase 2.4 미완료 |

---

## 의존성 관계 (실제 진행)

```
Phase 2.1 ✅ ──▶ Phase 2.2 ✅ ──▶ Phase 2.3 ✅ ──▶ Phase 2.4 ✅
(코어 추상화)     (레지스트리)      (L1 컨트랙트)     (앱 템플릿)
                                                      │
                                                      ▼
                                                   Phase 3 ✅
                                                (멀티역할 플랫폼)
```

- Phase 2.1 → 2.2: 트레이트 정의 후 레지스트리 구현
- Phase 2.2 → 2.3: 프로토콜 확장 후 L1 변경
- Phase 2.3 → 2.4: L1 `programTypeId` 지원 후 앱 특화 템플릿 등록
- Phase 2.4 → Phase 3: 모듈화 완료 후 멀티역할 플랫폼 아키텍처 (별도 문서 참조)

---

## 테스트 현황

| 크레이트 | 테스트 수 | 상태 |
|---------|----------|------|
| `ethrex-guest-program` | 15 | ✅ 전체 통과 |
| `ethrex-prover` (registry + exec 백엔드) | 10 | ✅ 전체 통과 |
| **합계** | **25** | **✅** |

> 안정화 태스크로 추가된 테스트:
> - `ethrex-guest-program`: `openvm_backend_elf_lookup` (+1)
> - `ethrex-prover`: `backend_name_is_exec`, `execute_with_elf_invalid_input_returns_serialization_error`, `prove_with_elf_invalid_input_returns_serialization_error`, `execute_with_elf_empty_input_returns_error`, `prove_with_elf_empty_input_returns_error` (+5)

```bash
# 검증 명령
cargo test -p ethrex-guest-program -p ethrex-prover
cargo check -p ethrex-l2
cargo check -p ethrex-storage-rollup --features sql
```

---

## Phase 3: 멀티역할 플랫폼 (Guest Program Store) ✅ 완료

### 목표
- GPTs Store 모델의 Guest Program 플랫폼 구축
- 사용자가 자신의 컨트랙트 + 서킷을 만들어 공유
- 서버 (Express.js) + 클라이언트 (Next.js) 아키텍처
- Google, Naver, Kakao OAuth 소셜 로그인
- `GuestProgramRegistry.sol` L1 온체인 컨트랙트

### 실제 구현 내용

#### 3.1 Express 서버 ✅

**디렉토리**: `platform/server/`

| 파일 | 설명 |
|------|------|
| `server.js` | Express 메인 서버 (CORS, 라우트, 헬스체크, 레이트리미팅) |
| `package.json` | 의존성: express, bcryptjs, better-sqlite3, cors, google-auth-library, multer, uuid |
| `lib/google-auth.js` | Google OAuth ID Token 검증 |
| `lib/naver-auth.js` | Naver OAuth 코드 교환 |
| `lib/kakao-auth.js` | Kakao OAuth 코드 교환 |
| `lib/validate.js` | 입력 검증 헬퍼: isValidEmail, isValidPassword, isValidProgramId, isValidCategory |
| `middleware/auth.js` | 세션 관리 (인메모리 Map, 24h TTL, `ps_` 접두사 토큰), `requireAuth`, `requireAdmin` 미들웨어 |
| `db/schema.sql` | SQLite 스키마: users, programs, program_usage, deployments 테이블 |
| `db/db.js` | SQLite 연결 (better-sqlite3, WAL 모드, 자동 마이그레이션) |
| `db/users.js` | CRUD: createUser, getUserById, getUserByEmail, findOrCreateOAuthUser, updateUser |
| `db/programs.js` | CRUD: createProgram, getActivePrograms, approveProgram, rejectProgram; programTypeId 10부터 자동 할당 |
| `db/deployments.js` | CRUD: createDeployment, getDeploymentsByUser, updateDeployment, deleteDeployment |
| `routes/auth.js` | 인증 라우트: signup, login, google, naver, kakao, me, profile update, logout |
| `routes/store.js` | 공개 스토어 API: programs 목록 (검색/카테고리), 프로그램 상세, 카테고리, featured |
| `routes/programs.js` | 크리에이터 API (인증 필수): create, list mine, get, update, delete/deactivate, ELF/VK 업로드 |
| `routes/deployments.js` | 디플로이먼트 API (인증 필수): create, list mine, get, update, delete |
| `routes/admin.js` | 관리자 API (admin 역할 필수): all programs, program detail, approve, reject, stats, users |
| `.env.example` | 환경변수 템플릿 |

#### 3.2 Next.js 클라이언트 ✅

**디렉토리**: `platform/client/`

| 파일 | 설명 |
|------|------|
| `package.json` | Next.js 15, React 19, @react-oauth/google, Tailwind CSS 4 |
| `tsconfig.json`, `next.config.ts`, `postcss.config.mjs` | 프로젝트 설정 |
| `lib/api.ts` | API 클라이언트: authApi, storeApi, programsApi, deploymentsApi, adminApi 래퍼; Bearer 토큰 자동 첨부; apiUpload 멀티파트 헬퍼 |
| `lib/types.ts` | TypeScript 인터페이스: User, Program, Deployment |
| `components/auth-provider.tsx` | React AuthContext, useAuth 훅, 세션 자동 확인 |
| `components/providers.tsx` | GoogleOAuthProvider + AuthProvider + Nav 래퍼 |
| `components/social-login-buttons.tsx` | NaverLoginButton, KakaoLoginButton, GoogleLoginButton (@react-oauth/google 연동) |
| `components/nav.tsx` | 네비게이션 바 (Store/My Programs/Deployments/Admin 링크, 인증 상태 기반) |

**페이지** (16개 라우트):

| 라우트 | 설명 |
|--------|------|
| `/` | 홈 (히어로 섹션, Featured Programs, 아키텍처 개요) |
| `/login` | 로그인 (소셜 버튼 + 이메일/패스워드) |
| `/signup` | 회원가입 (소셜 버튼 + 이메일/패스워드) |
| `/auth/callback/naver` | Naver OAuth 콜백 (Suspense 래핑) |
| `/auth/callback/kakao` | Kakao OAuth 콜백 (Suspense 래핑) |
| `/store` | 스토어 목록 (검색, 카테고리 필터, 프로그램 카드) |
| `/store/[id]` | 프로그램 상세 (설명, 통계, "Use This Program" 모달) |
| `/creator` | 내 프로그램 목록 (상태별 색상 배지) |
| `/creator/new` | 새 프로그램 생성 (programId, 이름, 카테고리, 설명) |
| `/creator/[id]` | 프로그램 편집 (이름/설명 수정, ELF/VK 업로드, 비활성화) |
| `/deployments` | 내 디플로이먼트 목록 (상태별 색상 배지) |
| `/deployments/[id]` | 디플로이먼트 상세 (설정 편집, L2 TOML 설정 내보내기, 삭제) |
| `/admin` | 관리자 대시보드 (통계 카드, 프로그램 목록, 승인/거부) |
| `/admin/programs/[id]` | 관리자 프로그램 상세 (ELF/VK 해시, 크리에이터 정보, 승인/거부) |
| `/profile` | 사용자 프로필 (이름 편집, 프로그램/디플로이먼트 통계) |

빌드 검증: `npx next build` 성공, 16개 라우트 전체 생성.

#### 3.3 GuestProgramRegistry.sol L1 컨트랙트 ✅

**파일**: `crates/l2/contracts/src/l1/GuestProgramRegistry.sol` (신규)
**파일**: `crates/l2/contracts/src/l1/interfaces/IGuestProgramRegistry.sol` (신규)

```solidity
contract GuestProgramRegistry is
    IGuestProgramRegistry, Initializable, UUPSUpgradeable, Ownable2StepUpgradeable
{
    uint8 public constant STORE_PROGRAM_START_ID = 10;
    uint8 public nextProgramTypeId;  // starts at 10

    mapping(bytes32 => ProgramInfo) internal _programs;   // keccak256(programId) → info
    mapping(uint8 => bytes32) internal _typeIdToHash;     // typeId → programId hash
    uint8 public programCount;
}
```

**주요 기능**:
- `registerProgram(programId, name, creator)` → 자동 `programTypeId` 할당 (10부터)
- `registerOfficialProgram(programId, name, creator, typeId)` → 공식 템플릿 (typeId 2-9)
- `deactivateProgram(programId)` / `activateProgram(programId)`
- `getProgram(programId)`, `getProgramByTypeId(typeId)`, `isProgramActive(typeId)`
- `initialize(owner)`: 기본 EVM-L2 프로그램 사전 등록 (typeId=1)
- UUPS 프록시 패턴 (Ownable2Step)

**programTypeId 할당 범위**:
- 0: 예약 (OnChainProposer에서 DEFAULT_PROGRAM_TYPE_ID로 매핑)
- 1: EVM-L2 (기본, 사전 등록)
- 2-9: 공식 템플릿 (코어 팀 예약)
- 10-255: 스토어 프로그램 (커뮤니티 등록)

#### 3.4 OnChainProposer 연동 ✅

**파일**: `crates/l2/contracts/src/l1/OnChainProposer.sol` (수정)
**파일**: `crates/l2/contracts/src/l1/based/OnChainProposer.sol` (수정)

- `GUEST_PROGRAM_REGISTRY` 스토리지 변수 추가
- `commitBatch()`에서 레지스트리 검증: `isProgramActive(effectiveProgramTypeId)`
- `setGuestProgramRegistry(address)` setter (onlyOwner)
- zero-address 체크로 하위 호환성 유지

#### 3.5 L1 배포자 통합 ✅

**파일**: `cmd/ethrex/build_l2.rs` (수정)
- GuestProgramRegistry를 Solidity 컴파일 대상에 추가

**파일**: `cmd/ethrex/l2/deployer.rs` (수정)
- `GUEST_PROGRAM_REGISTRY_BYTECODE` 상수 추가
- `GUEST_PROGRAM_REGISTRY_INITIALIZER_SIGNATURE`, `SET_GUEST_PROGRAM_REGISTRY_SIGNATURE` 상수
- `ContractAddresses` 구조체에 `guest_program_registry_address` 필드
- `deploy_contracts()`에서 UUPS 프록시 배포 (`deploy_with_proxy_from_bytecode_no_wait`)
- `initialize_contracts()`에서 레지스트리 초기화 + OnChainProposer 연동
- `cargo check -p ethrex` 컴파일 통과

#### 3.6 디플로이먼트 시스템 ✅

- `deployments` 테이블: 사용자가 프로그램을 선택하여 L2 배포 설정 생성
- CRUD API: create, list, get, update, delete (인증 필수)
- "Use This Program" 모달: 스토어 상세 페이지에서 배포 이름/Chain ID/RPC URL 입력
- L2 TOML 설정 내보내기: 디플로이먼트 상세에서 ethrex L2 노드 설정 파일 미리보기 및 다운로드

#### 3.7 보안 및 검증 ✅

- `lib/validate.js`: 이메일, 비밀번호, 이름, programId, 카테고리, URL 검증 헬퍼
- `auth.js` 라우트: 이메일 형식, 비밀번호 강도(8-128자), 이름 길이 검증
- `programs.js` 라우트: programId 형식(소문자+숫자+하이픈), 카테고리 화이트리스트, PUT 업데이트 필드 화이트리스트
- `server.js`: IP 기반 인메모리 레이트리미팅 (100 req/min)

#### 3.8 ELF/VK 업로드 ✅

- `multer` 기반 파일 업로드 (100MB 제한)
- `POST /api/programs/:id/upload/elf`: SHA-256 해시 계산, 파일 저장
- `POST /api/programs/:id/upload/vk`: SP1/RISC0 VK 파일 업로드
- 크리에이터 페이지에서 업로드 UI, 해시 표시

### Phase 3 완료 기준

- [x] Express 서버 구현 (인증, 스토어 API, 크리에이터 API, 관리자 API, 디플로이먼트 API)
- [x] Next.js 클라이언트 구현 (16개 라우트, OAuth 콜백, 반응형 UI)
- [x] Google/Naver/Kakao OAuth 소셜 로그인
- [x] GuestProgramRegistry.sol L1 컨트랙트 (UUPS, 인터페이스 포함)
- [x] OnChainProposer ↔ GuestProgramRegistry 연동
- [x] L1 배포자에 GuestProgramRegistry 배포 + 초기화 + 연동 코드
- [x] 디플로이먼트 시스템 ("Use This Program" → L2 TOML 설정 내보내기)
- [x] ELF/VK 업로드 (SHA-256 해시, multer)
- [x] 입력 검증 및 레이트리미팅
- [x] 관리자 프로그램 상세 뷰 (ELF/VK 해시, 크리에이터 정보)
- [x] `npx next build` 성공 (16개 라우트)
- [x] `cargo check -p ethrex` 성공
- [x] 19개 Rust 테스트 통과

---

## 안정화 태스크 (Post-Phase 3) ✅ 완료

Phase 2.1-2.4 + Phase 3 구현 완료 후, 4가지 핵심 안정화 작업을 수행했다.

### S1. OpenVM ELF 레지스트리 통합 ✅

OpenVM 백엔드의 로컬 `include_bytes!`를 제거하고 중앙 레지스트리로 통합.

| 파일 | 변경 내용 |
|------|----------|
| `crates/guest-program/src/lib.rs` | `ZKVM_OPENVM_PROGRAM_ELF` 상수 추가 (`#[cfg(feature = "openvm")]`) |
| `crates/guest-program/src/programs/evm_l2.rs` | `elf()` 메서드에 `backends::OPENVM` 분기 추가, `openvm_backend_elf_lookup` 테스트 |
| `crates/l2/prover/src/backend/openvm.rs` | 로컬 `static PROGRAM_ELF` 제거 → `ZKVM_OPENVM_PROGRAM_ELF` 임포트 |

### S2. SP1 Setup 캐싱 (ELF 해시 기반) ✅

`prove_with_elf()` 호출 시 매번 `client.setup(elf)` 재실행하던 성능 병목 해결.

| 파일 | 변경 내용 |
|------|----------|
| `crates/l2/prover/Cargo.toml` | `sha2` 의존성 추가 |
| `crates/l2/prover/src/backend/sp1.rs` | `ELF_KEY_CACHE` 정적 변수, `get_or_setup_keys()` 헬퍼, `prove_with_elf()` 캐시 사용 |

설계 결정:
- `Mutex<HashMap>` 사용 (cache miss는 warm-up 이후 거의 없음, `RwLock` 불필요)
- 캐시 eviction 없음 (운영에서 ELF 수 1-3개)
- `execute_with_elf()`는 pk/vk 불필요하므로 캐싱 불필요

### S3. prove_with_elf 통합 테스트 ✅

Exec 백엔드에 ELF 경로 에러 처리 테스트 5개 추가.

| 파일 | 변경 내용 |
|------|----------|
| `crates/l2/prover/src/backend/exec.rs` | 테스트 모듈: `backend_name_is_exec`, invalid/empty input 에러 테스트 |

### S4. 동적 programTypeId ✅

하드코딩된 `program_type_id = 1`을 storage 기반 동적 조회로 교체.

| 파일 | 변경 내용 |
|------|----------|
| `crates/l2/storage/src/api.rs` | `store_program_id_by_batch`, `get_program_id_by_batch` 트레이트 메서드 |
| `crates/l2/storage/src/store_db/in_memory.rs` | `program_id_by_batch` 필드 및 구현 |
| `crates/l2/storage/src/store_db/sql.rs` | `batch_program_id` 테이블, `read_from_row_text` 헬퍼, UPSERT/SELECT |
| `crates/l2/storage/src/store.rs` | 위임 메서드 2개 |
| `crates/l2/sequencer/proof_coordinator.rs` | proof 저장 시 `program_id` 함께 저장 |
| `crates/l2/sequencer/l1_committer.rs` | `resolve_program_type_id()` 함수, storage 조회 후 동적 변환 |

데이터 흐름:
```
proof_coordinator → handle_submit()
  → store_proof_by_batch_and_type()
  → store_program_id_by_batch()       ← 신규

l1_committer → send_commitment()
  → get_program_id_by_batch()          ← 신규
  → resolve_program_type_id()          ← 신규: "evm-l2"→1, "zk-dex"→2, "tokamon"→3
```

---

## Phase 3 이후 남은 작업

| 항목 | 설명 | 우선순위 |
|------|------|---------|
| 멀티 ELF 빌드 도구 | `GUEST_PROGRAMS` 환경변수로 빌드 대상 선택 | 높음 |
| 실제 ELF 구현 | ZK-DEX, Tokamon의 zkVM 엔트리포인트 + 입력/출력 타입 정의 | 높음 |
| Guest Program SDK | `cargo generate` 템플릿, CLI 도구 | 중간 |
| E2E 테스트 | 서버/클라이언트 통합 테스트, SP1/RISC0 prove_with_elf zkVM 테스트 | 중간 |
| 프로덕션 세션 스토리지 | Redis 또는 DB 기반 세션 (현재 인메모리) | 중간 |
| `serialize_raw()` 표준화 | ELF 경로와 레거시 경로 직렬화 로직 통합 | 중간 |
| 개발자 문서 | `docs/l2/guest-program-development.md` | 낮음 |
| 동적 ELF 로딩 | 파일시스템/원격에서 ELF 로드 (재컴파일 없이 교체) | 낮음 |
| ELF 아키텍처 검증 | `validate_elf()` ELF 헤더로 riscv32/64 확인 | 낮음 |
| Fuzzing 테스트 | `serialize_input`/`encode_output` 안정성 검증 | 낮음 |

---

## 전체 의존성 관계 (최종)

```
Phase 2.1 ✅ ──▶ Phase 2.2 ✅ ──▶ Phase 2.3 ✅ ──▶ Phase 2.4 ✅
(코어 추상화)     (레지스트리)      (L1 컨트랙트)     (앱 템플릿)
                                                      │
                                                      ▼
                                                   Phase 3 ✅
                                                (멀티역할 플랫폼)
                                                      │
                                                      ▼
                                                  안정화 S1-S4 ✅
                                          (캐싱, 테스트, ELF 통합, 동적 typeId)
                                                      │
                                                      ▼
                                                   남은 작업 ⏳
                                             (ELF 구현, SDK, 빌드 도구)
```
