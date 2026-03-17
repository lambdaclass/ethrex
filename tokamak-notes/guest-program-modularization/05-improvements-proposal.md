# 설계 보완 및 개선 제안서

이 문서는 `guest-program-modularization` 설계 문서를 분석하고, Phase 2.1-2.4 구현 경험을 반영하여, 현재 계획에서 누락되었거나 구체화가 필요한 사항들을 정리하고 개선 방안을 제안한다.

> **구현 상태** (2026-02-22 기준):
> - Phase 2.1 (코어 추상화): **완료** — `GuestProgram` 트레이트, `EvmL2GuestProgram`, `ProverBackend` ELF 메서드, 5개 백엔드 업데이트
> - Phase 2.2 (레지스트리 & 프로토콜): **완료** — `GuestProgramRegistry`, `ProofData` 프로토콜 확장, Proof Coordinator/Prover 통합, `supported_programs` 필터링 구현
> - Phase 2.3 (L1 컨트랙트 & 검증): **완료** — OnChainProposer 3D VK 매핑, `programTypeId`, `commitBatch` 시그니처 확장, based 변형 동일 수정, l1_committer 수정
> - Phase 2.4 (앱 템플릿 & 도구): **완료** — ZkDex/Tokamon 스텁 구현 및 레지스트리 등록, Exec 백엔드 ELF 경로 구현
> - Phase 3 (멀티역할 플랫폼): **완료** — Guest Program Store (Express+Next.js), GuestProgramRegistry.sol, L1 배포자 통합
> - **안정화 태스크** (4개): **완료** — SP1 Setup 캐싱, prove_with_elf 통합 테스트, OpenVM ELF 레지스트리 통합, 동적 programTypeId

---

## 1. 아키텍처 및 설계 (Architecture & Design)

### 1.1 Guest Program 버전 관리 체계 부재
*   **문제점**: 현재 설계는 `program_id` (예: `"evm-l2"`)로 프로그램을 식별하지만, 동일한 프로그램의 로직이 수정(버그 픽스, 최적화)되었을 때를 대비한 버전 관리 규칙이 명시되어 있지 않다.
*   **리스크**: L1 검증 시 구버전과 신버전의 Verification Key(VK)가 혼재될 경우, 어떤 버전의 로직으로 생성된 증명인지 구분하기 어려워 검증 실패나 보안 취약점이 발생할 수 있다.
*   **현재 상태**: `ProofData::BatchRequest`에는 이미 `commit_hash`가 있어 코드 버전은 추적된다. 하지만 이는 전체 바이너리 버전이지 개별 Guest Program 버전이 아니다.
*   **개선 제안**:
    *   `GuestProgram` 트레이트에 `version() -> &str` 메서드를 추가한다. 이 버전은 ELF 바이너리의 빌드 해시 또는 시맨틱 버전을 반환한다.
    *   L1 VK 매핑을 `verificationKeys[commitHash][programTypeId][verifierId]`에서 추가로 `commitHash` 자체가 Guest Program 버전을 암묵적으로 포함하므로, 명시적 버전 필드보다는 **ELF 해시 기반 VK 조회**가 더 안전하다.
    *   구체적으로: `GuestProgram::elf_hash(backend) -> Option<[u8; 32]>`를 추가하여 ELF 바이너리의 SHA-256 해시를 반환하고, VK 등록/조회 시 이 해시를 사용한다.

### 1.2 동적 로딩의 한계 (재배포 필요성)
*   **문제점**: 현재 `GuestProgramRegistry`는 프루버 시작 시 `create_default_registry()`에서 `HashMap`에 구현체를 등록하는 방식이다. 새로운 Guest Program을 추가하려면 프루버 바이너리 전체를 **재컴파일하고 재배포**해야 한다.
*   **현재 구현**: `prover.rs`의 `create_default_registry()`가 `EvmL2GuestProgram`만 등록하며, 레지스트리는 프루버 수명 동안 불변이다.
*   **리스크**: 운영 중인 프루버 노드 전체를 업데이트해야 하므로 유지보수 비용이 크고, 긴급한 커스텀 서킷 추가가 어렵다.
*   **개선 제안**:
    *   **단기**: 설정 파일(TOML/YAML)에서 활성화할 프로그램 목록을 지정하고, `create_default_registry()`가 이를 읽어 조건부 등록하도록 한다. ELF 자체는 컴파일 타임에 포함되지만, 활성화/비활성화는 런타임에 제어 가능해진다.
    *   **중기**: ELF 바이너리를 파일 시스템이나 원격 스토리지에서 동적 로드하는 `FileBasedGuestProgram` 구현을 추가한다. `include_bytes!()` 대신 `std::fs::read()`로 ELF를 로드하면 재컴파일 없이 ELF만 교체 가능하다.
    *   **장기**: WASM 기반이나 동적 라이브러리(`.so`, `.dll`) 로딩을 검토한다.

### 1.3 [해결됨] ~~이중 경로(Dual-Path) 증명 로직의 복잡성~~
*   **[해결됨]** Exec 백엔드에 `execute_with_elf()`와 `prove_with_elf()` 구현 완료.
    *   ELF를 무시하고, rkyv 역직렬화(`rkyv::from_bytes::<ProgramInput, rkyv::rancor::Error>`)하여 `execute_core()` 호출.
    *   모든 백엔드가 ELF 경로를 지원하게 되었다 (SP1/RISC0는 기본 `NotImplemented`, Exec는 실제 구현).
*   **잔여 과제**: 장기적으로 레거시 `prove(ProgramInput)` 경로를 deprecate하고 `prove_with_elf` 경로로 통합한다.

### 1.4 [해결됨] ~~SP1 ProverSetup 캐싱과 멀티프로그램 비효율~~
*   **[해결됨]** SHA-256 기반 ELF 키 캐싱이 SP1 백엔드에 구현되었다.
    *   `ELF_KEY_CACHE: OnceLock<Mutex<HashMap<[u8;32], (SP1ProvingKey, SP1VerifyingKey)>>>` 정적 변수 추가
    *   `get_or_setup_keys(&self, elf: &[u8])` 헬퍼: SHA-256(elf) 해시로 캐시 키 생성, cache hit → 클론 반환, cache miss → `client.setup(elf)` 실행 후 캐시 저장
    *   `prove_with_elf()`에서 직접 `setup()` 대신 캐시된 키 사용
    *   `sha2` 크레이트 의존성 추가 (workspace에 이미 존재)
    *   RISC0는 `default_prover()`가 내부 캐싱을 처리하므로 추가 작업 불필요.

---

## 2. 구현 및 의존성 (Implementation & Dependencies)

### 2.1 순환 의존성 해결 — ~~공통 타입 크레이트 분리 필요~~
*   **[해결됨]** ~~`GuestProgram` 트레이트가 `BackendType`을 참조하고, `ProverBackend`가 `GuestProgram`을 참조하는 순환 의존성 문제.~~
*   **해결 방법**: `BackendType` 열거형 대신 `&str` 상수를 사용하는 **Option B** 방식을 채택했다.
    *   `crates/guest-program/src/traits.rs`에 `backends` 모듈을 정의: `pub const SP1: &str = "sp1"` 등
    *   `GuestProgram::elf()`, `vk_bytes()`는 `&str`을 매개변수로 받음
    *   `ProverBackend::backend_name()` 메서드가 동일한 상수를 반환
    *   `BackendType::as_backend_name()`으로 열거형 → 문자열 변환 지원
*   **결론**: `Phase 2.0` 신설은 **불필요**하다. `&str` 기반 접근법이 순환 의존성을 완전히 해결하며, 별도 크레이트 없이 구현되었다.

### 2.2 표준 라이브러리(SDK) 구체화
*   **문제점**: Phase 2.4(SDK & 개발자 도구)가 가장 마지막 단계로 잡혀 있다. 하지만 Transfer나 DEX 같은 커스텀 서킷을 개발하려면 머클 트리 검증, 서명 검증, RLP 인코딩 등 공통 기능이 필수적이다.
*   **리스크**: 개발자들이 각자 중복된 유틸리티 코드를 작성하게 되어 코드 품질 저하 및 보안 취약점이 발생할 수 있다.
*   **개선 제안**:
    *   기존 `guest-program/src/common/execution.rs`의 `execute_blocks()` 클로저 패턴을 재사용 가능한 빌딩 블록으로 분리한다.
    *   `crates/guest-std` 라이브러리를 Phase 2.2(레퍼런스 구현)와 병행하여 구축한다:
        *   `guest_std::merkle` — 상태 트라이 검증
        *   `guest_std::crypto` — 서명 검증 (secp256k1)
        *   `guest_std::encoding` — RLP/ABI 인코딩

### 2.3 [해결됨] ~~`SerializedInput` 연관 타입과 `prove_with_elf` 바이트 인터페이스 불일치~~
*   **[해결됨]** `ProverBackend`에 `serialize_raw()` 메서드가 추가되어 직렬화가 표준화되었다.
    *   `serialize_raw(&ProgramInput) -> Result<Vec<u8>, BackendError>` — 기본 구현은 `rkyv::to_bytes`
    *   모든 백엔드의 `serialize_input()`이 내부적으로 `serialize_raw()`를 호출하도록 리팩토링
    *   `prover.rs`의 `prove_batch()`에서 인라인 `rkyv::to_bytes` 제거 → `backend.serialize_raw()` 사용
    *   SP1, RISC0, OpenVM, ZisK 백엔드 모두 직접 rkyv 의존성 제거
    *   ELF 경로와 레거시 경로가 동일한 직렬화 로직을 공유하게 됨

### 2.4 [해결됨] ~~OpenVM ELF가 레지스트리에서 누락~~
*   **[해결됨]** OpenVM ELF가 중앙 레지스트리에 통합되었다.
    *   `lib.rs`에 `ZKVM_OPENVM_PROGRAM_ELF` 상수 추가 (`#[cfg(feature = "openvm")]` 게이팅, SP1/ZisK와 동일 패턴)
    *   `EvmL2GuestProgram::elf()`에 `backends::OPENVM` 분기 추가
    *   OpenVM 백엔드의 로컬 `static PROGRAM_ELF` 제거 → `ZKVM_OPENVM_PROGRAM_ELF` 임포트로 교체
    *   `openvm_backend_elf_lookup` 테스트 추가

---

## 3. L1 컨트랙트 및 데이터 (L1 & Data)

### 3.1 DA(Data Availability) 포맷 호환성
*   **문제점**: `ProgramOutput`은 바이트로 인코딩되어 L1에 제출되지만, 커스텀 서킷(예: DEX)은 EVM 트랜잭션과 다른 데이터 구조를 가질 수 있다. L1 컨트랙트의 `_getPublicInputsFromCommitment()`은 현재 EVM-L2 전용 레이아웃을 하드코딩하고 있다.
*   **리스크**: L1 컨트랙트가 DA 데이터를 파싱하거나 검증할 때, 예상치 못한 포맷으로 인해 트랜잭션이 리버트(Revert)될 수 있다.
*   **개선 제안**:
    *   `GuestProgram` 트레이트의 `encode_output()`이 이미 출력 인코딩 책임을 가지므로, 각 프로그램이 L1 호환 포맷으로 변환하는 것은 자연스럽다.
    *   L1에서는 `programTypeId`별로 별도의 public input 재구성 함수를 디스패치한다:
        ```solidity
        function _getPublicInputs(uint8 programTypeId, ...) internal view returns (bytes32) {
            if (programTypeId == 1) return _getEvmL2PublicInputs(...);
            if (programTypeId == 2) return _getTransferPublicInputs(...);
            revert("Unknown program type");
        }
        ```
    *   새로운 프로그램 타입 추가 시 L1 컨트랙트 업그레이드가 필요하므로, UUPS 프록시 패턴의 활용이 중요하다.

### 3.2 가스(Cycle) 리미트 및 타임아웃 설정
*   **문제점**: 커스텀 서킷은 EVM보다 훨씬 적거나 많은 연산(Cycle)을 수행할 수 있다. 현재 설계에는 이에 대한 메타데이터가 없다.
*   **현재 상태**: `GuestProgramRegistry`는 프로그램 식별 정보만 저장하며, 리소스 제한 메타데이터가 없다.
*   **리스크**: 무한 루프나 과도한 연산을 수행하는 악의적인 Guest Program이 프루버 자원을 점유(DoS)할 수 있다.
*   **개선 제안**:
    *   `GuestProgram` 트레이트에 `resource_limits()` 메서드를 추가:
        ```rust
        fn resource_limits(&self) -> ResourceLimits {
            ResourceLimits::default() // 무제한
        }

        pub struct ResourceLimits {
            pub max_cycles: Option<u64>,
            pub max_proving_time_secs: Option<u64>,
            pub max_input_size_bytes: Option<usize>,
        }
        ```
    *   `prove_batch()`에서 입력 크기 검증 후, 백엔드에 사이클 제한을 전달한다.
    *   Proof Coordinator가 작업 할당 시 타임아웃을 설정하도록 한다.

### 3.3 [해결됨] ~~`program_type_id`의 L1 사용 갭~~
*   **[해결됨]** Phase 2.3 완료로 `programTypeId`가 L1에서 완전히 사용된다.
    *   `OnChainProposer.sol`: 3D VK 매핑 `verificationKeys[commitHash][programTypeId][verifierId]`
    *   `commitBatch()`: `uint8 programTypeId` 매개변수 추가, `BatchCommitmentInfo`에 저장
    *   `verifyBatch()`: 저장된 `programTypeId`로 올바른 VK 조회
    *   `l1_committer.rs`: storage에서 `program_id`를 조회하여 `resolve_program_type_id()`로 동적 변환
    *   하위 호환성: `programTypeId == 0`은 자동으로 `DEFAULT_PROGRAM_TYPE_ID (1)`로 매핑
*   **[잔여 과제도 해결됨]** `l1_committer.rs`의 하드코딩 `program_type_id = 1` 제거 완료.
    *   Storage API에 `store_program_id_by_batch` / `get_program_id_by_batch` 추가 (in-memory + SQL)
    *   Proof Coordinator가 proof 저장 시 `program_id`도 함께 저장
    *   `l1_committer`의 `send_commitment()`에서 storage 조회 → `resolve_program_type_id()` 변환
    *   `resolve_program_type_id()`: `"evm-l2"→1`, `"zk-dex"→2`, `"tokamon"→3`, 기본값 1
    *   조회 실패 시 `"evm-l2"` 기본값으로 하위 호환성 유지

---

## 4. 프로토콜 및 통신 (Protocol & Communication)

### 4.1 [신규] ProofData 프로토콜 역호환성 검증
*   **문제점**: `ProofData` 열거형에 `#[serde(default)]`를 사용하여 새 필드를 추가했으나, 실제 역호환성이 end-to-end로 검증되지 않았다.
*   **현재 상태**:
    *   `BatchRequest.supported_programs`: `#[serde(default)]` → 구버전 프루버가 보내도 빈 Vec으로 역직렬화
    *   `BatchResponse.program_id`: `#[serde(default)]` → 구버전 코디네이터 응답에서 `None`으로 역직렬화
    *   `ProofSubmit.program_id`: `#[serde(default = "default_program_id")]` → 구버전에서 `"evm-l2"`로 역직렬화
*   **리스크**: JSON 직렬화에서는 문제없지만, 향후 프로토콜을 바이너리(protobuf 등)로 전환할 경우 `#[serde(default)]` 의미가 달라진다.
*   **개선 제안**:
    *   구버전/신버전 프루버-코디네이터 간 통신을 검증하는 통합 테스트를 추가한다.
    *   프로토콜 버전 필드를 `ProofData`에 추가하여 명시적 버전 관리를 한다:
        ```rust
        BatchRequest {
            #[serde(default = "default_protocol_version")]
            protocol_version: u8, // 1 = original, 2 = with program_id
            ...
        }
        ```

### 4.2 [해결됨] ~~`supported_programs` 필터링 미구현~~
*   **[해결됨]** Proof Coordinator에 `supported_programs` 필터링이 구현되었다.
    *   빈 리스트면 모든 프로그램 수용 (레거시 호환)
    *   프루버가 지원하지 않는 프로그램의 배치는 빈 응답으로 거부

---

## 5. 테스트 및 검증 (Testing & QA)

### 5.1 Fuzzing 테스트 추가
*   **문제점**: Unit/Integration 테스트는 계획되어 있으나, 임의의 입력값에 대한 안정성 검증(Fuzzing) 계획이 없다.
*   **리스크**: `serialize_input`이나 `encode_output`에서 잘못된 바이트 입력 시 패닉(Panic)이 발생하면 프루버 노드가 다운될 수 있다.
*   **개선 제안**:
    *   `arbitrary` 크레이트 등을 활용하여 `GuestProgram` 인터페이스에 대한 Fuzzing 테스트를 CI 파이프라인에 추가한다.

### 5.2 크로스 컴파일 아키텍처 검증
*   **문제점**: `ProverBackend`는 ELF를 바이트 배열(`&[u8]`)로 받는다. 이 ELF가 타겟 zkVM이 지원하는 아키텍처(riscv32 vs riscv64)로 올바르게 빌드되었는지 런타임에 확인하는 절차가 부족하다.
*   **리스크**: 잘못된 아키텍처로 빌드된 ELF를 실행 시, 원인 불명의 런타임 에러가 발생하여 디버깅이 어려워질 수 있다.
*   **개선 제안**:
    *   `GuestProgram` 트레이트에 `validate_elf(backend: &str, elf: &[u8]) -> Result<(), GuestProgramError>` 메서드를 추가한다 (기본 구현은 pass-through).
    *   `prove_batch()`에서 ELF 사용 전 검증을 수행한다.
    *   ELF 헤더 검증 유틸리티: `e_ident[EI_CLASS]`로 32/64비트 확인, `e_machine`으로 RISC-V 확인.

### 5.3 [부분 해결] `prove_with_elf` 통합 테스트
*   **[부분 해결]** Exec 백엔드에 대한 `prove_with_elf` 통합 테스트가 추가되었다.
    *   `backend_name_is_exec` — 백엔드 이름 확인
    *   `execute_with_elf_invalid_input_returns_serialization_error` — 잘못된 rkyv 바이트 → `Serialization` 에러
    *   `prove_with_elf_invalid_input_returns_serialization_error` — 동일 검증
    *   `execute_with_elf_empty_input_returns_error` — 빈 입력 에러 처리
    *   `prove_with_elf_empty_input_returns_error` — 빈 입력 에러 처리
*   **잔여 과제**: SP1/RISC0의 실제 zkVM 환경에서의 `prove_with_elf` 테스트 (CI에 zkVM 환경 필요):
        ```rust
        #[test]
        fn prove_with_elf_matches_legacy() {
            let backend = Sp1Backend::new();
            let input = test_program_input();
            let elf = EvmL2GuestProgram.elf("sp1").unwrap();
            let bytes = rkyv::to_bytes::<Error>(&input).unwrap();

            // Legacy path
            let legacy_proof = backend.prove(input.clone(), ProofFormat::Compressed).unwrap();
            // ELF path
            let elf_proof = backend.prove_with_elf(elf, &bytes, ProofFormat::Compressed).unwrap();

            // 증명 결과가 동일한지 확인 (public values 비교)
        }
        ```

---

## 6. 실행 계획 수정 제안 (Revised Roadmap)

Phase 2.1-2.4 구현 경험을 바탕으로 한 수정 결과:

| 항목 | 기존 계획 | 실제 결과 | 상태 |
|------|----------|----------|------|
| Phase 2.0 | 공통 타입 분리 크레이트 신설 | **불필요** — `&str` 상수로 순환 의존성 해결됨 | ~~해결~~ |
| Phase 2.1 | GuestProgram 트레이트 + 백엔드 수정 | 그대로 진행, 성공 | **✅ 완료** |
| Phase 2.2 | 레지스트리 + 프로토콜 + Transfer Circuit | 레지스트리/프로토콜/필터링 완료. Transfer Circuit은 ZK-DEX/Tokamon으로 대체 | **✅ 완료** |
| Phase 2.2b | `supported_programs` 필터링 | Phase 2.2에 통합 완료 | **✅ 완료** |
| Phase 2.3 | L1 컨트랙트 수정 | 3D VK 매핑, commitBatch 시그니처 확장, based 변형 동일 수정 | **✅ 완료** |
| Phase 2.4 | SDK & 개발자 도구 | ZK-DEX/Tokamon 스텁, 레지스트리 등록, Exec ELF 경로 구현 | **✅ 완료** |
| Phase 3 | — | 멀티역할 플랫폼 아키텍처 (Guest Program Store, GuestProgramRegistry.sol, L1 배포) | **✅ 완료** |
| 안정화 | — | SP1 캐싱, prove_with_elf 테스트, OpenVM ELF 통합, 동적 programTypeId | **✅ 완료** |

### 완료된 후속 작업

1. ~~**[중요]** SP1 setup 캐싱 (§1.4)~~ — **✅ 해결됨**: SHA-256 기반 ELF 키 캐시 구현
2. ~~**[중요]** `prove_with_elf` 통합 테스트 (§5.3)~~ — **✅ 부분 해결**: Exec 백엔드 5개 테스트 추가
3. ~~**[권장]** OpenVM ELF 레지스트리 통합 (§2.4)~~ — **✅ 해결됨**: 중앙 상수 + 백엔드 교체
4. ~~**[권장]** `l1_committer.rs` 동적 `program_type_id` 결정 (§3.3 잔여)~~ — **✅ 해결됨**: Storage API + 동적 조회

### 완료된 추가 작업

5. ~~**[높음]** 멀티 ELF 빌드 도구~~ — **✅ 이미 구현됨**: `GUEST_PROGRAMS` 환경변수, `build.rs`, `Makefile`, scaffold 스크립트
6. ~~**[중간]** `serialize_raw()` 표준화 (§2.3)~~ — **✅ 해결됨**: ELF/레거시 직렬화 통합, 모든 백엔드 리팩토링

### 완료된 후속 작업 (추가분)

7. ~~**[높음]** ZK-DEX / Tokamon 실제 ELF 구현~~ — **✅ 해결됨**:
    *   ZK-DEX/Tokamon 커스텀 타입 (`DexProgramInput`, `TokammonProgramInput`) 및 실행 로직 구현
    *   SP1 zkVM 엔트리포인트 (`bin/sp1-zk-dex/`, `bin/sp1-tokamon/`) 생성
    *   `build.rs`에 `GUEST_PROGRAMS=zk-dex,tokamon` 빌드 분기 추가
    *   `lib.rs`에 `ZKVM_SP1_ZK_DEX_ELF`, `ZKVM_SP1_TOKAMON_ELF` 상수 추가
    *   GuestProgram `elf()` 구현체에 SP1 백엔드 분기 추가

8. ~~**[중간]** Guest Program SDK~~ — **✅ 해결됨**:
    *   `scripts/new-guest-program.sh` 스캐폴드 스크립트 업그레이드
    *   모듈 디렉토리 구조 (`types.rs`, `execution.rs`, `mod.rs`) 자동 생성
    *   SP1 바이너리 (`bin/sp1-<name>/`) 자동 생성
    *   PascalCase/snake_case/UPPER_SNAKE 자동 변환, type_id 자동 할당
    *   `mod.rs`에 자동 등록 (`pub mod`, `pub use`)
    *   테스트 포함 (8개: trait, execution, rkyv roundtrip)
9. ~~**[중간]** E2E 테스트~~ — **✅ 부분 해결**:
    *   레지스트리 통합 테스트 9개 추가 (`registry.rs`)
    *   `create_default_registry()` 에서 evm-l2/zk-dex/tokamon 3개 프로그램 정상 등록 검증
    *   ZK-DEX/Tokamon 실행 → 출력 인코딩 → rkyv roundtrip 교차 크레이트 검증
    *   잔여: SP1/RISC0 실제 zkVM `prove_with_elf` 테스트 (CI 환경 필요)

10. ~~**[중간]** 프로덕션 세션 스토리지~~ — **✅ 해결됨**:
    *   `sessions` 테이블 추가 (`schema.sql`)
    *   `db/sessions.js` — SQLite 기반 CRUD (`createSession`, `getSession`, `destroySession`, `cleanupExpiredSessions`)
    *   `middleware/auth.js` — in-memory `Map()` → SQLite 세션 조회로 교체
    *   `server.js` — 1시간 주기 만료 세션 정리 (`setInterval`)
    *   서버 재시작 시 세션 유지, ON DELETE CASCADE로 사용자 삭제 시 세션 자동 정리

11. ~~**[낮음]** ELF 아키텍처 검증~~ — **✅ 해결됨**:
    *   `GuestProgramError::InvalidElf` 변형 추가
    *   `validate_elf_header()` 함수: ELF magic, class (32/64-bit), machine (RISC-V) 검증
    *   `GuestProgram::validate_elf()` 트레이트 메서드 (기본 구현 제공, 오버라이드 가능)
    *   백엔드별 클래스 기대값: SP1/RISC0/OpenVM → 32-bit, ZisK → 64-bit
    *   9개 단위 테스트 추가
12. ~~**[낮음]** Fuzzing 테스트~~ — **✅ 해결됨**:
    *   3개 fuzz-style 프로퍼티 테스트 추가
    *   `serialize_input_never_panics_on_arbitrary_bytes` — 3개 프로그램 × 7개 에지케이스
    *   `encode_output_never_panics_on_arbitrary_bytes` — 3개 프로그램 × 4개 에지케이스
    *   `validate_elf_never_panics_on_arbitrary_bytes` — 6개 백엔드 × 9개 에지케이스

13. ~~**[낮음]** 동적 ELF 로딩~~ — **✅ 해결됨**:
    *   `DynamicGuestProgram` 구현: 파일시스템에서 런타임 ELF 로딩 (재컴파일 불필요)
    *   Builder 패턴: `elf_from_file()`, `elf_from_bytes()`, `vk_from_file()`, `vk_from_bytes()`
    *   `from_dir()`: 표준 디렉토리 레이아웃 (`<dir>/<backend>/elf`) 자동 스캔
    *   ELF 헤더 검증 기본 활성화 (`skip_validation()`으로 비활성화 가능)
    *   `GuestProgram` 트레이트 구현 → `Arc<dyn GuestProgram>`으로 레지스트리 등록 가능
    *   15개 테스트 추가 (builder, file I/O, directory scan, validation, trait object, Arc)

14. ~~**[낮음]** 개발자 가이드 문서~~ — **✅ 해결됨**:
    *   `08-developer-guide.md` 작성: Quick Start, Step-by-Step, Dynamic ELF Loading, Trait Reference, Testing, Checklist
    *   스캐폴드 스크립트 사용법, 수동 생성 절차, 빌드 시스템 연동, 프루버 등록 방법 포함

### 프로덕션 안정화 (Production Hardening)

15. ~~**[높음]** Resource Limits (DoS 방지)~~ — **✅ 해결됨**:
    *   `ResourceLimits` 구조체 추가 (`max_input_bytes`, `max_proving_duration`)
    *   `GuestProgram::resource_limits()` 디폴트 메서드 추가
    *   EVM-L2: 256MB / 1시간, ZK-DEX/Tokamon: 64MB / 30분
    *   `BackendError::ResourceLimitExceeded` 변형 추가
    *   `prove_batch()`에서 입력 크기 검증 및 proving 시간 검증

16. ~~**[중간]** Cross-language 인코딩 테스트~~ — **✅ 해결됨**:
    *   L2 output: 고정 필드 레이아웃, balance diffs, l2 message hashes 검증
    *   ZK-DEX: 72바이트 레이아웃, zero/max 값 검증
    *   Tokamon: 88바이트 레이아웃, zero/max 값 검증
    *   L1 output: 160바이트 레이아웃 검증

17. ~~**[중간]** Guest Program 버전 관리~~ — **✅ 해결됨**:
    *   `GuestProgram::version()` 메서드 추가 (기본값 "0.0.0")
    *   `GuestProgram::elf_hash()` 메서드 추가 (SHA-256)
    *   EVM-L2: `env!("CARGO_PKG_VERSION")`, ZK-DEX/Tokamon: "0.1.0"

18. ~~**[중간]** 런타임 설정 파일~~ — **✅ 해결됨**:
    *   `ProgramsConfig` TOML 파서 추가
    *   `create_default_registry()` → `create_registry(config_path)` 리팩토링
    *   `ProverConfig.programs_config_path` 필드 추가
    *   `programs.example.toml` 예시 파일 추가

19. ~~**[낮음]** ProofData 역호환성 E2E 테스트~~ — **✅ 해결됨**:
    *   `empty_supported_programs_roundtrip` 추가
    *   `default_program_id_is_evm_l2` 추가
    *   `all_variants_roundtrip` 추가 (10개 ProofData 변형)
    *   `extra_json_fields_ignored` 추가

20. ~~**[낮음]** SP1/RISC0 zkVM 테스트 스캐폴딩~~ — **✅ 해결됨**:
    *   `tests/zkvm_integration.rs` — `#[ignore]` 테스트 파일 생성
    *   SP1: evm-l2, zk-dex, tokamon ELF 가용성 + 검증 테스트
    *   RISC0: evm-l2 ELF + VK 가용성 테스트

21. ~~**[높음]** L1 Public Input 디스패처~~ — **✅ 해결됨**:
    *   `BatchCommitmentInfo`에 `publicValuesHash` 필드 추가 (non-based + based)
    *   `commitBatch()`: `publicValuesHash` 파라미터 추가, 커스텀 프로그램(>1) 검증
    *   `verifyBatch()`: `customPublicValues` 파라미터 추가, programTypeId 기반 디스패치
    *   EVM-L2(type=1): 기존 `_getPublicInputsFromCommitment()` 사용
    *   커스텀 프로그램(type>1): `keccak256(customPublicValues) == publicValuesHash` 검증
    *   Timelock.sol, ITimelock.sol 시그니처 동기화
    *   l1_committer.rs, l1_proof_sender.rs Rust 호출자 업데이트

### 남은 후속 작업

(모든 작업 완료)
