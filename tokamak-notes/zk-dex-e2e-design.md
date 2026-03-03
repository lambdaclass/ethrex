# ZK-DEX E2E 실행 설계 문서

**작성일**: 2026-02-24
**최종 업데이트**: 2026-02-27
**브랜치**: `feat/zk/zk-dex-full-circuit`
**선행 문서**: `zk-dex-progress.md`, `12-app-specific-circuit-dev-plan.md`

---

## 1. 목표

Mock 데이터가 아닌 **실제 L1/L2 환경**에서 ZK-DEX 트랜잭션을 실행하고,
end-to-end로 SP1 증명을 생성하여 L1에서 검증되는 것을 확인한다.

```
L1 (Anvil)                          L2 (ethrex)
┌──────────────────┐               ┌──────────────────────────┐
│ OnChainProposer  │◀─ commit ────│ L1 Committer             │
│   (programTypeId │◀─ verify ────│ L1 Proof Sender          │
│    = 2: zk-dex)  │               │                          │
│                  │               │ Proof Coordinator        │
│ GuestProgram     │               │   (guest_program_id =    │
│   Registry       │               │    "zk-dex")             │
│                  │               │                          │
│ SP1 Verifier     │               │ Sequencer (EVM 실행)     │
└──────────────────┘               └────────────┬─────────────┘
                                                │
                                   ┌────────────▼─────────────┐
                                   │ Prover                   │
                                   │   registry: zk-dex       │
                                   │   ELF: sp1-zk-dex        │
                                   │   serialize_input():     │
                                   │     ProgramInput →       │
                                   │     AppProgramInput      │
                                   └──────────────────────────┘
```

---

## 2. 핵심 설계 결정: 표준 ProgramOutput

### 2.1 플랫폼 규칙

**모든 Guest Program은 동일한 `ProgramOutput` 포맷을 출력해야 한다.**

```rust
// crates/guest-program/src/l2/output.rs — 플랫폼 표준 출력
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

ZK-DEX의 `execute_app_circuit()`은 이미 이 타입을 반환한다:
```rust
// app_execution.rs:312
// 7. Build output (same format as evm-l2 ProgramOutput).
Ok(ProgramOutput {
    initial_state_hash: input.prev_state_root,
    final_state_hash,
    l1_out_messages_merkle_root: ...,
    // ... 모든 필드가 EVM-L2와 동일
})
```

### 2.2 이 결정이 가져오는 이점

`ProgramOutput`이 표준 포맷이므로, L1 컨트랙트는 **프로그램 종류에 관계없이
동일한 방식으로 public inputs를 재구성**할 수 있다:

```solidity
// OnChainProposer.sol — 기존 코드 그대로 동작
publicInputs = _getPublicInputsFromCommitment(batchNumber);
```

프로그램별로 달라지는 것은 **VK(Verification Key)뿐**이다:
```solidity
// VK 조회: commitHash × programTypeId × verifierId
bytes32 sp1Vk = verificationKeys[batchCommitHash][batchProgramTypeId][SP1_VERIFIER_ID];
```

| 항목 | EVM-L2 | ZK-DEX | 비고 |
|------|--------|--------|------|
| programTypeId | 1 | 2 | VK 조회 키로만 사용 |
| ProgramOutput 포맷 | 표준 | 표준 (동일) | 플랫폼 규칙 |
| L1 public inputs 재구성 | `_getPublicInputsFromCommitment()` | 동일 | 변경 불필요 |
| VK | EVM-L2 ELF 기반 | ZK-DEX ELF 기반 | 다름 |
| `publicValuesHash` | 불필요 (`H256::zero()`) | 불필요 (`H256::zero()`) | 동일 |
| `customPublicValues` | 빈 바이트 | 빈 바이트 | 동일 |
| commit → prove → verify 순서 | 기존 유지 | 기존 유지 | 변경 불필요 |

### 2.3 왜 `customPublicValues` 방식을 쓰지 않는가

OnChainProposer는 `programTypeId > 1`인 경우 `customPublicValues` 경로를 제공하지만,
이는 **ProgramOutput이 표준 포맷이 아닌 프로그램**을 위한 것이다.

ZK-DEX는 표준 `ProgramOutput`을 출력하므로 이 경로가 필요 없다.
L1 컨트랙트가 commitment 데이터로부터 public inputs를 직접 재구성하며,
SP1 증명이 이를 검증한다. 증명이 맞지 않으면 검증 실패.

```
시퀀서 commit: "state root 0xAAA → 0xBBB"
                    ↓
L1 저장 (commitment 데이터)
                    ↓
프루버 증명: DexCircuit 실행 → ProgramOutput(0xAAA → 0xBBB)
                    ↓
L1 검증:
  1. commitment에서 public inputs 재구성
  2. SP1.verify(vk, publicInputs, proof)
  3. 증명의 public values ≠ commitment → 검증 실패 → 시퀀서 거짓말 탐지
```

---

## 3. 현재 상태 분석

### 3.1 이미 구현된 것 (코드 레벨)

| 영역 | 상태 | 파일 |
|------|------|------|
| AppCircuit 트레이트 + 공통 실행 엔진 | ✅ 완료 | `guest-program/src/common/app_execution.rs` |
| AppState (storage proof 기반 상태) | ✅ 완료 | `guest-program/src/common/app_state.rs` |
| AppProgramInput 타입 | ✅ 완료 | `guest-program/src/common/app_types.rs` |
| 증분 MPT 업데이트 | ✅ 완료 | `guest-program/src/common/incremental_mpt.rs` |
| ProgramInput → AppProgramInput 변환 | ✅ 완료 | `guest-program/src/common/input_converter.rs` |
| DexCircuit (token transfer) | ✅ 완료 | `guest-program/src/programs/zk_dex/circuit.rs` |
| ZkDexGuestProgram.serialize_input() | ✅ 완료 | `guest-program/src/programs/zk_dex/mod.rs` |
| SP1 ZK-DEX 바이너리 | ✅ 완료 | `guest-program/bin/sp1-zk-dex/src/main.rs` |
| SP1 crypto precompile 패치 | ✅ 완료 | `guest-program/bin/sp1-zk-dex/Cargo.toml` |
| Proof Coordinator: program_id 라우팅 | ✅ 완료 | `l2/sequencer/proof_coordinator.rs` |
| Prover: GuestProgramRegistry + ELF 디스패치 | ✅ 완료 | `l2/prover/src/prover.rs` |
| L1 Committer: resolve_program_type_id() | ✅ 완료 | `l2/sequencer/l1_committer.rs:1566` |
| OnChainProposer: programTypeId별 VK 조회 | ✅ 완료 | `contracts/src/l1/OnChainProposer.sol` |
| GuestProgramRegistry 컨트랙트 | ✅ 완료 | `contracts/src/l1/GuestProgramRegistry.sol` |
| **ProgramOutput 호환** | ✅ 동일 | `app_execution.rs` → `l2/output.rs` 동일 타입 사용 |

### 3.2 E2E 실행을 막던 갭 — 모두 해결됨

| # | 갭 | 상태 | 해결 방법 |
|---|-----|------|----------|
| G1 | ZK-DEX VK가 L1에 미등록 | ✅ 해결 | `deployer.rs`에서 `upgradeVerificationKey()` 호출 |
| G2 | GuestProgramRegistry에 zk-dex 미등록 | ✅ 해결 | `deployer.rs`에서 `registerOfficialProgram()` 호출 |
| G3 | ZK-DEX ELF가 기본 빌드에 미포함 | ✅ 해결 | Makefile에 `GUEST_PROGRAMS=evm-l2,zk-dex` 설정 |
| G4 | Makefile에 ZK-DEX 전용 타겟 없음 | ✅ 해결 | 3개 타겟 추가 |

> **참고**: `publicValuesHash`, `customPublicValues`, commit 순서 변경은
> ZK-DEX가 표준 `ProgramOutput`을 사용하므로 **갭이 아니었다** (§2 참조).

---

## 4. 아키텍처: 전체 데이터 흐름

### 4.1 EVM L2 (기존) vs ZK-DEX 비교

```
                        EVM L2                         ZK-DEX
                        ──────                         ──────
시퀀서 실행             EVM interpreter               EVM interpreter (동일)
                             │                              │
                             ▼                              ▼
ProverInputData         ExecutionWitness              ExecutionWitness (동일)
                        (전체 state trie 노드)         (전체 state trie 노드)
                             │                              │
                             ▼                              ▼
serialize_input()       Identity (그대로 전달)         ProgramInput → AppProgramInput
                                                      (Merkle proof만 추출)
                             │                              │
                             ▼                              ▼
SP1 Guest               execute_blocks()              execute_app_circuit()
                        (EVM 재실행)                   (DexCircuit 실행)
                             │                              │
                             ▼                              ▼
사이클                  65,360,896                     357,761 (182x 감소)
                             │                              │
                             ▼                              ▼
Output                  ProgramOutput                  ProgramOutput (동일 타입)
                             │                              │
                             ▼                              ▼
L1 commit               commitBatch(typeId=1)          commitBatch(typeId=2)
L1 verify               _getPublicInputsFromCommitment  동일 (표준 ProgramOutput)
VK 조회                 vk[hash][1][SP1]               vk[hash][2][SP1]
```

### 4.2 핵심 설계 결정

**시퀀서는 변경 없음**: 시퀀서는 항상 EVM으로 블록을 실행한다. ZK-DEX든 EVM L2든 시퀀서 쪽은 동일하다.
차이는 **프루버 내부**에서만 발생한다:

1. `serialize_input()` 단계에서 `ProgramInput`(전체 witness) → `AppProgramInput`(Merkle proof)으로 **경량화**
2. SP1 게스트 바이너리가 EVM 대신 **DexCircuit**을 실행
3. 출력은 **동일한 `ProgramOutput`** → L1 검증 로직 변경 불필요

이 설계 덕분에 **시퀀서, L1 Committer, L1 Proof Sender 코드 변경이 불필요**하다.

---

## 5. 구현 — 완료

### Phase 1: L1 컨트랙트 배포 파이프라인 수정 ✅

> **갭 G1, G2 해결** — ZK-DEX VK 등록 + GuestProgramRegistry 등록

**파일**: `cmd/ethrex/l2/deployer.rs`

#### 5.1 추가된 상수

```rust
const GUEST_PROGRAM_REGISTRY_REGISTER_OFFICIAL_SIGNATURE: &str =
    "registerOfficialProgram(string,string,address,uint8)";
const UPGRADE_VERIFICATION_KEY_SIGNATURE: &str =
    "upgradeVerificationKey(bytes32,uint8,uint8,bytes32)";
```

#### 5.2 추가된 CLI 옵션 (`DeployerOptions`)

```rust
#[arg(
    long = "register-guest-programs",
    value_delimiter = ',',
    value_name = "PROGRAM_IDS",
    env = "ETHREX_REGISTER_GUEST_PROGRAMS",
    help = "Guest programs to register on L1 (e.g., zk-dex,tokamon)."
)]
pub register_guest_programs: Vec<String>,

#[arg(
    long = "zk-dex-sp1-vk",
    value_name = "PATH",
    env = "ETHREX_ZK_DEX_SP1_VK",
    help = "Path to the ZK-DEX SP1 verification key. Defaults to build output path."
)]
pub zk_dex_sp1_vk_path: Option<String>,
```

#### 5.3 Guest Program 등록 로직 (`initialize_contracts()` 끝)

`opts.register_guest_programs`에 지정된 각 프로그램에 대해:

1. **GuestProgramRegistry에 등록** — `registerOfficialProgram(name, description, creator, typeId)`
2. **SP1 VK 등록** (SP1 활성화 시) — `upgradeVerificationKey(commitHash, programTypeId, verifierId, vk)`

```rust
for program_id in &opts.register_guest_programs {
    let program_type_id = resolve_deployer_program_type_id(program_id);
    if program_type_id <= 1 { continue; }  // skip unknown/default

    // 1. registerOfficialProgram
    let register_calldata = encode_calldata(
        GUEST_PROGRAM_REGISTRY_REGISTER_OFFICIAL_SIGNATURE,
        &[
            Value::String(program_id.clone()),
            Value::String(format!("{program_id} guest program")),
            Value::Address(deployer_address),
            Value::Uint(U256::from(program_type_id)),
        ],
    )?;
    // ... build_generic_tx + send_generic_transaction

    // 2. upgradeVerificationKey (if SP1)
    if opts.sp1 {
        let vk = get_vk_for_program(program_id, opts)?;
        let upgrade_vk_calldata = encode_calldata(
            UPGRADE_VERIFICATION_KEY_SIGNATURE,
            &[
                Value::FixedBytes(commit_hash.0.to_vec().into()),
                Value::Uint(U256::from(program_type_id)),
                Value::Uint(U256::from(SP1_VERIFIER_ID)),
                Value::FixedBytes(vk.to_vec().into()),
            ],
        )?;
        // ... build_generic_tx + send_generic_transaction
    }
}
```

#### 5.4 추가된 헬퍼 함수

```rust
/// Maps a guest program ID string to its on-chain programTypeId.
fn resolve_deployer_program_type_id(program_id: &str) -> u8 {
    match program_id {
        "evm-l2" => 1, "zk-dex" => 2, "tokamon" => 3, _ => 0,
    }
}

/// Reads the SP1 verification key for a guest program.
/// For "zk-dex": reads from --zk-dex-sp1-vk or default build output path
/// (crates/guest-program/bin/sp1-zk-dex/out/riscv32im-succinct-zkvm-vk-bn254).
fn get_vk_for_program(program_id: &str, opts: &DeployerOptions) -> Result<Bytes, DeployerError> {
    match program_id {
        "zk-dex" => { /* opts.zk_dex_sp1_vk_path || default path */ }
        _ => Ok(Bytes::new()),
    }
}
```

#### Phase 1 검증

```bash
# L1 배포 후 확인
cast call $ON_CHAIN_PROPOSER "verificationKeys(bytes32,uint8,uint8)" \
    $COMMIT_HASH 2 1  # commitHash, programTypeId=2, SP1=1
# → 0x... (non-zero VK)

cast call $GUEST_PROGRAM_REGISTRY "isProgramActive(uint8)" 2
# → true
```

빌드 검증: `cargo check --release --features l2,l2-sql,sp1` — ✅ 통과

---

### Phase 2: 빌드 및 Makefile ✅

> **갭 G3, G4 해결** — ZK-DEX ELF 빌드 + 편의 타겟

#### 5.5 Makefile 타겟 추가

**파일**: `crates/l2/Makefile`

```makefile
# ==============================================================================
# ZK-DEX E2E
# ==============================================================================

deploy-l1-sp1-zk-dex: ## 📜 Deploys L1 contracts with SP1 verifier + ZK-DEX program
	COMPILE_CONTRACTS=true \
	GUEST_PROGRAMS=evm-l2,zk-dex \
	cargo run --release --features l2,l2-sql,sp1 --manifest-path ../../Cargo.toml -- l2 deploy \
	--eth-rpc-url ${L1_RPC_URL} \
	--private-key ${L1_PRIVATE_KEY} \
	--sp1 true \
	--on-chain-proposer-owner ${L2_OWNER_ADDRESS} \
	--bridge-owner ${L2_OWNER_ADDRESS} \
	--bridge-owner-pk ${BRIDGE_OWNER_PRIVATE_KEY} \
	--deposit-rich \
	--private-keys-file-path ../../fixtures/keys/private_keys_l1.txt \
	--genesis-l1-path ${L1_GENESIS_FILE_PATH} \
	--genesis-l2-path ${L2_GENESIS_FILE_PATH} \
	--register-guest-programs zk-dex

init-l2-zk-dex: ## 🚀 Initializes L2 with ZK-DEX guest program
	export $(shell cat ../../cmd/.env | xargs); \
	GUEST_PROGRAMS=evm-l2,zk-dex \
	cargo run --release --features l2,l2-sql,sp1 --manifest-path ../../Cargo.toml -- \
	l2 \
	--proof-coordinator.guest-program-id zk-dex \
	--watcher.block-delay 0 \
	--network ${L2_GENESIS_FILE_PATH} \
	--http.port ${L2_PORT} \
	--http.addr ${L2_RPC_ADDRESS} \
	${ETHREX_NO_MONITOR:+--no-monitor}

init-prover-sp1-zk-dex: ## 🔐 Starts SP1 prover with ZK-DEX program
	GUEST_PROGRAMS=evm-l2,zk-dex \
	cargo run --release --features "l2,l2-sql,$(GPU?),sp1" --manifest-path ../../Cargo.toml -- \
	l2 prover \
	--proof-coordinators tcp://127.0.0.1:3900 \
	--backend sp1 \
	--programs-config programs-zk-dex.toml
```

#### 5.6 programs-zk-dex.toml

**파일**: `crates/l2/programs-zk-dex.toml` (신규 생성)

```toml
# Guest Program Registry Configuration — ZK-DEX E2E
default_program = "zk-dex"
enabled_programs = ["zk-dex"]
```

---

### Phase 3: E2E 테스트 실행 — 완료 (2026-02-26)

#### 6.1 실행 순서

```
Terminal 1: L1 Docker
─────────────────────
make init-l1-docker

Terminal 2: L1 컨트랙트 배포
─────────────────────
make deploy-l1-sp1-zk-dex

Terminal 3: L2 시퀀서 (ZK-DEX 모드)
─────────────────────
ETHREX_NO_MONITOR=true make init-l2-zk-dex

Terminal 4: SP1 프루버 (ZK-DEX)
─────────────────────
PROVER_CLIENT_TIMED=true make init-prover-sp1-zk-dex

Terminal 5: 트랜잭션 전송
─────────────────────
# DEX token transfer 트랜잭션 전송 (아래 6.2 참조)
```

#### 6.2 테스트 트랜잭션

ZK-DEX의 DexCircuit은 **token transfer**를 지원한다.
테스트 시나리오:

1. **ETH 전송** (가장 단순) — DexCircuit이 아닌 공통 로직에서 처리
2. **Token Transfer** — DexCircuit의 `classify_tx()` → `execute_operation()` 경로

ETH 전송 테스트 (기존 load-test 활용 가능):
```bash
LOAD_TEST_TX_AMOUNT=10 make load-test
```

Token Transfer 테스트 (커스텀 스크립트 필요):
```bash
# DEX 컨트랙트에 token transfer 호출
# function selector: transfer(address,address,uint256)
cast send $DEX_CONTRACT "transfer(address,address,uint256)" \
    $TOKEN_ADDRESS $RECIPIENT $AMOUNT \
    --rpc-url http://localhost:1729 \
    --private-key $SENDER_PK
```

#### 6.3 검증 포인트

| # | 검증 항목 | 확인 방법 |
|---|----------|----------|
| 1 | L2 블록 생성 | `curl localhost:1729 -d '{"method":"eth_blockNumber",...}'` |
| 2 | 배치 생성 | 시퀀서 로그: `"Batch N finalized"` |
| 3 | 프루버가 배치 수신 | 프루버 로그: `"Received batch N with program_id zk-dex"` |
| 4 | serialize_input 성공 | 프루버 로그: 에러 없이 진행 |
| 5 | SP1 증명 생성 | 프루버 로그: `"proving_time_ms"` |
| 6 | L1 commit 성공 | `cast call $OCP "lastCommittedBatch()" --rpc-url $L1_RPC` |
| 7 | L1 verify 성공 | `cast call $OCP "lastVerifiedBatch()" --rpc-url $L1_RPC` |
| 8 | 사이클 수 확인 | 프루버 로그: `cycle-tracker` 출력 (목표: ~357K) |

#### 6.4 예상 실행 시간

| 단계 | 예상 시간 |
|------|----------|
| L1 Docker 기동 | ~10초 |
| 컨트랙트 배포 | ~30초 |
| L2 시퀀서 빌드 + 기동 | ~3-5분 (최초 빌드) |
| 프루버 빌드 + 기동 | ~5-10분 (SP1 ELF 컴파일 포함) |
| 트랜잭션 전송 + 배치 생성 | ~1분 |
| SP1 증명 생성 | ~3-4분 (CPU, crypto precompile 패치 적용) |
| L1 commit + verify | ~30초 |
| **총 E2E** | **~15-20분** (최초), **~5-10분** (재실행) |

---

## 6. 전체 파일 변경 요약

### 수정 (EDIT) — 완료

| # | 파일 | 변경 내용 | Phase |
|---|------|-----------|-------|
| 1 | `cmd/ethrex/l2/deployer.rs` | CLI 옵션 2개 + 상수 2개 + 등록 로직 ~100줄 + 헬퍼 함수 2개 | 1 |
| 2 | `crates/l2/Makefile` | ZK-DEX 전용 타겟 3개 추가 | 2 |

### 생성 (CREATE) — 완료

| # | 파일 | 설명 | Phase |
|---|------|------|-------|
| 1 | `crates/l2/programs-zk-dex.toml` | ZK-DEX 프루버 설정 | 2 |

### 변경 불필요 (표준 ProgramOutput 덕분)

| 파일 | 이유 |
|------|------|
| `crates/l2/sequencer/l1_committer.rs` | `publicValuesHash`는 `H256::zero()` 유지 (표준 포맷) |
| `crates/l2/sequencer/l1_proof_sender.rs` | `customPublicValues`는 빈 바이트 유지 (표준 포맷) |
| `crates/l2/common/src/prover.rs` | 변경 없음 |
| `crates/l2/contracts/src/l1/OnChainProposer.sol` | `_getPublicInputsFromCommitment()` 그대로 사용 |

---

## 7. 진행 상태

```
Phase 1 (L1 배포 파이프라인)     ✅ 완료
   ↓
Phase 2 (빌드 + Makefile)       ✅ 완료
   ↓
Phase 3 (E2E 테스트)            ✅ 완료 — Batch 1~9 SP1 Groth16 증명 + L1 온체인 검증 성공
   ↓
Phase 4 (출금 UX + Docker 인프라) ✅ 완료 — Early batch commit + Withdrawal Claim UI
```

빌드 검증: `cargo build --release -p ethrex` — ✅ 통과 (2026-02-27)

총 수정 파일 2개 + 생성 1개. 구현량이 매우 적다.

---

## 8. 출금 시 즉시 배치 커밋 (Early Batch Commit)

### 8.1 배경

기본 L1 committer는 `commit_time_ms` 주기(예: 5분)로만 배치를 커밋한다.
사용자가 L2에서 출금(withdraw) 요청을 보내면 다음 주기까지 최대 5분을 기다려야 한다.
출금 요청이 감지되면 즉시 배치를 커밋하여 출금 지연을 줄인다.

### 8.2 설계 결정: 조건부 즉시 커밋

무조건 즉시 커밋하면 프루버가 바쁜 상황에서 배치만 쌓이고 실질적 이득이 없다.
따라서 **두 가지 조건을 모두 만족할 때만** 즉시 커밋한다:

1. **출금 TX 존재**: 미커밋 블록 중 `BRIDGE_ADDRESS`(0x...ffff) 대상 TX가 있음
2. **증명 대기 배치 없음**: L1의 `lastCommittedBatch ≤ lastVerifiedBatch`

```
60초마다 handle_commit_message() 실행
    ├── 5분 타이머 만료? → 커밋 (기존 동작)
    ├── 타이머 미만료 + 출금 TX 있음 + committed ≤ verified
    │   → 즉시 커밋 (프루버가 유휴 상태)
    ├── 타이머 미만료 + 출금 TX 있음 + committed > verified
    │   → 스킵 (프루버가 바빠서 새 배치 의미 없음, 타이머 대기)
    └── 타이머 미만료 + 출금 없음 → 스킵
```

### 8.3 구현

**파일**: `crates/l2/sequencer/l1_committer.rs`

#### `has_pending_withdrawals()` 메서드

```rust
async fn has_pending_withdrawals(&self) -> Result<bool, CommitterError> {
    // 마지막 커밋된 배치의 마지막 블록 번호
    let last_committed_block = self.rollup_store
        .get_block_numbers_by_batch(self.last_committed_batch)
        .await?
        .and_then(|blocks| blocks.last().copied())
        .unwrap_or(0);

    let latest_block = self.store.get_latest_block_number().await?;

    // 미커밋 블록 순회하며 BRIDGE_ADDRESS 대상 TX 확인
    for block_num in (last_committed_block + 1)..=latest_block {
        if let Some(body) = self.store.get_block_body(block_num).await? {
            for tx in &body.transactions {
                if tx.to() == TxKind::Call(BRIDGE_ADDRESS) {
                    return Ok(true);
                }
            }
        }
    }
    Ok(false)
}
```

#### `handle_commit_message()` 수정

```rust
let timer_expired = current_time - self.last_committed_batch_timestamp > commit_time;

let early_commit = if !timer_expired {
    let has_withdrawal = self.has_pending_withdrawals().await.unwrap_or(false);
    if has_withdrawal {
        let last_verified = get_last_verified_batch(
            &self.eth_client, self.on_chain_proposer_address,
        ).await.unwrap_or(0);
        let no_pending_proof = current_last_committed_batch <= last_verified;
        // 프루버 유휴 상태에서만 즉시 커밋
        no_pending_proof
    } else {
        false
    }
} else {
    false
};

let should_send_commitment = timer_expired || early_commit;
```

### 8.4 증명 상태 판단

L1 OnChainProposer 컨트랙트의 두 카운터를 비교:

| 상태 | committed | verified | 의미 |
|------|-----------|----------|------|
| 프루버 유휴 | 5 | 5 | 모든 배치 증명 완료 → 즉시 커밋 OK |
| 프루버 바쁨 | 5 | 3 | 배치 4,5 증명 대기/진행 중 → 즉시 커밋 불필요 |

### 8.5 검증 방법

1. `cargo build -p ethrex` 컴파일 확인
2. Localnet 시작 → L1 deposit → L2 withdraw TX 전송
3. 프루버가 유휴일 때: 로그 `"Pending withdrawal detected, triggering early batch commit"` 확인
4. 프루버가 바쁠 때: 로그 `"prover is busy, waiting for timer"` 확인
5. 출금 없는 일반 상황에서 기존 5분 주기 커밋 정상 동작 확인

---

## 9. 리스크 및 대응 (기존 §8)

| 리스크 | 영향 | 대응 |
|--------|------|------|
| serialize_input()에서 trie 재구축 실패 | 증명 불가 | 유닛 테스트로 사전 검증, 실제 L2 상태로 테스트 |
| AppProgramInput의 Merkle proof 불완전 | state root 불일치 | 시퀀서 EVM 실행 결과와 서킷 결과 비교 테스트 |
| SP1 ELF 컴파일 시간 | 빌드 지연 | ELF 캐싱, GUEST_PROGRAMS 분리 빌드 |
| VK 불일치 (빌드 버전 차이) | L1 검증 실패 | 동일 빌드에서 ELF/VK 생성 보장 |
| `_getPublicInputsFromCommitment()` 인코딩 불일치 | L1 검증 실패 | SP1 게스트의 `ProgramOutput.encode()`와 L1 재구성 로직 일치 확인 테스트 |

---

## 10. 향후 확장

이 E2E 파이프라인이 성공하면:

1. **대규모 배치 벤치마크** — 10/100/1000 transfers로 확장
2. **Native ARM 벤치마크** — Rosetta 2 없이 직접 실행
3. **GPU 가속** — `GPU=true` 모드 테스트
4. **Tokamon 서킷** — 동일 파이프라인으로 tokamon 서킷 E2E
5. **멀티 프로그램 동시 운영** — EVM-L2와 ZK-DEX를 같은 L1에서 동시 운영
6. **비표준 ProgramOutput 프로그램** — `customPublicValues` 경로 활용 (필요 시)

---

## 참조 문서

| 문서 | 설명 |
|------|------|
| `tokamak-notes/zk-dex-progress.md` | 전체 프로젝트 진행현황 |
| `tokamak-notes/guest-program-modularization/12-app-specific-circuit-dev-plan.md` | 앱 서킷 개발 계획 |
| `tokamak-notes/local-setup-guide.md` | 로컬 실행 가이드 |
| `tokamak-notes/sp1-zk-dex-vs-baseline.md` | ZK-DEX vs EVM L2 상세 비교 |
| `crates/l2/contracts/src/l1/OnChainProposer.sol` | L1 검증 컨트랙트 |
| `crates/l2/contracts/src/l1/GuestProgramRegistry.sol` | 프로그램 레지스트리 컨트랙트 |
| `crates/guest-program/src/l2/output.rs` | ProgramOutput 표준 정의 |
