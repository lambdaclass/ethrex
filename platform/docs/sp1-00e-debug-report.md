# SP1 Proof Verification 00e 에러 분석 보고서

> **날짜**: 2026-03-05
> **브랜치**: `feat/l2-deployment-engine`
> **증상**: Deployment engine으로 생성한 zk-dex 배포에서 batch 4부터 L1 proof verification 실패 (error code "00e")
> **참고**: `make zk-dex-docker` (원본 compose)에서는 정상 동작했음

---

## 1. 00e 에러란?

`OnChainProposer.sol` line 547-559에서 SP1 proof verification이 실패할 때 발생:

```solidity
if (REQUIRE_SP1_PROOF) {
    bytes32 sp1Vk = verificationKeys[batchCommitHash][batchProgramTypeId][SP1_VERIFIER_ID];
    try
        ISP1Verifier(SP1_VERIFIER_ADDRESS).verifyProof(sp1Vk, publicInputs, sp1ProofBytes)
    {} catch {
        revert("00e"); // Invalid SP1 proof failed proof verification
    }
}
```

3개 입력 중 하나라도 불일치하면 실패:
- **sp1Vk** — on-chain에 등록된 verification key
- **publicInputs** — on-chain에서 `_getPublicInputsFromCommitment()`로 재구성
- **sp1ProofBytes** — prover가 생성한 Groth16 proof bytes

---

## 2. compose-generator.js 수정 사항 (완료)

### 수정 1: ETHREX_L2_SP1 중복 제거
- **원인**: base template에 `${ETHREX_L2_SP1:-false}` + `deployerExtraEnv`에 `ETHREX_L2_SP1=true` → 중복
- **수정**: base template에서 `${profile.sp1Enabled}`으로 직접 설정, deployerExtraEnv에서 제거

### 수정 2: ETHREX_OSAKA_ACTIVATION_TIME 추가
- **원인**: 원본 compose에는 `ETHREX_OSAKA_ACTIVATION_TIME=${ETHREX_OSAKA_ACTIVATION_TIME:-1761677592}` 있지만 생성된 compose에 누락
- **영향**: `None` → `Fork::Osaka`, 설정 시 → `Fork::Prague` (blob sidecar wrapper_version 차이)
- **수정**: L2 서비스 환경변수에 추가

### 수정 3: Genesis 볼륨 중복 마운트 제거
- **원인**: base template과 `l2ExtraVolumes`에서 동일한 genesis 파일 마운트
- **수정**: `l2ExtraVolumes`에서 genesis 마운트 제거

**결과**: 이 3개 수정 후에도 00e 에러 지속.

---

## 3. 검증 완료 항목 (문제 아님)

### 3.1 commitHash 일치
- Deployer, L2 committer, proof sender 모두 같은 Docker 이미지 사용
- `VERGEN_GIT_SHA=docker-build` (Dockerfile.sp1 line 83에서 설정)
- `commitHash = keccak("docker-build")` = `0xb04b4c087c284aa093dfd1162edb2da418854845c67ee97a842134c34bfcb36f`
- **파일**: `crates/l2/sequencer/utils.rs:192` — `env!("VERGEN_GIT_SHA")`

### 3.2 programTypeId 일치
- Committer: `ETHREX_GUEST_PROGRAM_ID=zk-dex` → `resolve_program_type_id("zk-dex")` = 2
- On-chain batch 4 commitment: `programTypeId = 2` 확인
- **파일**: `crates/l2/sequencer/l1_committer.rs:1323`

### 3.3 VK 등록 경로
Deployer 로그에서 확인:
1. `initialize()` → `/usr/local/bin/riscv32im-succinct-zkvm-vk-bn254` (host mount, evm-l2 VK) → `verificationKeys[commitHash][1][SP1]`
2. `register_guest_programs("zk-dex")` → Docker 내부 `/ethrex/crates/guest-program/bin/sp1-zk-dex/out/riscv32im-succinct-zkvm-vk-bn254` → `verificationKeys[commitHash][2][SP1]`

zk-dex는 `programTypeId=2`를 사용하므로, `initialize()`의 evm-l2 VK는 영향 없음.

VK 등록 트랜잭션 (`0x6d3862d9...`) calldata 확인:
```
commitHash:    b04b4c08... ✓
programTypeId: 2 ✓
verifierId:    1 (SP1) ✓
vk:            00c40e105259b564873710f4a0369401b02b5cd9e5ecbc42fb63f427df67fdd8
```
이 VK는 Docker 이미지 내부 zk-dex VK와 일치 (`0x00c40e10...`).

### 3.4 Docker 이미지 일관성
- L1: `ethrex:l1-tokamak-ac0f344c` (별도)
- Deployer, L2, Prover: 모두 `ethrex:zk-dex-tokamak-ac0f344c` (동일 이미지)
- ELF: `include_bytes!("../bin/sp1-zk-dex/out/riscv32im-succinct-zkvm-elf")` — 컴파일 시점에 포함
- **파일**: `crates/guest-program/src/lib.rs:68-69`

### 3.5 Host VK vs Docker VK 차이 (참고)
Docker 빌드 시 guest program이 재컴파일되어 새 VK 생성됨:

| VK | Host 디스크 | Docker 이미지 내부 |
|---|---|---|
| evm-l2 | `0x0016a778...` | `0x001ac09a...` |
| zk-dex | `0x0072d829...` | `0x00c40e10...` |

Host VK는 deployer의 `initialize()`에서 `programTypeId=1`에 등록되지만, zk-dex는 `programTypeId=2`를 사용하므로 영향 없음.

### 3.6 ProgramOutput 인코딩 순서
**Rust** (`crates/guest-program/src/l2/output.rs:32-68`):
```
initial_state_hash | final_state_hash | l1_out_messages_merkle_root |
l1_in_messages_rolling_hash | blob_versioned_hash | last_block_hash |
chain_id (U256) | non_privileged_count (U256) |
[balance_diffs...] | [l2_in_message_rolling_hashes...]
```

**Solidity** (`OnChainProposer.sol:795-848`):
```
initialStateRoot | newStateRoot | withdrawalsLogsMerkleRoot |
processedPrivilegedTransactionsRollingHash | blobKZGVersionedHash |
lastBlockHash | CHAIN_ID (bytes32) | nonPrivilegedTransactions (bytes32) |
[balanceDiffs...] | [l2InMessageRollingHashes...]
```

필드 순서 동일 ✓

### 3.7 Proof 생성 성공
```
Proved batch 4 in 497.19s (program: zk-dex, elf)
wrap_groth16_bn254: Running verify in docker — 성공
```
Prover 자체 검증(verify in docker)도 통과 ✓

### 3.8 balance_diffs scaling
- zk-dex genesis에 `native_token_l1_address` 없음
- `native_token_scale_factor = None`
- guest program과 committer 모두 `None`으로 `get_balance_diffs()` 호출
- **파일**: `crates/l2/common/src/messages.rs`

---

## 4. 근본 원인 (해결됨)

### 4.1 WITHDRAWAL_GAS 상수 불일치 ✅

**근본 원인**: Guest program(서킷)의 `WITHDRAWAL_GAS` 상수가 실제 EVM gas와 불일치.

| 구분 | 값 |
|------|-----|
| Guest program `WITHDRAWAL_GAS` (수정 전) | **100,000** |
| 실제 EVM `CommonBridgeL2.withdraw()` gas | **95,002** |

zk-dex 앱체인은 특정 함수만 실행하므로 **고정 gas 상수**로 계산하는 설계.
서킷이 100,000으로 gas fee를 분배하면 sender/coinbase/vault 잔액이 EVM과 달라지고,
결과적으로 `balance_diffs` → `publicInputs` 불일치 → **00e 에러**.

**수정**: `WITHDRAWAL_GAS = 100_000` → `95_002`
- **파일**: `crates/guest-program/src/common/handlers/constants.rs:67`
- **커밋**: `fix(guest-program): set WITHDRAWAL_GAS to 95,002 to match actual EVM gas`

### 4.2 잘못된 수정 시도 (되돌림)

`app_execution.rs`에서 고정 gas 상수 대신 block header의 실제 gas를 사용하도록
변경했으나, 이는 zk-dex의 설계 의도에 맞지 않아 되돌림.

앱체인별로 서킷/프루버/검증 컨트랙트가 일관된 gas 계산을 사용해야 하며,
zk-dex는 고정 gas 상수가 올바른 접근.

### 4.3 SP1Verifier 컨트랙트 버전 호환성 (미확인)

- 배포된 SP1Verifier 컨트랙트 bytecode 크기: ~6000 bytes
- Prover SDK: `sp1-sdk = "=5.0.8"`
- WITHDRAWAL_GAS 수정으로 00e가 해결되면 이 항목은 무관

### 4.4 l2_in_message_rolling_hashes 인코딩 차이 (알려진 버그, 미수정)

- **Rust**: `chain_id.to_be_bytes()` → **8 bytes** (u64)
- **Solidity**: `bytes32(rh.chainId)` → **32 bytes** (uint256)
- 현재 zk-dex 배치에 L2-to-L2 메시지가 없으면 영향 없음
- **파일**: `crates/guest-program/src/l2/output.rs:62-64`

---

## 5. 디버그 로깅 (추가됨)

근본 원인 조사를 위해 다음 파일에 `[DEBUG-00e]` 로그 추가:

| 파일 | 내용 |
|------|------|
| `crates/l2/prover/src/backend/sp1.rs` | SP1 public values (field-by-field + sha256) |
| `crates/l2/sequencer/l1_committer.rs` | commitBatch calldata 필드 |
| `crates/l2/sequencer/l1_proof_sender.rs` | rollup store 데이터 (prev/cur state, merkle root 등) |

이 로그는 00e 수정 확인 후 제거 예정.

---

## 6. 배포 환경 정보

### 이전 배포 (00e 발생)
```
Deploy ID:    ac0f344c-73d8-458a-9220-04208ee3c3f8
Project:      tokamak-ac0f344c
Docker Image: ethrex:zk-dex-tokamak-ac0f344c
L1 Port:      8546
L2 Port:      1730
```

### 검증 배포 (WITHDRAWAL_GAS 수정 포함)
```
Deploy ID:    781e135b-f9dc-4d06-a73c-6d6c53e263a4
Project:      tokamak-781e135b
L1 Port:      8547
L2 Port:      1731
Status:       빌드 중 (e2e 테스트 예정)
```

---

## 7. E2E 테스트

`platform/tests/e2e-bridge.js` 추가:
- L1/L2 health check
- L1 balance 확인
- Bridge deposit (L1→L2 ETH 전송)
- L2 block 진행 확인

```bash
node platform/tests/e2e-bridge.js --l1-port 8547 --l2-port 1731 --bridge <address>
```

---

## 8. 다음 단계

1. ~~publicInputs 비교~~ → **WITHDRAWAL_GAS 수정으로 해결** (검증 배포로 확인 예정)
2. 디버그 로그 제거 (00e 해결 확인 후)
3. E2E 테스트 실행 및 결과 검증
4. 다른 gas 상수 (`ETH_TRANSFER_GAS=21,000`, `SYSTEM_CALL_GAS=50,000`) 실제 EVM gas와 일치 여부 확인
