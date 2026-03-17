# Guest Program 모듈화 — 설계 개요

## 문제 정의

현재 ethrex의 Guest Program은 **단일 모놀리식 구조**로 되어 있다.
`crates/guest-program/src/` 전체가 하나의 EVM 실행 프로그램이며, L1/L2 분기만 feature flag(`l2`)로 처리한다.

이 구조에서는 다음이 불가능하다:

- **Guest Program 교체**: 프루버가 다른 로직(DEX, 결제 전용 등)을 증명하려면 코드 전체를 포크해야 한다.
- **앱 전용 L2 배포**: Tokamak 플랫폼에서 앱 개발자가 자기 L2에 맞는 Circuit을 넣을 수 없다.
- **ELF 런타임 선택**: ELF 바이너리가 `include_bytes!()`로 컴파일 타임에 고정되어 있어, 같은 프루버 바이너리로 다른 Guest Program을 실행할 수 없다.
- **L1 멀티프로그램 검증**: `OnChainProposer.sol`의 VK 매핑이 `(commitHash, verifierId)` 2차원이므로, 동일 커밋에서 여러 Guest Program을 구분할 수 없다.

## 목표

**Guest Program을 교체 가능한 플러그인 구조로 만든다.**

```
현재 아키텍처:
  Prover → [고정된 Guest Program (EVM)] → Proof → L1 검증

목표 아키텍처:
  Prover → [GuestProgramRegistry] → [선택된 Guest Program] → Proof → L1 검증
                    │
                    ├── EVM-L2 (기본, 범용)
                    ├── Transfer Circuit (단순 전송 전용)
                    ├── DEX Circuit (주문 매칭 특화)
                    └── Custom Circuit (개발자 제작)
```

핵심 목표:
1. `GuestProgram` 트레이트 정의 — bytes 수준 추상화로 타입 폭발 방지
2. `ProverBackend`를 Guest Program에 무관하게 만들기 — ELF + bytes 입력만 받도록
3. Guest Program 레지스트리 — 런타임에 program_id로 ELF/VK 선택
4. L1 컨트랙트 확장 — 프로그램 타입별 VK 관리 및 검증
5. 멀티 ELF 빌드 시스템 — 프로그램별 독립 크레이트 및 빌드

## 범위

### 포함 (In Scope)

| 항목 | 설명 |
|------|------|
| `GuestProgram` 트레이트 | Guest Program 인터페이스 정의 및 기존 EVM 구현 래핑 |
| `ProverBackend` 리팩토링 | `ProgramInput` 직접 참조 제거, bytes 기반 인터페이스 |
| `GuestProgramRegistry` | 런타임 프로그램 등록/조회 |
| L1 컨트랙트 확장 | VK 매핑에 `programTypeId` 차원 추가 |
| `ProofData` 프로토콜 확장 | `program_id` 필드 추가 |
| 빌드 시스템 확장 | 멀티 프로그램 ELF 빌드 |
| Transfer Circuit 레퍼런스 | 단순 전송 전용 Guest Program 예제 |

### 제외 (Out of Scope)

| 항목 | 이유 |
|------|------|
| ZK 최적화 (Phase 3) | 모듈화 완료 후 별도 진행 |
| 병렬 블록 실행 (Phase 4) | 모듈화와 독립적으로 진행 |
| 시뇨리지 마이닝 (Phase 6) | 경제모델은 별도 설계 |
| 프로덕션 보안 감사 | 구현 완료 후 별도 진행 |
| L1 프로그램 모듈화 | L1 Guest Program은 현행 유지 |

## 용어 정의

| 용어 | 정의 |
|------|------|
| **Guest Program** | zkVM 내부에서 실행되는 프로그램. 입력을 받아 상태 전이를 검증하고 공개 값(public values)을 출력한다. |
| **Circuit** | Guest Program의 다른 표현. 특정 연산만 증명하도록 최적화된 Guest Program을 가리킬 때 주로 사용한다. |
| **ELF** | Executable and Linkable Format. Guest Program을 RISC-V 타겟으로 컴파일한 바이너리. zkVM이 이를 실행한다. |
| **VK (Verification Key)** | ELF를 컴파일/셋업한 결과물. L1에서 증명을 검증할 때 사용한다. 동일 ELF는 항상 동일 VK를 생성한다. |
| **ProverBackend** | 증명 생성 엔진 (SP1, RISC0, ZisK, OpenVM 등). ELF를 실행하고 증명을 생성한다. |
| **BackendType** | ProverBackend의 종류를 나타내는 열거형 (`SP1`, `RISC0`, `ZisK`, `OpenVM`, `Exec`). |
| **ProgramInput** | Guest Program에 전달되는 입력 데이터. 블록, 실행 증거, 설정 등을 포함한다. |
| **ProgramOutput** | Guest Program이 출력하는 공개 값. L1 컨트랙트의 `_getPublicInputsFromCommitment()`과 일치해야 한다. |
| **BatchProof** | 프루버가 생성한 배치 증명. `ProofCalldata`(on-chain 검증용) 또는 `ProofBytes`(Aligned 등) 형태. |
| **program_id** | Guest Program을 식별하는 문자열 (예: `"evm-l2"`, `"transfer"`, `"dex"`). |
| **programTypeId** | L1 컨트랙트에서 Guest Program 종류를 식별하는 정수 (예: 1=EVM-L2, 2=Transfer). |
| **Proof Coordinator** | 시퀀서 내부 컴포넌트. 프루버에게 배치를 할당하고 증명을 수집한다. |

## 관련 문서

| 문서 | 설명 |
|------|------|
| `01-current-architecture.md` | 현재 아키텍처 상세 분석 |
| `02-target-architecture.md` | 목표 아키텍처 설계 |
| `03-implementation-phases.md` | 구현 단계별 계획 |
| `04-risk-analysis.md` | 리스크 분석 및 완화 전략 |
| `../zk-optimization-plan.md` | 전체 ZK 최적화 계획 (Phase 1-6) |

## 설계 원칙

1. **기존 동작 무변경 (Zero Behavior Change)**: 모듈화 후에도 기존 EVM-L2 Guest Program은 동일하게 동작해야 한다. 모든 기존 테스트가 통과해야 한다.

2. **Bytes 수준 추상화**: Generic 타입 폭발을 피하기 위해 `GuestProgram` 트레이트는 bytes 수준에서 동작한다. 각 Guest Program은 자체 타입 시스템을 가지되, 프루버 인터페이스에서는 `&[u8]`로 통일한다.

3. **확장, 교체 아님**: 기존 `ProverBackend` 트레이트를 교체하지 않고 확장한다. SP1/RISC0 등 백엔드 코드의 변경을 최소화한다.

4. **점진적 마이그레이션**: 한 번에 전체를 바꾸지 않고, 단계별로 추상화를 도입한다. 각 단계마다 중간 상태가 작동 가능해야 한다.

5. **독립 빌드**: 각 Guest Program은 독립된 크레이트로 관리한다. 하나의 Guest Program을 수정해도 다른 것에 영향이 없어야 한다.
