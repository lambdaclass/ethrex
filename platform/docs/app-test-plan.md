# 앱별 오프라인 단위 테스트 계획

> **목표**: Docker 배포 없이, 저장된 fixture 데이터로 프루빙/검증 파이프라인 전체를 앱별로 단위 테스트
> **대상 앱**: zk-dex (programTypeId=2), 향후 evm-l2(1), tokamon(3)

---

## 1. 아키텍처: Fixture 기반 테스트

```
crates/guest-program/tests/
├── fixtures/
│   ├── zk-dex/
│   │   ├── batch_8_empty.json          # 빈 배치 (deposit만, 비특권 tx 없음)
│   │   ├── batch_11_deposit.json       # deposit 포함 배치
│   │   ├── batch_12_withdrawal.json    # withdrawal 포함 배치
│   │   └── README.md                   # fixture 수집 방법
│   ├── evm-l2/
│   │   └── ...
│   └── tokamon/
│       └── ...
├── test_program_output.rs              # ProgramOutput 인코딩 검증
├── test_public_values_hash.rs          # sha256(publicValues) 검증
├── test_state_continuity.rs            # batch간 state hash 연속성
├── test_balance_diffs.rs               # balance_diffs 계산 + 인코딩
├── test_commitment_match.rs            # committer calldata ↔ prover output 일치
└── test_gas_constants.rs               # 앱별 gas 상수 핀
```

### Fixture JSON 포맷 (앱 공통)

```json
{
  "app": "zk-dex",
  "batch_number": 11,
  "program_type_id": 2,
  "chain_id": 65536999,
  "description": "batch with 1 deposit tx",

  "program_output": {
    "initial_state_hash": "0xf2d3abac...",
    "final_state_hash": "0xe13dfb03...",
    "l1_out_messages_merkle_root": "0xc55f9da9...",
    "l1_in_messages_rolling_hash": "0x0001807d...",
    "blob_versioned_hash": "0x0177acaa...",
    "last_block_hash": "0xc73a2408...",
    "chain_id": "0x03e803e7",
    "non_privileged_count": 1,
    "balance_diffs": [],
    "l2_in_message_rolling_hashes": []
  },

  "encoded_public_values_hex": "f2d3abac...0001",
  "sha256_hash": "47b261816ac029786edfe31367cd76e2734541a6ddd44ff6d45a22901c98d9ef",

  "committer": {
    "new_state_root": "0xe13dfb03...",
    "withdrawals_merkle_root": "0xc55f9da9...",
    "priv_tx_rolling_hash": "0x0001807d...",
    "last_block_hash": "0xc73a2408...",
    "non_privileged_txs": 1,
    "commit_hash": "0xb04b4c08...",
    "balance_diffs_encoded": [],
    "l2_in_rolling_hashes_encoded": []
  },

  "proof_sender": {
    "prev_state_root": "0xf2d3abac...",
    "cur_state_root": "0xe13dfb03...",
    "l1_out_merkle_root": "0xc55f9da9..."
  }
}
```

---

## 2. 수집할 데이터 (로그 포인트)

배포 시 한 번만 수집하면 해당 앱의 모든 단위 테스트에 영구 사용 가능.

### 2.1 프루버 (`sp1.rs`) — 이미 [DEBUG-00e] 로그 있음

| 데이터 | 용도 | 상태 |
|--------|------|:----:|
| `public_values` (전체 hex) | ProgramOutput 인코딩 검증 | ✅ 수집됨 |
| `sha256(public_values)` | 해시 검증 | ✅ 수집됨 |
| 필드별 값 (8개 고정 필드) | 개별 필드 검증 | ✅ 수집됨 |
| `SP1Stdin` 직렬화 bytes | proof 재생성 (오프라인) | ❌ 추가 필요 |
| proof bytes (Groth16 calldata) | 오프체인 검증 | ❌ 추가 필요 |
| VK bytes | 오프체인 검증 | ❌ 추가 필요 |

### 2.2 커미터 (`l1_committer.rs`) — 이미 [DEBUG-00e] 로그 있음

| 데이터 | 용도 | 상태 |
|--------|------|:----:|
| commitBatch 전체 calldata 필드 | committer ↔ prover 일치 검증 | ✅ 수집됨 |
| balance_diffs 값 | balance_diffs 인코딩 검증 | ⚠️ 빈 배치만 수집됨 |
| `get_balance_diffs()` 입력 (L2 messages) | 계산 로직 검증 | ❌ 추가 필요 |
| native_token_scale_factor | 스케일링 검증 | ❌ 추가 필요 |

### 2.3 프루프 센더 (`l1_proof_sender.rs`) — 이미 [DEBUG-00e] 로그 있음

| 데이터 | 용도 | 상태 |
|--------|------|:----:|
| verifyBatch calldata | on-chain 검증 재현 | ❌ 추가 필요 |
| rollup store 데이터 (prev/cur state 등) | state 연속성 검증 | ✅ 수집됨 |

---

## 3. 테스트 단계별 계획

### Phase 1: Fixture 수집 로직 추가 (코드 변경)

기존 `[DEBUG-00e]` 로그를 **fixture 파일 자동 저장**으로 확장.
환경변수 `ETHREX_DUMP_FIXTURES=1`일 때만 활성화.

#### 3.1 프루버 fixture 덤프 (`sp1.rs`)

```rust
// prove() 성공 후:
if std::env::var("ETHREX_DUMP_FIXTURES").is_ok() {
    let dir = format!("fixtures/{}/{}", program_id, batch_number);
    std::fs::create_dir_all(&dir).ok();
    // SP1Stdin (witness) — proof 재생성용
    std::fs::write(format!("{dir}/stdin.bin"), &stdin_bytes).ok();
    // Public values — 인코딩 검증용
    std::fs::write(format!("{dir}/public_values.bin"), &pv_bytes).ok();
    // Proof calldata — 오프체인 검증용
    std::fs::write(format!("{dir}/proof.bin"), &proof_bytes).ok();
    // VK — 검증용
    std::fs::write(format!("{dir}/vk.bin"), &vk_bytes).ok();
    // JSON 메타데이터
    std::fs::write(format!("{dir}/metadata.json"), &json_metadata).ok();
}
```

#### 3.2 커미터 fixture 덤프 (`l1_committer.rs`)

```rust
if std::env::var("ETHREX_DUMP_FIXTURES").is_ok() {
    let dir = format!("fixtures/{}/{}", program_id, batch_number);
    // commitBatch calldata 필드들
    // balance_diffs 입력 (L2 messages) + 출력 (encoded diffs)
    // native_token_scale_factor
}
```

#### 3.3 프루프 센더 fixture 덤프 (`l1_proof_sender.rs`)

```rust
if std::env::var("ETHREX_DUMP_FIXTURES").is_ok() {
    let dir = format!("fixtures/{}/{}", program_id, batch_number);
    // verifyBatch calldata
    // rollup store 데이터 (prev/cur state 등)
}
```

### Phase 2: 오프라인 단위 테스트 (배포 불필요)

fixture 데이터 기반. `cargo test`로 실행.

#### Test 1: ProgramOutput 인코딩 (✅ 이미 구현)
- fixture의 필드 값으로 `ProgramOutput` 구성
- `.encode()` 결과가 `encoded_public_values_hex`와 일치 확인
- `sha256(encoded)` == fixture의 `sha256_hash`

#### Test 2: Committer ↔ Prover 일치
- fixture에서 committer calldata의 state_root, merkle_root 등 추출
- prover output의 동일 필드와 byte-exact 비교
- **이것이 00e 에러를 사전에 잡는 핵심 테스트**

```rust
#[test]
fn committer_matches_prover_batch11() {
    let fixture = load_fixture("zk-dex/batch_11_deposit.json");
    // committer의 new_state_root == prover의 final_state_hash
    assert_eq!(fixture.committer.new_state_root, fixture.program_output.final_state_hash);
    // committer의 withdrawals_merkle_root == prover의 l1_out_messages_merkle_root
    assert_eq!(fixture.committer.withdrawals_merkle_root, fixture.program_output.l1_out_messages_merkle_root);
    // ... 모든 공유 필드 비교
}
```

#### Test 3: Balance Diffs 계산
- fixture에서 L2 messages 입력 + native_token_scale_factor
- `get_balance_diffs()` 호출
- 결과가 fixture의 balance_diffs와 일치

```rust
#[test]
fn balance_diffs_zk_dex_batch12() {
    let fixture = load_fixture("zk-dex/batch_12_withdrawal.json");
    let diffs = get_balance_diffs(&fixture.l2_messages, fixture.native_token_scale_factor);
    assert_eq!(diffs, fixture.expected_balance_diffs);
}
```

#### Test 4: State Hash 연속성
- 여러 배치 fixture 로드
- batch N의 `final_state_hash` == batch N+1의 `initial_state_hash`
- (✅ 이미 구현, fixture 기반으로 확장)

#### Test 5: Gas 상수 핀 (✅ 이미 구현)
- 앱별 gas 상수가 변경되면 테스트 실패
- 변경 시 의도적으로 fixture 업데이트 필요

### Phase 3: 오프라인 프루빙 테스트 (선택, 느림)

SP1Stdin fixture로 proof 재생성. CI에서 주기적 실행 (8~10분).

```rust
#[test]
#[ignore] // cargo test --ignored 로만 실행
fn sp1_prove_zk_dex_batch11() {
    let stdin_bytes = std::fs::read("fixtures/zk-dex/11/stdin.bin").unwrap();
    let stdin: SP1Stdin = deserialize(&stdin_bytes);
    let elf = include_bytes!("../bin/sp1-zk-dex/out/riscv32im-succinct-zkvm-elf");
    let client = ProverClient::builder().cpu().build();
    let (pk, vk) = client.setup(elf);
    let proof = client.prove(&pk, &stdin).compressed().run().unwrap();
    // public values 일치 확인
    let pv = proof.public_values.to_vec();
    let expected = std::fs::read("fixtures/zk-dex/11/public_values.bin").unwrap();
    assert_eq!(pv, expected);
}
```

### Phase 4: 오프라인 검증 테스트 (선택, 빠름)

저장된 proof + VK로 오프체인 검증. 수 초.

```rust
#[test]
#[ignore]
fn sp1_verify_zk_dex_batch11() {
    let proof_bytes = std::fs::read("fixtures/zk-dex/11/proof.bin").unwrap();
    let vk_bytes = std::fs::read("fixtures/zk-dex/11/vk.bin").unwrap();
    let proof: SP1ProofWithPublicValues = bincode::deserialize(&proof_bytes).unwrap();
    let vk: SP1VerifyingKey = bincode::deserialize(&vk_bytes).unwrap();
    let client = ProverClient::builder().cpu().build();
    client.verify(&proof, &vk).expect("proof verification failed");
}
```

### Phase 5: Foundry on-chain 검증 테스트 (선택)

Solidity 테스트로 L1 OnChainProposer.verifyBatch() 재현.

```solidity
// test/VerifyBatchFixture.t.sol
function test_verifyBatch_zk_dex_batch11() public {
    bytes memory proof = vm.readFileBinary("fixtures/zk-dex/11/proof_calldata.bin");
    bytes32 vk = 0x00c40e105259b564873710f4a0369401b02b5cd9e5ecbc42fb63f427df67fdd8;
    bytes memory publicValues = vm.readFileBinary("fixtures/zk-dex/11/public_values.bin");
    // SP1Verifier should not revert
    ISP1Verifier(sp1Verifier).verifyProof(vk, publicValues, proof);
}
```

---

## 4. 구현 우선순위

| 순서 | 작업 | 소요 | 효과 |
|:----:|------|------|------|
| **1** | Fixture JSON 포맷 정의 + 로더 유틸 | 1시간 | 인프라 |
| **2** | 기존 로그에서 zk-dex batch 8/11/12 fixture 생성 | 30분 | 즉시 테스트 가능 |
| **3** | Test 1~5 구현 (Phase 2) | 2시간 | **핵심: 00e 재발 방지** |
| **4** | `ETHREX_DUMP_FIXTURES` 덤프 로직 추가 (Phase 1) | 2시간 | 다음 배포시 자동 수집 |
| **5** | Phase 3 (오프라인 프루빙) | 1시간 | proof 재현성 검증 |
| **6** | Phase 4 (오프라인 검증) | 1시간 | 검증 로직 독립 테스트 |
| **7** | Phase 5 (Foundry) | 2시간 | L1 컨트랙트 검증 |

---

## 5. 새 앱 추가 시 워크플로우

```
1. 앱 개발 완료 (guest program + circuit)
2. Docker 배포 1회 (ETHREX_DUMP_FIXTURES=1)
3. fixture 자동 생성됨: fixtures/{app_name}/{batch_number}/
4. fixture를 테스트 디렉토리에 복사
5. cargo test — 모든 오프라인 테스트 실행
6. 이후 코드 변경 시 cargo test만으로 회귀 검증
```

---

## 6. 현재 코드 변경 필요 사항

### 6.1 수정 필요 파일

| 파일 | 변경 내용 |
|------|----------|
| `crates/l2/prover/src/backend/sp1.rs` | fixture 덤프 로직 추가 |
| `crates/l2/sequencer/l1_committer.rs` | fixture 덤프 로직 추가 |
| `crates/l2/sequencer/l1_proof_sender.rs` | fixture 덤프 로직 추가 |
| `crates/guest-program/Cargo.toml` | dev-deps 추가 (serde_json) |

### 6.2 새로 생성할 파일

| 파일 | 내용 |
|------|------|
| `crates/guest-program/tests/fixtures/` | fixture 데이터 디렉토리 |
| `crates/guest-program/tests/fixture_loader.rs` | JSON fixture 로더 유틸 |
| `crates/guest-program/tests/test_commitment_match.rs` | committer ↔ prover 일치 |
| `crates/guest-program/tests/test_balance_diffs.rs` | balance_diffs 계산 검증 |

### 6.3 기존 테스트 이동/확장

| 현재 위치 | 변경 |
|-----------|------|
| `output.rs` 내 batch 8/11 테스트 | fixture JSON 기반으로 전환 |
| `constants.rs` 내 gas 상수 테스트 | 앱별 gas 상수 매핑 추가 |
