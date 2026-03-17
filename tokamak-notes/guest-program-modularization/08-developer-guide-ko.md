# Guest Program 개발자 가이드

이 가이드는 Tokamak zkVM 프레임워크용 커스텀 Guest Program을 생성하는 방법을 설명합니다. Guest Program은 zkVM(SP1, RISC0, ZisK, OpenVM) 내부에서 실행되는 독립적인 RISC-V 바이너리로, 올바른 실행의 암호학적 증명을 생성합니다.

## 아키텍처 개요

```
┌─────────────────────────────────────────────────────┐
│ crates/guest-program/                               │
│                                                     │
│  src/                                               │
│   ├── traits.rs          GuestProgram 트레이트 +    │
│   │                      ELF 검증 유틸리티          │
│   ├── programs/                                     │
│   │   ├── mod.rs         모듈 레지스트리            │
│   │   ├── dynamic.rs     런타임 ELF 로더            │
│   │   ├── evm_l2.rs      기본 EVM-L2 프로그램       │
│   │   ├── zk_dex/        ZK-DEX 레퍼런스            │
│   │   │   ├── mod.rs     GuestProgram 구현 + 테스트 │
│   │   │   ├── types.rs   입출력 타입 (rkyv)         │
│   │   │   └── execution.rs  비즈니스 로직           │
│   │   └── tokamon/       Tokamon 레퍼런스            │
│   │       ├── mod.rs                                │
│   │       ├── types.rs                              │
│   │       └── execution.rs                          │
│   └── lib.rs             ELF 상수 + re-exports      │
│                                                     │
│  bin/                                               │
│   ├── sp1/               EVM-L2 SP1 바이너리        │
│   ├── sp1-zk-dex/        ZK-DEX SP1 바이너리        │
│   ├── sp1-tokamon/       Tokamon SP1 바이너리        │
│   ├── risc0/             RISC0 바이너리             │
│   └── zisk/              ZisK 바이너리              │
│                                                     │
│  scripts/                                           │
│   └── new-guest-program.sh  스캐폴드 생성기         │
└─────────────────────────────────────────────────────┘
```

### 핵심 개념

- **`GuestProgram` 트레이트** (`traits.rs`): 핵심 추상화. 모든 Guest Program은 이 트레이트를 구현합니다.
- **프로그램 레지스트리** (`crates/l2/prover/src/registry.rs`): 프루버 시작 시 `program_id` → `Arc<dyn GuestProgram>` 매핑을 관리합니다.
- **ELF 바이너리**: zkVM 내부에서 실행되는 컴파일된 RISC-V 실행 파일. 프로그램당 백엔드별로 하나의 ELF가 필요합니다.
- **`program_type_id`**: L1의 VK 매핑에서 프로그램 타입을 식별하는 데 사용되는 정수(u8)입니다.

---

## 빠른 시작: 새 프로그램 스캐폴딩

새 Guest Program을 만드는 가장 빠른 방법은 스캐폴드 스크립트입니다:

```bash
# 저장소 루트에서 실행
./scripts/new-guest-program.sh my-awesome-program
```

생성되는 파일:

| 파일 | 용도 |
|------|------|
| `src/programs/my_awesome_program/types.rs` | rkyv + serde 입출력 타입 |
| `src/programs/my_awesome_program/execution.rs` | 실행 로직 스켈레톤 |
| `src/programs/my_awesome_program/mod.rs` | `GuestProgram` 트레이트 구현 + 8개 테스트 |
| `bin/sp1-my-awesome-program/Cargo.toml` | SP1 게스트 바이너리 설정 |
| `bin/sp1-my-awesome-program/src/main.rs` | SP1 zkVM 진입점 |

스캐폴드는 `programs/mod.rs`에 모듈을 자동 등록하고, 사용 가능한 다음 `program_type_id`를 할당합니다.

스캐폴딩 후 다음 세 파일을 커스터마이즈하세요:

1. **`types.rs`** — 입출력 데이터 구조 정의
2. **`execution.rs`** — 비즈니스 로직 구현
3. **`mod.rs`** — ELF 컴파일 후 `elf()` 업데이트

---

## 단계별 가이드: 수동 생성

### 1. 타입 정의 (`types.rs`)

입력 타입은 반드시 `rkyv` 트레이트(zkVM 직렬화용)와 `serde` 트레이트(JSON/설정용)를 derive해야 합니다:

```rust
use rkyv::{Archive, Deserialize as RDeserialize, Serialize as RSerialize};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, RSerialize, RDeserialize, Archive, Clone, Debug)]
pub struct MyProgramInput {
    pub initial_state_root: [u8; 32],
    // 도메인 특화 필드:
    pub transfers: Vec<Transfer>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct MyProgramOutput {
    pub initial_state_root: [u8; 32],
    pub final_state_root: [u8; 32],
    pub item_count: u64,
}
```

출력 타입에는 L1 검증자가 기대하는 바이트 레이아웃을 생성하는 `encode()` 메서드가 필요합니다:

```rust
impl MyProgramOutput {
    pub fn encode(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(72);
        buf.extend_from_slice(&self.initial_state_root);
        buf.extend_from_slice(&self.final_state_root);
        buf.extend_from_slice(&self.item_count.to_be_bytes());
        buf
    }
}
```

### 2. 실행 로직 구현 (`execution.rs`)

실행 함수는 핵심 비즈니스 로직입니다. 입력을 받아 검증하고, 상태 전이를 계산하고, 출력을 반환합니다:

```rust
use ethrex_crypto::keccak::keccak_hash;
use super::types::{MyProgramInput, MyProgramOutput};

#[derive(Debug, thiserror::Error)]
pub enum MyExecutionError {
    #[error("빈 입력")]
    EmptyInput,
    #[error("잘못된 전송: {0}")]
    InvalidTransfer(String),
}

pub fn execution_program(
    input: MyProgramInput,
) -> Result<MyProgramOutput, MyExecutionError> {
    if input.transfers.is_empty() {
        return Err(MyExecutionError::EmptyInput);
    }

    let mut state = input.initial_state_root;

    for transfer in &input.transfers {
        // 각 전송 검증...
        // 결정론적으로 상태 업데이트:
        let mut preimage = Vec::new();
        preimage.extend_from_slice(&state);
        // ... 전송 데이터 추가 ...
        state = keccak_hash(&preimage);
    }

    Ok(MyProgramOutput {
        initial_state_root: input.initial_state_root,
        final_state_root: state,
        item_count: input.transfers.len() as u64,
    })
}
```

**중요**: 실행 함수는 반드시 **결정론적**이어야 합니다 — 같은 입력은 항상 같은 출력을 생성해야 합니다. zkVM은 프루버 내부에서 이 함수를 재실행합니다.

### 3. 트레이트 구현 (`mod.rs`)

```rust
pub mod execution;
pub mod types;

use crate::traits::{GuestProgram, GuestProgramError, backends};

pub struct MyGuestProgram;

impl MyGuestProgram {
    fn non_empty(elf: &[u8]) -> Option<&[u8]> {
        if elf.is_empty() || elf == [0] { None } else { Some(elf) }
    }
}

impl GuestProgram for MyGuestProgram {
    fn program_id(&self) -> &str {
        "my-program"
    }

    fn elf(&self, backend: &str) -> Option<&[u8]> {
        match backend {
            backends::SP1 => Self::non_empty(crate::ZKVM_SP1_MY_PROGRAM_ELF),
            _ => None,
        }
    }

    fn vk_bytes(&self, _backend: &str) -> Option<Vec<u8>> {
        None
    }

    fn program_type_id(&self) -> u8 {
        4 // L1 VK 매핑용 고유 정수
    }
}
```

### 4. SP1 바이너리 생성

각 Guest Program에는 zkVM 진입점 바이너리가 필요합니다. SP1의 경우:

**`bin/sp1-my-program/Cargo.toml`**:
```toml
[package]
name = "ethrex-guest-sp1-my-program"
version = "9.0.0"
edition = "2024"
license = "MIT OR Apache-2.0"

[workspace]

[profile.release]
lto = "thin"
codegen-units = 1

[dependencies]
sp1-zkvm = { version = "=5.0.8" }
rkyv = { version = "0.8.10", features = ["std", "unaligned"] }
ethrex-guest-program = { path = "../../", default-features = false }

[patch.crates-io]
tiny-keccak = { git = "https://github.com/sp1-patches/tiny-keccak", tag = "patch-2.0.2-sp1-4.0.0" }
```

**`bin/sp1-my-program/src/main.rs`**:
```rust
#![no_main]

use ethrex_guest_program::programs::my_program::execution::execution_program;
use ethrex_guest_program::programs::my_program::types::MyProgramInput;
use rkyv::rancor::Error;

sp1_zkvm::entrypoint!(main);

pub fn main() {
    let input = sp1_zkvm::io::read_vec();
    let input = rkyv::from_bytes::<MyProgramInput, Error>(&input).unwrap();

    let output = execution_program(input).unwrap();

    sp1_zkvm::io::commit_slice(&output.encode());
}
```

**핵심 사항**:
- SP1에는 `#![no_main]`과 `sp1_zkvm::entrypoint!(main)`이 필수입니다
- 입력은 `sp1_zkvm::io::read_vec()`로 읽습니다 (원시 바이트)
- 출력은 `sp1_zkvm::io::commit_slice()`로 커밋합니다 (공개 값)
- `ethrex-crypto`의 riscv32 keccak을 위해 `tiny-keccak` 패치가 필요합니다

### 5. 빌드 시스템 연결

**`lib.rs`** — ELF 상수 추가:
```rust
#[cfg(all(not(clippy), feature = "sp1"))]
pub static ZKVM_SP1_MY_PROGRAM_ELF: &[u8] =
    include_bytes!("../bin/sp1-my-program/out/riscv32im-succinct-zkvm-elf");
#[cfg(any(clippy, not(feature = "sp1")))]
pub const ZKVM_SP1_MY_PROGRAM_ELF: &[u8] = &[];
```

**`build.rs`** — 빌드 함수 추가 (기존 패턴과 동일):
```rust
if programs.contains(&"my-program".to_string()) {
    #[cfg(all(not(clippy), feature = "sp1"))]
    build_sp1_my_program();
}
```

**`Makefile`** — 타겟 추가:
```makefile
sp1-my-program:
	$(ENV_PREFIX) GUEST_PROGRAMS=my-program cargo check $(CARGO_FLAGS) --features sp1
```

### 6. 프루버에 등록

`crates/l2/prover/src/prover.rs`에서 `create_default_registry()`에 프로그램을 추가합니다:

```rust
fn create_default_registry() -> GuestProgramRegistry {
    let mut reg = GuestProgramRegistry::new("evm-l2");
    reg.register(Arc::new(EvmL2GuestProgram));
    reg.register(Arc::new(MyGuestProgram));  // <-- 여기에 추가
    reg
}
```

---

## 동적 ELF 로딩 (재컴파일 불필요)

ELF 바이너리가 디스크 파일로 제공되는 프로그램(예: Guest Program Store에서 다운로드한 경우)에는 `DynamicGuestProgram`을 사용합니다:

### 디렉토리에서 로드

```rust
use ethrex_guest_program::programs::dynamic::DynamicGuestProgram;

// 디렉토리 구조: /opt/elfs/my-program/sp1/elf, /opt/elfs/my-program/risc0/elf, ...
let program = DynamicGuestProgram::from_dir(
    "my-program",
    10,  // program_type_id
    "/opt/elfs/my-program",
)?;

// 등록
registry.register(Arc::new(program));
```

### 빌더 사용

```rust
use ethrex_guest_program::programs::dynamic::DynamicGuestProgram;
use ethrex_guest_program::traits::backends;

let program = DynamicGuestProgram::builder("my-program", 10)
    .elf_from_file(backends::SP1, "/path/to/sp1.elf")?
    .elf_from_file(backends::RISC0, "/path/to/risc0.elf")?
    .vk_from_bytes(backends::RISC0, risc0_image_id.to_vec())
    .build();
```

### 원시 바이트에서 로드

```rust
let elf_bytes: Vec<u8> = download_elf_from_store("my-program", "sp1").await?;

let program = DynamicGuestProgram::builder("my-program", 10)
    .elf_from_bytes(backends::SP1, elf_bytes)?
    .build();
```

ELF 헤더 검증(매직 넘버, RISC-V 클래스, 머신 타입)은 기본적으로 수행됩니다. 검증을 건너뛰려면:

```rust
let program = DynamicGuestProgram::builder("my-program", 10)
    .skip_validation()
    .elf_from_bytes(backends::SP1, raw_bytes)?
    .build();
```

---

## `GuestProgram` 트레이트 레퍼런스

```rust
pub trait GuestProgram: Send + Sync {
    /// 고유 ID (예: "evm-l2", "zk-dex").
    fn program_id(&self) -> &str;

    /// 백엔드용 ELF 바이너리. 미지원 시 None 반환.
    fn elf(&self, backend: &str) -> Option<&[u8]>;

    /// 백엔드용 검증 키 바이트.
    fn vk_bytes(&self, backend: &str) -> Option<Vec<u8>>;

    /// L1 프로그램 타입 식별자.
    fn program_type_id(&self) -> u8;

    /// 원시 입력 바이트 직렬화 (기본: 패스스루).
    fn serialize_input(&self, raw: &[u8]) -> Result<Vec<u8>, GuestProgramError>;

    /// 원시 출력 바이트를 L1용으로 인코딩 (기본: 패스스루).
    fn encode_output(&self, raw: &[u8]) -> Result<Vec<u8>, GuestProgramError>;

    /// ELF 헤더 검증 (기본: 매직 + 클래스 + 머신 검사).
    fn validate_elf(&self, backend: &str, elf: &[u8]) -> Result<(), GuestProgramError>;
}
```

### 백엔드 상수

`backend` 파라미터로 `backends::SP1`, `backends::RISC0`, `backends::ZISK`, `backends::OPENVM`, `backends::EXEC`를 사용합니다.

### 에러 타입

```rust
pub enum GuestProgramError {
    Serialization(String),
    UnsupportedBackend(String),
    InvalidElf(String),
    Internal(String),
}
```

---

## 테스트

모든 Guest Program은 다음 항목을 검증하는 테스트를 포함해야 합니다:

| 테스트 | 검증 내용 |
|--------|----------|
| `program_id_is_correct` | ID가 레지스트리에서 사용하는 문자열과 일치 |
| `program_type_id_is_correct` | L1용 고유 정수 |
| `unsupported_backend_returns_none` | 존재하지 않는 백엔드 → None |
| `serialize_input_is_identity` | 패스스루 직렬화 |
| `execution_produces_deterministic_output` | 동일 입력 → 동일 출력 |
| `execution_rejects_empty_input` | 빈/잘못된 입력에 대한 에러 |
| `output_encode_length` | 바이트 레이아웃이 L1 기대값과 일치 |
| `rkyv_roundtrip` | 직렬화 → 역직렬화가 데이터를 보존 |

테스트 실행:

```bash
# 모든 guest-program 테스트
cargo test -p ethrex-guest-program

# 모든 프루버 테스트 (레지스트리 통합 포함)
cargo test -p ethrex-prover

# 둘 다
cargo test -p ethrex-guest-program -p ethrex-prover
```

---

## 기존 프로그램 레퍼런스

| 프로그램 | ID | 타입 ID | 설명 |
|---------|-----|---------|------|
| `EvmL2GuestProgram` | `evm-l2` | 1 | 기본 EVM-L2 블록 실행 |
| `ZkDexGuestProgram` | `zk-dex` | 2 | 프라이버시 보존 DEX 전송 |
| `TokammonGuestProgram` | `tokamon` | 3 | 위치 기반 보상 게임 |

---

## 체크리스트

새 Guest Program 제출 전:

- [ ] 타입이 `rkyv` (`Archive`, `RSerialize`, `RDeserialize`) 및 `serde` 트레이트를 derive
- [ ] 실행 함수가 결정론적 (난수 없음, 시스템 콜 없음)
- [ ] 출력 `encode()`가 L1 검증자가 기대하는 바이트 레이아웃과 일치
- [ ] `program_type_id`가 등록된 모든 프로그램에서 고유
- [ ] SP1 바이너리가 `#![no_main]`과 `sp1_zkvm::entrypoint!`로 컴파일
- [ ] 8개 표준 테스트 모두 통과
- [ ] `create_default_registry()`에 프로그램 등록 (또는 동적으로 로드)
- [ ] `lib.rs`와 `build.rs`에 ELF 상수 추가 (컴파일 타임 임베딩용)
