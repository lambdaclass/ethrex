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
ls -la /tmp/fixtures/zk-dex/
# batch_1/committer.json, batch_1/prover.json
# batch_2/committer.json, batch_2/prover.json
# ...
```

Note: Empty/genesis batches (1-2) only have committer.json because the prover
skips them. Only batches with actual transactions will have prover.json.

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

## 내일 해야 할 작업 (TODO)

### 1단계: fixture dump 버그 수정 배포 (최우선)
1. **Docker 이미지 재빌드** — `ProofCalldata.public_values` 수정이 반영된 이미지
2. **L2 + 프루버 재시작** — 새 이미지로 (L1은 유지 가능)
3. **이전 fixture 데이터 삭제** — `/tmp/fixtures/zk-dex/batch_*/prover.json` (잘못된 데이터)
4. **E2E 테스트 재실행** — deposit + withdrawal 트랜잭션 생성
5. **프루버 완료 대기** — batch에 대한 SP1 proof 생성 (각 ~3분)

### 2단계: fixture 데이터 수집 및 테스트 검증
6. **merge-fixtures.sh 실행** — committer.json + prover.json 병합
7. **repo에 복사** — `crates/guest-program/tests/fixtures/zk-dex/` 디렉토리
8. **오프라인 테스트 실행** — `cargo test` 로 세 가지 테스트 통과 확인:
   - `test_program_output` — 인코딩 일치
   - `test_commitment_match` — committer ↔ prover 필드 일치 (00e 방지)
   - `test_state_continuity` — 배치 간 state hash 연속성

### 2.5단계: 앱별 fixture 구조 테스트 검증
- 현재 fixture는 `tests/fixtures/{app}/batch_{N}.json` 구조로 앱별 분리됨
- `load_fixture(app, filename)`, `load_all_fixtures(app)` 로 앱별로 로드
- **검증할 것**:
  - `load_all_fixtures("zk-dex")`가 새로 수집한 fixture를 정상 로드하는지
  - `fixture_to_program_output()`가 앱별 chain_id, program_type_id를 올바르게 변환하는지
  - 다른 앱 디렉토리(`evm-l2/`)가 없을 때 빈 Vec 반환하는지
  - 앱별 fixture 간 데이터가 섞이지 않는지 (chain_id 일관성 확인)
- **관련 파일**:
  - `crates/guest-program/tests/fixture_types.rs` — 로더 + 변환 헬퍼
  - `crates/guest-program/tests/test_program_output.rs` — 인코딩 테스트
  - `crates/guest-program/tests/test_commitment_match.rs` — committer ↔ prover 일치
  - `crates/guest-program/tests/test_state_continuity.rs` — 배치 간 연속성

### 3단계: 정리 및 커밋
9. **기존 수동 fixture 교체** — batch_8/11/12 (이전 세션에서 수동 캡처한 것) → 자동 수집 데이터로
10. **[DEBUG-00e] 로그 정리** — `sp1.rs`, `l1_proof_sender.rs`에서 디버그 로그 제거
    (문제 해결 후 더 이상 불필요)
11. **커밋** — fixture dump 버그 수정 + 새 fixture 데이터 + 테스트 통과 확인

### 4단계: 인프라 개선 (선택)
12. compose-generator에 `ETHREX_DUMP_FIXTURES` 옵션 추가 (선택적 활성화)
13. 다른 앱 (evm-l2, tokamon) fixture 수집
14. CI 파이프라인에 fixture 테스트 추가
