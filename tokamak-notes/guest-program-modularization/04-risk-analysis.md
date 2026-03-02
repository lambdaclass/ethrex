# 리스크 분석 및 완화 전략

이 문서는 Guest Program 모듈화 과정에서 예상되는 리스크를 분석하고 구체적인 완화 전략을 제시한다.

## R1. 하위 호환성 (Backward Compatibility)

### 리스크

모듈화 과정에서 기존 EVM-L2 Guest Program의 동작이 변경되면:
- 이미 커밋된 배치의 검증이 실패할 수 있다.
- 프루버 클라이언트와 코디네이터 간 프로토콜 불일치가 발생한다.
- L1 컨트랙트에 저장된 기존 VK가 무효화될 수 있다.

### 영향도: **높음**

기존 시스템이 중단되면 배치 검증이 멈추고 L2 진행이 불가능하다.

### 완화 전략

| # | 전략 | 설명 |
|---|------|------|
| M1.1 | Zero Behavior Change 원칙 | Phase 2.1에서 기존 메서드를 제거하지 않고 새 메서드를 추가. 기존 코드 경로는 변경 없이 유지. |
| M1.2 | 포괄적 테스트 게이트 | 각 Phase 완료 시 기존 모든 테스트(unit + integration)가 통과해야 다음 Phase로 진행. CI에 regression 테스트 추가. |
| M1.3 | serde 하위 호환 | `ProofData`에 `#[serde(default)]` 사용. 이전 버전 프루버가 `program_id` 없이 통신해도 `"evm-l2"`로 기본 처리. |
| M1.4 | Feature flag 보호 | 새 기능을 `modular-guest` feature flag 뒤에 배치. 문제 발생 시 flag off로 기존 동작 복원. |
| M1.5 | 점진적 롤아웃 | Phase 2.1 완료 후 테스트넷에서 충분히 검증한 뒤 Phase 2.2 진행. |

### 구체적 검증 항목

- [ ] `EvmL2GuestProgram.elf(SP1)` == `ZKVM_SP1_PROGRAM_ELF` (바이트 동일성)
- [ ] 레지스트리를 통한 증명 결과가 기존 직접 호출과 동일한 `BatchProof` 생성
- [ ] `ProofData` JSON 직렬화가 이전 포맷과 호환 (새 필드 없는 JSON도 역직렬화 가능)
- [ ] `programTypeId=1` 배치가 기존 `_getPublicInputsFromCommitment()` 결과와 동일한 bytes 출력

---

## R2. L1 컨트랙트 업그레이드

### 리스크

`OnChainProposer.sol`의 VK 매핑을 2차원에서 3차원으로 변경하면:
- **스토리지 레이아웃 변경**: `mapping(bytes32 => mapping(uint8 => bytes32))` → `mapping(bytes32 => mapping(uint8 => mapping(uint8 => bytes32)))`. Solidity의 매핑 스토리지 슬롯 계산이 달라진다.
- 기존 VK 데이터가 새 슬롯에 자동으로 존재하지 않는다.
- 업그레이드 트랜잭션 실패 시 컨트랙트가 불일치 상태에 빠질 수 있다.

### 영향도: **높음**

L1 컨트랙트는 한번 배포되면 취소가 어렵다. 잘못된 업그레이드는 전체 L2를 멈출 수 있다.

### 완화 전략

| # | 전략 | 설명 |
|---|------|------|
| M2.1 | UUPS Proxy 활용 | 이미 `UUPSUpgradeable`을 사용하므로 구현 교체 가능. Timelock을 통한 안전한 업그레이드. |
| M2.2 | 마이그레이션 함수 | 새 구현에 `migrateVerificationKeys()` 함수 포함. 기존 VK를 `programTypeId=1` 슬롯으로 복사. |
| M2.3 | 두 단계 업그레이드 | (1) 새 매핑 구조 배포 + 마이그레이션 실행, (2) 마이그레이션 확인 후 기존 매핑 접근자 제거. |
| M2.4 | 포크 테스트 | Anvil/Hardhat fork 환경에서 메인넷 상태를 복제하여 업그레이드 시뮬레이션. VK 조회가 정상인지 확인. |
| M2.5 | Pause 메커니즘 활용 | 업그레이드 직전 `pause()` → 업그레이드 → 검증 → `unpause()`. 이미 `PausableUpgradeable`이 구현되어 있다. |

### 스토리지 레이아웃 분석

현재 `verificationKeys`의 스토리지 슬롯:
```
slot = keccak256(verifierId . keccak256(commitHash . SLOT_verificationKeys))
```

변경 후:
```
slot = keccak256(verifierId . keccak256(programTypeId . keccak256(commitHash . SLOT_verificationKeys)))
```

**핵심**: 매핑 변수의 선언 순서가 바뀌면 `SLOT_verificationKeys`도 변경될 수 있다. 기존 변수 순서를 유지하고 새 변수는 뒤에 추가해야 한다.

**대안**: 기존 `verificationKeys`를 deprecated로 유지하고 새 변수 `verificationKeysV2`를 추가:
```solidity
// 기존 (deprecated, 읽기 전용으로 유지)
mapping(bytes32 => mapping(uint8 => bytes32)) public verificationKeys;

// 신규
mapping(bytes32 => mapping(uint8 => mapping(uint8 => bytes32))) public verificationKeysV2;
```

이 방식이 스토리지 충돌을 완전히 방지한다.

---

## R3. 빌드 시간 증가

### 리스크

멀티 ELF 컴파일로 빌드 시간이 크게 증가한다:
- 현재 SP1 ELF 빌드만 해도 수 분 소요 (RISC-V 크로스 컴파일 + 최적화).
- Guest Program이 N개이면 빌드 시간이 최대 N배 증가.
- CI/CD 파이프라인 시간 증가로 개발 속도 저하.

### 영향도: **중간**

빌드 시간이 개발자 생산성에 직접 영향을 미친다.

### 완화 전략

| # | 전략 | 설명 |
|---|------|------|
| M3.1 | 선택적 빌드 | `GUEST_PROGRAMS=evm-l2` 환경변수로 필요한 프로그램만 빌드. 기본값은 `evm-l2`만. |
| M3.2 | 병렬 빌드 | 각 Guest Program의 ELF 빌드를 병렬로 실행 (빌드 스크립트에서 `std::thread::spawn`). |
| M3.3 | ELF 캐싱 | CI에서 빌드된 ELF를 아티팩트로 저장. 소스 변경 없으면 캐시 사용. |
| M3.4 | 증분 빌드 | `build.rs`에서 Guest Program 소스 파일의 해시를 비교. 변경된 프로그램만 재빌드. |
| M3.5 | 사전 빌드 ELF | 개발 시에는 사전 빌드된 ELF를 사용. CI에서만 소스 빌드. |

### 예상 빌드 시간

| 시나리오 | 현재 | 모듈화 후 (2 프로그램) | 완화 후 |
|---------|------|----------------------|---------|
| SP1 ELF 빌드 | ~3분 | ~6분 | ~3분 (선택적 빌드) |
| RISC0 ELF 빌드 | ~2분 | ~4분 | ~2분 (선택적 빌드) |
| 전체 CI | ~10분 | ~16분 | ~10분 (ELF 캐싱) |

---

## R4. Public Input 인코딩 불일치

### 리스크

각 Guest Program 타입이 자체 `ProgramOutput.encode()` 구현을 가지므로:
- Guest Program의 encode()와 L1의 `_getPublicInputsFromCommitment()` 사이에 **바이트 단위 불일치**가 발생하면 증명 검증이 실패한다.
- 엔디안, 패딩, 필드 순서 등 미세한 차이가 검증 실패를 유발한다.
- 새 Guest Program 추가 시 L1에도 대응하는 인코딩 함수를 배포해야 하므로 **두 곳을 동시에 수정**해야 한다.

### 영향도: **높음**

인코딩 불일치는 디버깅이 매우 어렵다. 증명은 유효하지만 검증이 실패하면 원인 파악에 시간이 오래 걸린다.

### 완화 전략

| # | 전략 | 설명 |
|---|------|------|
| M4.1 | 인코딩 일치 테스트 | Rust에서 생성한 인코딩과 Solidity에서 재구성한 인코딩을 바이트 비교하는 크로스 언어 테스트. |
| M4.2 | 인코딩 스키마 명세 | 각 Guest Program의 public inputs 인코딩을 바이트 오프셋 단위로 문서화. |
| M4.3 | 공유 인코딩 생성기 | 인코딩 스키마 정의 파일에서 Rust `encode()`와 Solidity `_getPublicInputs()`를 **자동 생성**. |
| M4.4 | 단계적 검증 | 테스트넷에서 새 Guest Program의 인코딩을 먼저 검증한 뒤 메인넷 배포. |
| M4.5 | 범용 인코딩 포맷 | 장기적으로 ABI 인코딩 표준 (`abi.encode()` / `ethabi`)을 사용하여 양쪽 구현 통일. |

### 구체적 테스트 예시

```rust
#[test]
fn test_evm_l2_encoding_matches_solidity() {
    let output = ProgramOutput {
        initial_state_hash: H256::from([0x01; 32]),
        final_state_hash: H256::from([0x02; 32]),
        // ... 모든 필드 설정
    };

    let rust_encoded = output.encode();

    // Solidity에서 동일한 입력으로 _getPublicInputsFromCommitment() 실행
    let solidity_encoded = call_solidity_encoder(&output);

    assert_eq!(rust_encoded, solidity_encoded, "Encoding mismatch!");
}
```

---

## R5. 보안 — 커스텀 Guest Program

### 리스크

제3자가 만든 커스텀 Guest Program이 보안 검증을 우회할 수 있다:
- **상태 루트 검증 생략**: Guest Program이 초기/최종 상태 루트를 검증하지 않으면, 임의의 상태 전이를 증명할 수 있다.
- **public values 조작**: 잘못된 public values를 출력하면 L1이 잘못된 상태를 승인할 수 있다.
- **DoS**: 무한 루프나 과도한 메모리 사용으로 프루버를 공격할 수 있다.

### 영향도: **높음**

보안 위반은 자금 손실로 이어질 수 있다.

### 완화 전략

| # | 전략 | 설명 |
|---|------|------|
| M5.1 | 필수 검증 항목 | `GuestProgram` 트레이트에 **반드시 포함해야 하는 검증** 목록 정의: (1) 초기 상태 루트 검증, (2) 최종 상태 루트 검증, (3) 체인 ID 포함. |
| M5.2 | 공통 검증 래퍼 | `execute_blocks()`처럼 공통 검증 로직을 제공하는 라이브러리 함수를 만들고, 커스텀 Guest Program이 이를 호출하도록 권장. |
| M5.3 | VK 등록 제한 | L1에서 VK를 등록할 수 있는 권한을 Timelock(governance)으로 제한. 임의의 Guest Program VK를 등록할 수 없도록. |
| M5.4 | ELF 화이트리스트 | 프루버가 등록되지 않은 ELF를 실행하지 않도록 레지스트리에서 관리. 파일 시스템에서 임의 ELF 로딩 금지. |
| M5.5 | 감사 가이드라인 | 커스텀 Guest Program 감사 시 확인해야 할 체크리스트 작성. |

### 필수 검증 체크리스트

모든 Guest Program이 반드시 보장해야 하는 사항:

```
[필수] 초기 상태 루트가 이전 배치의 최종 상태 루트와 일치
[필수] 최종 상태 루트가 마지막 블록 헤더의 state_root와 일치
[필수] 체인 ID가 public values에 포함
[필수] 블록 해시가 public values에 포함
[권장] 트랜잭션 서명 검증
[권장] 가스 사용량 검증
[권장] 이중 지출 방지
```

---

## R6. 크레이트 의존성 순환

### 리스크

`GuestProgram` 트레이트가 `BackendType`을 참조하면:
```
guest-program → (BackendType을 위해) → l2-prover → (ProgramInput을 위해) → guest-program
```
순환 의존성이 발생한다.

### 영향도: **중간**

컴파일 불가능하므로 아키텍처 재설계가 필요하다.

### 완화 전략

| # | 전략 | 설명 |
|---|------|------|
| M6.1 | 공유 타입 크레이트 | `BackendType`, `ProverType` 등 공유 타입을 별도 크레이트(예: `ethrex-prover-types`)로 분리. |
| M6.2 | 문자열 기반 백엔드 식별 | `BackendType` 대신 `&str` (예: `"sp1"`, `"risc0"`)로 백엔드를 식별. 의존성 제거. |
| M6.3 | 트레이트 위치 변경 | `GuestProgram` 트레이트를 `guest-program` 대신 `l2-common`에 정의. |

**추천**: M6.1 — 공유 타입 크레이트. 가장 깔끔하고 향후 확장에도 유리하다.

```
ethrex-prover-types (신규)
├── BackendType
├── ProverType
├── ProofFormat
└── GuestProgramError

guest-program ──depends──▶ ethrex-prover-types
l2-prover    ──depends──▶ ethrex-prover-types
l2-common    ──depends──▶ ethrex-prover-types
```

---

## R7. Upstream 호환성

### 리스크

ethrex upstream(lambdaclass)이 `ProverBackend` 트레이트나 Guest Program 구조를 변경하면:
- Tokamak 포크의 모듈화 코드와 충돌이 발생한다.
- Merge conflict 해결 비용이 증가한다.
- Upstream의 최적화(Phase 3)를 가져오기 어려워진다.

### 영향도: **중간**

장기적 유지보수 비용에 영향.

### 완화 전략

| # | 전략 | 설명 |
|---|------|------|
| M7.1 | 최소 변경 원칙 | 기존 코드를 수정하기보다 새 파일/모듈을 추가하는 방식 선호. upstream 파일과의 충돌 최소화. |
| M7.2 | 어댑터 패턴 | 기존 `ProverBackend`를 직접 수정하지 않고, 래퍼 레이어(`ModularProverBackend`)를 추가. |
| M7.3 | 정기 리베이스 | upstream `main`을 주기적으로 (2주) 리베이스하여 차이를 최소화. |
| M7.4 | upstream 제안 | 모듈화 설계가 안정되면 upstream에 RFC/PR 형태로 제안. 채택되면 유지보수 부담 해소. |

---

## R8. 성능 오버헤드

### 리스크

추상화 레이어 추가로 인한 성능 저하:
- `GuestProgramRegistry` 조회 오버헤드
- bytes 기반 인터페이스의 추가 직렬화/역직렬화
- ~~ELF별 셋업 반복 (SP1의 `client.setup(elf)`)~~ → **해결됨**

### 영향도: **낮음**

레지스트리 조회와 직렬화는 증명 생성 시간(수십 초~수십 분) 대비 무시 가능.

### 완화 전략

| # | 전략 | 설명 | 상태 |
|---|------|------|------|
| M8.1 | ELF 셋업 캐싱 | SP1의 `(pk, vk) = client.setup(elf)` 결과를 SHA-256(elf) 해시 기반으로 캐싱. `ELF_KEY_CACHE: OnceLock<Mutex<HashMap<[u8;32], (PK, VK)>>>`. | **✅ 구현됨** |
| M8.2 | 레지스트리 사전 초기화 | 프루버 시작 시 레지스트리의 모든 프로그램에 대해 셋업 완료. 런타임 조회는 `HashMap::get()` (O(1)). | 적용됨 |
| M8.3 | 벤치마크 비교 | 모듈화 전후로 동일 배치에 대한 증명 시간을 측정. 5% 이상 차이나면 최적화. | 향후 |

---

## 리스크 매트릭스 요약

| 리스크 | 영향도 | 발생 가능성 | 전체 수준 | 주요 완화 |
|--------|--------|-----------|----------|----------|
| R1. 하위 호환성 | 높음 | 중간 | **높음** | M1.1-M1.5: Zero change + serde default + feature flag |
| R2. L1 컨트랙트 업그레이드 | 높음 | 중간 | **높음** | M2.1-M2.5: UUPS proxy + 마이그레이션 + fork 테스트 |
| R3. 빌드 시간 | 중간 | 높음 | **중간** | M3.1-M3.5: 선택적 빌드 + 캐싱 |
| R4. 인코딩 불일치 | 높음 | 중간 | **높음** | M4.1-M4.5: 크로스 언어 테스트 + 자동 생성 |
| R5. 보안 (커스텀 GP) | 높음 | 낮음 | **중간** | M5.1-M5.5: 필수 검증 + VK 권한 + 감사 |
| R6. 의존성 순환 | 중간 | ~~높음~~ | ~~**중간**~~ **해결됨** | M6.2: `&str` 기반 백엔드 식별 (`backends::SP1` 등) |
| R7. Upstream 호환 | 중간 | 중간 | **중간** | M7.1-M7.4: 최소 변경 + 정기 리베이스 |
| R8. 성능 오버헤드 | 낮음 | ~~낮음~~ | ~~**낮음**~~ **완화됨** | M8.1: ELF 해시 기반 SP1 setup 캐싱 구현 완료 |

## 우선 대응 항목

Phase 2.1 시작 전 반드시 해결:
1. **R6 (의존성 순환)**: `BackendType` 위치 결정 → 공유 크레이트 생성 또는 문자열 기반 식별자 선택
2. **R1 (하위 호환성)**: 기존 테스트 목록 확보 및 CI regression 테스트 설정

Phase 2.3 시작 전 반드시 해결:
3. **R2 (L1 업그레이드)**: 스토리지 레이아웃 분석 및 마이그레이션 전략 확정
4. **R4 (인코딩 불일치)**: 크로스 언어 인코딩 테스트 프레임워크 구축
