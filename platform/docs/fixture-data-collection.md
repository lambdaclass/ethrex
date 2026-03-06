# Fixture Data Collection Guide

## Overview

Offline testing (without Docker) requires real data captured from actual deployments.
This document describes how to collect fixture data from a running deployment
and convert it into test-ready JSON fixtures.

## Architecture

```
┌──────────────────┐     ┌──────────────────┐
│   L2 Container   │     │ Prover Container │
│   (committer)    │     │                  │
│                  │     │                  │
│  ETHREX_DUMP_    │     │  ETHREX_DUMP_    │
│  FIXTURES=       │     │  FIXTURES=       │
│  /tmp/fixtures   │     │  /tmp/fixtures   │
│        │         │     │        │         │
│        ▼         │     │        ▼         │
│  committer.json  │     │   prover.json    │
└────────┬─────────┘     └────────┬─────────┘
         │   volume mount           │   volume mount
         ▼                          ▼
    /tmp/fixtures/zk-dex/batch_N/committer.json
    /tmp/fixtures/zk-dex/batch_N/prover.json
         │
         ▼  merge-fixtures.sh
    /tmp/fixtures/zk-dex/batch_N/fixture.json
         │
         ▼  cp to repo
    crates/guest-program/tests/fixtures/zk-dex/batch_N.json
```

## Step 1: Enable Fixture Dumping

Add `ETHREX_DUMP_FIXTURES=/tmp/fixtures` environment variable to both the L2 (committer)
and prover containers in docker-compose.yaml.

### L2 Container (committer)
```yaml
tokamak-app-l2:
  environment:
    - ETHREX_DUMP_FIXTURES=/tmp/fixtures
  volumes:
    - /tmp/fixtures:/tmp/fixtures   # mount host directory
```

### Prover Container
```yaml
tokamak-app-prover:
  environment:
    - ETHREX_DUMP_FIXTURES=/tmp/fixtures
  volumes:
    # prover already has /tmp:/tmp, so /tmp/fixtures maps automatically
    - /tmp:/tmp
```

## Step 2: Rebuild and Restart

The Docker image must include the ETHREX_DUMP_FIXTURES code
(committed in `578b2e125`).

```bash
# Stop L2 + prover (keep L1 + contracts)
docker compose -f <compose-file> -p <project> stop tokamak-app-prover tokamak-app-l2

# Rebuild with latest code
docker compose -f <compose-file> -p <project> build tokamak-app-l2

# Restart
docker compose -f <compose-file> -p <project> up -d tokamak-app-l2 tokamak-app-prover
```

**Warning**: `docker compose up` may recreate the deployer container, which re-deploys
contracts with **new addresses**. If this happens, the bridge/proposer addresses change.
Check `/env/.env` in the L2 container for the current addresses.

## Step 3: Generate Transactions

Run the E2E test to create deposit and withdrawal transactions:

```bash
# Must pass correct bridge/proposer addresses
node platform/tests/e2e-bridge.js \
  --l1-port 8547 --l2-port 1731 \
  --bridge <BRIDGE_ADDRESS> \
  --proposer <PROPOSER_ADDRESS>
```

To find the current addresses:
```bash
docker exec <l2-container> cat /env/.env
```

Or manually:
1. **Deposit**: Send ETH to the L1 bridge contract
2. **Wait**: For the committer to create a batch with the deposit
3. **Withdraw**: Send a withdrawal transaction on L2
4. **Wait**: For the committer + prover to process the withdrawal batch

## Step 4: Collect Fixture Files

After batches are committed and proved, check the output:

```bash
ls -la /tmp/fixtures/zk-dex/batch_2/
# committer.json   — committer calldata fields
# prover.json      — prover public values (field-by-field + encoded hex)
# stdin.bin        — serialized input for offline re-proving
# proof.bin        — serialized BatchProof for offline verification
```

Note: Empty/genesis batches (1-2) only have committer.json because the prover
skips them. Only batches with actual transactions will have prover.json + binaries.

### Binary Fixture Files

| File | Contents | Used For |
|------|----------|----------|
| `stdin.bin` | Serialized guest program input | Offline re-proving (Phase 3) |
| `proof.bin` | Serialized `BatchProof` (bincode) | Offline verification (Phase 4) |

These files enable re-proving and verification without Docker:
```rust
// Re-prove (requires SP1 SDK, ~10 min):
let stdin_bytes = std::fs::read("stdin.bin").unwrap();
let mut stdin = SP1Stdin::new();
stdin.write_slice(&stdin_bytes);
let proof = client.prove(&pk, &stdin, SP1ProofMode::Compressed).unwrap();

// Verify (requires SP1 SDK, ~seconds):
let batch_proof: BatchProof = bincode::deserialize(&std::fs::read("proof.bin").unwrap()).unwrap();
```

## Step 5: Merge and Copy

Use the merge script to combine committer + prover data:

```bash
cd crates/guest-program/tests

# Merge each batch (requires both committer.json + prover.json)
./merge-fixtures.sh /tmp/fixtures/zk-dex/batch_5

# Copy to test fixtures directory with descriptive name
cp /tmp/fixtures/zk-dex/batch_5/fixture.json fixtures/zk-dex/batch_5_deposit.json
```

## Step 6: Verify

```bash
cargo test -p ethrex-guest-program --test test_program_output
cargo test -p ethrex-guest-program --test test_commitment_match
cargo test -p ethrex-guest-program --test test_state_continuity
```

## Fixture JSON Schema

```json
{
  "app": "zk-dex",
  "batch_number": 11,
  "program_type_id": 2,
  "chain_id": 65536999,
  "description": "Batch with deposit transaction",
  "prover": {
    "initial_state_hash": "0x...",
    "final_state_hash": "0x...",
    "l1_out_messages_merkle_root": "0x...",
    "l1_in_messages_rolling_hash": "0x...",
    "blob_versioned_hash": "0x...",
    "last_block_hash": "0x...",
    "non_privileged_count": 1,
    "balance_diffs": [],
    "l2_in_message_rolling_hashes": [],
    "encoded_public_values": "0x...",
    "sha256_public_values": "0x..."
  },
  "committer": {
    "new_state_root": "0x...",
    "withdrawals_merkle_root": "0x...",
    "priv_tx_rolling_hash": "0x...",
    "non_privileged_txs": 1,
    "balance_diffs": [],
    "l2_in_message_rolling_hashes": []
  }
}
```

## What Each Test Verifies

| Test | Purpose |
|------|---------|
| `test_program_output` | ProgramOutput.encode() matches prover's encoded_public_values byte-for-byte |
| `test_commitment_match` | Committer calldata fields match prover public values (prevents 00e errors) |
| `test_state_continuity` | Batch N final_state_hash == Batch N+1 initial_state_hash |

## Progress Log

### 2026-03-05: First Collection Attempt

**Deployment**: `tokamak-781e135b` (zk-dex on `feat/l2-deployment-engine` branch)
- L1: http://127.0.0.1:8547
- L2: http://127.0.0.1:1731
- Bridge: `0x7d2712608584caf0c1cee95e69709d5f76f0884f`
- Proposer: `0x21c0e9d9cc08d6d299952940e058c252d0201ac7`

**완료된 작업**:
1. docker-compose.yaml에 `ETHREX_DUMP_FIXTURES=/tmp/fixtures` 환경변수 + 볼륨 마운트 추가
2. Docker 이미지 재빌드 (~8분, SP1 빌드 + 레이어 캐싱)
3. L2 + 프루버 재시작 (L1은 유지)
4. 두 컨테이너 모두 환경변수 확인됨
5. Committer fixture dump 정상 작동 확인: batch 1-6에 `committer.json` 저장됨
6. E2E 테스트 실행: deposit(1 ETH) + withdrawal(0.5 ETH) 성공
7. Batch 5 SP1 proof 완료 (188초): `prover.json` 저장됨

**수집된 데이터** (`/tmp/fixtures/zk-dex/`):
- `batch_1/committer.json` — 빈 배치
- `batch_2/committer.json` — 빈 배치
- `batch_3/committer.json` — 빈 배치
- `batch_4/committer.json` — 빈 배치
- `batch_5/committer.json` + `prover.json` — 빈 배치 (첫 번째 완전 proof)
- `batch_6/committer.json` — deposit 포함 배치

**발견된 버그 (수정 완료)**:

프루버의 fixture dump에서 **Groth16 proof bytes를 public_values로 잘못 사용**하는 버그 발견.

- **원인**: `ProofCalldata`에 `public_values` 필드가 없어서, fixture dump 코드가
  `calldata[0]` (Groth16 proof bytes)를 public_values로 착각하여 추출
- **증상**: prover.json의 `initial_state_hash` 등 모든 필드가 실제 값과 불일치
  (DEBUG-00e 로그와 비교하여 발견)
- **수정**: `ProofCalldata` 구조체에 `public_values: Vec<u8>` 필드 추가,
  SP1 `to_calldata()`에서 올바른 public_values 저장
- **수정 파일**:
  - `crates/l2/common/src/prover.rs` — `ProofCalldata`에 `public_values` 필드 추가
  - `crates/l2/prover/src/backend/sp1.rs` — `to_calldata()`에서 public_values 추출
  - `crates/l2/prover/src/prover.rs` — fixture dump가 `pc.public_values` 사용
  - `crates/l2/prover/src/backend/risc0.rs`, `exec.rs`, `tee/quote-gen/src/main.rs` — 빈 public_values 추가
- **상태**: 코드 수정 완료, 컴파일 확인. Docker 이미지 재빌드 + 재수집 필요.

**기존에 발생한 문제들**:
- Docker compose가 deployer를 재생성하여 컨트랙트 주소가 변경됨
  → 첫 번째 E2E 테스트가 이전 bridge 주소로 deposit 전송
- macOS 포트 고갈 (`EADDRNOTAVAIL`) — Docker 빌드 + 많은 연결로 인해 발생

## 2026-03-05 TODO (완료됨)

1단계~2.5단계 모두 완료. 상세 내용은 아래 "2026-03-06" 섹션 참조.

## 2026-03-06: Fixture Dump 버그 수정 배포 + 새 00e 버그 발견

### 완료된 작업

1. **Docker 이미지 재빌드** — `ProofCalldata.public_values` 수정 반영 (레이어 캐시로 빠르게 완료)
2. **전체 재배포** — L1/deployer/L2/prover 모두 새로 시작
   - 새 bridge: `0x3fd626312f3cda9be2c23fb27ecf55a563d471f2`
   - 새 proposer: `0x91e057067c11073e799374f163adf0e15eafcd18`
3. **1차 E2E 테스트** — deposit(1 ETH) + withdrawal(0.5 ETH) 성공, 12/12 통과
4. **Batch 2 fixture 자동 수집 성공** — committer.json + prover.json 모두 정상
   - Deposit + withdrawal이 batch_2에 함께 포함됨
   - 빈 배치(1, 3~)는 proof-free 검증되어 prover.json 없음
5. **merge-fixtures.sh** 정상 동작 확인
6. **기존 수동 fixture(batch_8/11/12) 제거** — 이전 배포 데이터로 state 불일치
7. **자동 수집 fixture(batch_2)로 테스트 교체** — 개별 테스트 삭제, `_all` 패턴으로 통합
8. **오프라인 테스트 5/5 통과**: program_output, commitment_match, state_continuity(skip), chain_id, program_type_id

### 새로 발견된 버그: 두 번째 실행 시 00e (state_root 불일치)

**재현 방법**: 같은 배포에서 E2E 테스트를 2번 실행

**증상**: batch 16 (두 번째 deposit+withdrawal) proof verification 실패
```
L1 Proof Sender: Failed because of an EthClient error:
  eth_estimateGas: execution reverted: 00e
```

**데이터 비교**:
| 필드 | Batch 2 (1차, 성공) | Batch 16 (2차, 실패) |
|------|:---:|:---:|
| state_root | committer = prover | committer != prover |
| merkle_root | match | match |
| rolling_hash | match | match |
| last_block_hash | match | match |
| non_priv_count | match | match |

- Committer state_root: `0x995f08af...`
- Prover state_root: `0xd0640d12...`
- 두 값 모두 initial_state = `0xb652083c...` (batch 2 final)에서 시작

**가능한 원인**:
1. **EVM warm/cold storage 가스 차이** — 첫 실행 시 모든 storage slot이 cold (가스 높음),
   두 번째 실행 시 일부 slot이 warm (가스 낮음). 고정 가스 상수는 cold 케이스에만 맞음.
2. **계정 존재 여부에 따른 가스 차이** — 새 계정 생성 vs 기존 계정 업데이트의 가스 비용 차이
3. **nonce 증가에 따른 저장소 레이아웃 차이** — 두 번째 tx의 nonce가 다르므로 가스 비용 차이

**영향**: zk-dex 앱에서 동일 계정으로 반복 트랜잭션 시 proof verification 실패
**심각도**: 높음 — production에서 재사용 시나리오 차단

**디버그 데이터**: `/tmp/fixtures/zk-dex/batch_16/` (committer.json + prover.json)

### 00e 버그 #2 해결 (2026-03-06)

**원인 확인**: `eth_getTransactionReceipt`로 비교
- 1st withdrawal gas_used: 95,002 (cold storage slots)
- 2nd withdrawal gas_used: 52,902 (warm storage slots)
- 차이: 42,100 — EVM warm/cold storage access 비용 차이

**수정**: 옵션 C 적용 — 고정 가스 상수 제거, block header `gas_used` 사용
- `WITHDRAWAL_GAS`, `ETH_TRANSFER_GAS`, `SYSTEM_CALL_GAS` 상수 모두 삭제
- `handle_withdrawal()`, `handle_eth_transfer()`, `handle_system_call()` 더 이상 gas 반환 안 함
- `app_execution.rs`에서 `block.header.gas_used / non_priv_tx_count`로 gas 계산
- 모든 non-privileged tx 타입이 동일한 gas 소스(block header) 사용

**검증**: 2x 연속 E2E 테스트 24/24 통과, batch_2 + batch_10 fixture 자동 수집 완료

### 남은 작업 (TODO)

1. ~~[DEBUG-00e] 로그 제거~~ — 두 버그 모두 해결, 로그 불필요
2. compose-generator에 `ETHREX_DUMP_FIXTURES` 옵션 추가 (선택적 활성화)
3. 다른 앱 (evm-l2, tokamon) fixture 수집
4. CI 파이프라인에 fixture 테스트 추가
