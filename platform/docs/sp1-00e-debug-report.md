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

## 4. 미확인 항목 (조사 필요)

### 4.1 publicInputs 실제 값 비교 ⚠️ (최우선)

**문제**: on-chain에서 `_getPublicInputsFromCommitment(4)`가 생성하는 바이트와, guest program이 `ProgramOutput.encode()`로 커밋한 바이트가 byte-exact로 동일한지 확인 필요.

특히 `balance_diffs` 부분:
- Committer가 `commitBatch()`에 보내는 balance_diffs
- Guest program이 `ProgramOutput`에 포함하는 balance_diffs
- 이 두 값이 정확히 같아야 함

**확인 방법**:
1. proof sender에 디버그 로그 추가하여 실제 전송 데이터 출력
2. 또는 L1에서 `debug_traceCall`로 `verifyBatch` 호출 시 입력값 추출
3. 또는 committer에서 batch 4의 `balance_diffs` 로그 추가

### 4.2 SP1Verifier 컨트랙트 버전 호환성

- 배포된 SP1Verifier 컨트랙트 bytecode 크기: ~6000 bytes
- Prover SDK: `sp1-sdk = "=5.0.8"`
- 컨트랙트가 SDK v5.0.x의 Groth16 proof format과 호환되는지 확인 필요
- Groth16 circuit artifacts: `/Users/zena/.sp1/circuits/groth16/v5.0.0`

**확인 방법**:
1. SP1Verifier 컨트랙트의 `VERIFIER_HASH()` 호출하여 버전 확인
2. sp1-contracts 레포에서 해당 해시가 v5.0.x에 해당하는지 대조

### 4.3 l2_in_message_rolling_hashes 인코딩 차이 (알려진 버그)

- **Rust**: `chain_id.to_be_bytes()` → **8 bytes** (u64)
- **Solidity**: `bytes32(rh.chainId)` → **32 bytes** (uint256)
- 현재 zk-dex 배치에 L2-to-L2 메시지가 없으면 영향 없음
- **파일**: `crates/guest-program/src/l2/output.rs:62-64`

---

## 5. 배포 환경 정보

```
Deploy ID:    ac0f344c-73d8-458a-9220-04208ee3c3f8
Project:      tokamak-ac0f344c
Docker Image: ethrex:zk-dex-tokamak-ac0f344c
L1 Port:      8546
L2 Port:      1730
Chain ID:     65536999
Bridge:       0xfecf35cab60cca3306ebd927d1673f545b26cfc8
Proposer:     0x6017eaa0011c4ac0bb38bc19782960eb8eab6b3d
SP1Verifier:  0x4971f10184e2da1cc4a325a9ab2e2741da7f7743
```

---

## 6. 다음 단계

1. **publicInputs 비교** (4.1) — proof sender에 `publicInputs` 로그 추가 또는 `debug_traceCall` 사용
2. **SP1Verifier 버전 확인** (4.2) — `VERIFIER_HASH()` 호출
3. **balance_diffs 디버깅** — committer가 commitBatch에 보내는 balance_diffs와 guest program output의 balance_diffs byte-level 비교
