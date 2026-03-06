# 앱별 오프라인 단위 테스트 계획

> **목표**: Docker 배포 없이, 저장된 fixture 데이터로 프루빙/검증 파이프라인 전체를 앱별로 단위 테스트
> **대상 앱**: zk-dex (programTypeId=2), 향후 evm-l2(1), tokamon(3)

---

## 현재 상태 요약 (2026-03-06)

| 항목 | 상태 | 비고 |
|------|:----:|------|
| Fixture JSON 포맷 + 로더 | ✅ | `fixture_types.rs`, `discover_all_apps()` |
| Fixture 자동 수집 (`ETHREX_DUMP_FIXTURES`) | ✅ | prover.rs + l1_committer.rs |
| Fixture 병합 스크립트 | ✅ | `merge-fixtures.sh` |
| Test 1: ProgramOutput 인코딩 | ✅ | `test_program_output.rs` |
| Test 2: Committer ↔ Prover 일치 | ✅ | `test_commitment_match.rs` |
| Test 3: Balance Diffs 계산 | ✅ | `messages.rs` 내 7개 단위 테스트 |
| Test 4: State Hash 연속성 | ✅ | `test_state_continuity.rs` |
| Test 5: Gas 상수 핀 | N/A | gas 상수 제거됨 (block header gas 사용으로 전환) |
| 앱 자동 탐색 | ✅ | `discover_all_apps()` — 디렉토리만 추가하면 자동 |
| CI workflow | ✅ | `.github/workflows/pr_fixture_tests.yml` |
| 새 앱 추가 가이드 | ✅ | `adding-new-app-fixtures.md` |
| Phase 3: 오프라인 프루빙 | ⚠️ | dump 코드 완료, Docker 재빌드 + fixture 재수집 필요 |
| Phase 4: 오프라인 검증 | ⚠️ | dump 코드 완료, Docker 재빌드 + fixture 재수집 필요 |
| Phase 5: Foundry on-chain 검증 | ❌ | Solidity 테스트 미구현 |
| Prover balance_diffs 디코딩 | ⚠️ | 인코딩 포맷 한계로 빈 배열, warn 처리 |

---

## 1. 아키텍처: Fixture 기반 테스트

```
crates/guest-program/tests/
├── fixtures/
│   ├── zk-dex/                           # ✅ 2개 fixture 수집됨
│   │   ├── batch_2_deposit_withdraw.json
│   │   └── batch_10_deposit_withdraw_2nd.json
│   ├── evm-l2/                           # ❌ 미수집 (배포 필요)
│   └── tokamon/                          # ❌ 미수집 (배포 필요)
├── fixture_types.rs                      # ✅ 로더 + discover_all_apps()
├── test_program_output.rs                # ✅ 앱 자동 탐색
├── test_commitment_match.rs              # ✅ 앱 자동 탐색
├── test_state_continuity.rs              # ✅ 앱 자동 탐색 (chain_id, program_type_id 포함)
└── merge-fixtures.sh                     # ✅ 범용 병합 스크립트
```

### Fixture JSON 포맷

```json
{
  "app": "zk-dex",
  "batch_number": 2,
  "program_type_id": 2,
  "chain_id": 65536999,
  "description": "batch with deposit + withdrawal",
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

---

## 2. 수집할 데이터 (로그 포인트)

### 2.1 프루버 (`prover.rs`) — 현재 수집 상태

| 데이터 | 용도 | 상태 |
|--------|------|:----:|
| `public_values` (전체 hex) | ProgramOutput 인코딩 검증 | ✅ prover.json에 저장 |
| `sha256(public_values)` | 해시 검증 | ✅ prover.json에 저장 |
| 필드별 값 (8개 고정 필드) | 개별 필드 검증 | ✅ prover.json에 저장 |
| `balance_diffs` (가변 필드) | balance_diffs 검증 | ⚠️ 빈 배열 (디코딩 불가) |
| `stdin.bin` (직렬화 input) | proof 재생성 (오프라인) | ✅ prove_batch()에서 dump |
| `proof.bin` (BatchProof bincode) | 오프체인 검증 | ✅ fixture dump에서 저장 |
| VK bytes | 오프체인 검증 | N/A (SP1은 ELF에서 VK 생성) |

### 2.2 커미터 (`l1_committer.rs`) — 현재 수집 상태

| 데이터 | 용도 | 상태 |
|--------|------|:----:|
| commitBatch calldata 필드 | committer ↔ prover 일치 검증 | ✅ committer.json에 저장 |
| balance_diffs 값 | balance_diffs 인코딩 검증 | ✅ committer.json에 저장 |
| `get_balance_diffs()` 입력 (L2 messages) | 계산 로직 검증 | ❌ 미수집 (단위 테스트로 대체) |
| native_token_scale_factor | 스케일링 검증 | ❌ 미수집 (단위 테스트로 대체) |

### 2.3 프루프 센더 (`l1_proof_sender.rs`)

| 데이터 | 용도 | 상태 |
|--------|------|:----:|
| verifyBatch calldata | on-chain 검증 재현 | ❌ 미수집 |
| rollup store 데이터 | state 연속성 검증 | ❌ 미수집 (fixture로 대체) |

---

## 3. 남은 작업 (Phase 3~5)

### Phase 3: 오프라인 프루빙 테스트

**목표**: SP1Stdin fixture로 proof를 오프라인에서 재생성하여 결정론적 검증.

**Dump 코드**: ✅ 완료
- `prove_batch()` registry path: `serialized` → `stdin.bin`
- `prove_batch()` legacy path: `serialize_raw(&input)` → `stdin.bin`
- Fixture dump: `bincode::serialize(&batch_proof)` → `proof.bin`

**남은 작업**:
1. Docker 이미지 재빌드 (stdin.bin/proof.bin dump 코드 포함)
2. Fixture 재수집 (기존 prover.json + 새 stdin.bin/proof.bin)
3. 테스트 작성 — SP1 SDK 의존성으로 `ethrex-prover` crate에 위치:

```rust
// crates/l2/prover/tests/test_offline_proving.rs
#[test]
#[ignore] // cargo test -p ethrex-prover --ignored (SP1 필요, ~10분)
fn sp1_reprove_from_fixture() {
    let stdin_bytes = std::fs::read("/path/to/fixtures/zk-dex/batch_2/stdin.bin").unwrap();
    let elf = ethrex_guest_program::ZKVM_SP1_PROGRAM_ELF;
    let client = sp1_sdk::CpuProver::new();
    let (pk, _vk) = client.setup(elf);
    let mut stdin = sp1_sdk::SP1Stdin::new();
    stdin.write_slice(&stdin_bytes);
    let proof = client.prove(&pk, &stdin, sp1_sdk::SP1ProofMode::Compressed).unwrap();
    // Compare public_values with prover.json's encoded_public_values
}
```

**의존성**: SP1 SDK (`sp1` feature), 느림 (~10분/batch)

---

### Phase 4: 오프라인 검증 테스트

**목표**: 저장된 proof.bin으로 오프체인 검증. 빠름 (수 초).

**Dump 코드**: ✅ 완료 (`proof.bin` = bincode-serialized `BatchProof`)

**남은 작업**:
1. Fixture 재수집 (Phase 3과 동시)
2. 테스트 작성:

```rust
// crates/l2/prover/tests/test_offline_verify.rs
#[test]
#[ignore] // cargo test -p ethrex-prover --ignored
fn sp1_verify_from_fixture() {
    let proof_bytes = std::fs::read("/path/to/fixtures/zk-dex/batch_2/proof.bin").unwrap();
    let batch_proof: BatchProof = bincode::deserialize(&proof_bytes).unwrap();
    // Extract SP1ProofWithPublicValues from BatchProof::ProofBytes
    // Setup SP1 client from ELF → verify
}
```

**의존성**: Phase 3 fixture 수집, SP1 SDK

---

### Phase 5: Foundry on-chain 검증 테스트

**목표**: Solidity 테스트로 L1 OnChainProposer.verifyBatch() 재현.

**필요한 코드 변경**:

1. Phase 4의 proof calldata 수집이 선행 조건
2. Foundry 프로젝트에 테스트 추가: `crates/l2/contracts/test/VerifyBatchFixture.t.sol`

```solidity
function test_verifyBatch_zk_dex() public {
    bytes memory proof = vm.readFileBinary("fixtures/zk-dex/batch_2/proof_calldata.bin");
    bytes32 vk = ...; // verifying key hash
    bytes memory publicValues = vm.readFileBinary("fixtures/zk-dex/batch_2/public_values.bin");
    ISP1Verifier(sp1Verifier).verifyProof(vk, publicValues, proof);
}
```

**의존성**: Phase 4 완료, Foundry, SP1 Verifier 컨트랙트
**우선순위**: 낮음 — L1 컨트랙트 변경 시 회귀 테스트로 가치 있음

---

## 4. Test 5 (Gas 상수 핀) — 해당 없음

원래 계획에서는 앱별 고정 가스 상수를 핀 테스트로 보호하려 했으나,
2026-03-06에 고정 가스 상수를 **block header gas_used**로 전환했으므로
이 테스트는 더 이상 필요하지 않음.

- `WITHDRAWAL_GAS`, `ETH_TRANSFER_GAS`, `SYSTEM_CALL_GAS` 상수 모두 삭제됨
- `app_execution.rs`에서 `block.header.gas_used / non_priv_tx_count`로 계산
- 상세: `fixture-data-collection.md` (2026-03-06 섹션)

---

## 5. deployment-engine-refactoring.md 남은 항목

| 항목 | 상태 | 비고 |
|------|:----:|------|
| Tools compose 포트 동적화 | ✅ | `TOOLS_*_PORT` 환경변수로 동적 할당 |
| GPU 감지 및 compose override | ✅ | `hasNvidiaGpu()` + compose deploy.resources |
| Metrics 포트 노출 | ✅ | `toolsMetricsPort` DB + compose 연동 |
| Deployer exit code 검증 | ✅ | bridge/proposer null 검증 + 에러 throw |

---

## 6. 구현 우선순위 (업데이트)

| 순서 | 작업 | 상태 | 비고 |
|:----:|------|:----:|------|
| 1 | Fixture JSON 포맷 + 로더 | ✅ | `fixture_types.rs` |
| 2 | Fixture 자동 수집 로직 | ✅ | `ETHREX_DUMP_FIXTURES` |
| 3 | Test 1~4 구현 | ✅ | 5개 테스트 파일 |
| 4 | Test 앱 자동 탐색 | ✅ | `discover_all_apps()` |
| 5 | CI workflow | ✅ | `pr_fixture_tests.yml` |
| 6 | 새 앱 추가 가이드 | ✅ | `adding-new-app-fixtures.md` |
| **7** | **다른 앱 fixture 수집** | **❌** | **evm-l2, tokamon 배포 필요** |
| **8** | **Phase 3: 오프라인 프루빙** | **⚠️** | **dump 코드 완료, 테스트는 fixture 재수집 후** |
| **9** | **Phase 4: 오프라인 검증** | **⚠️** | **dump 코드 완료, 테스트는 fixture 재수집 후** |
| **10** | **Phase 5: Foundry 검증** | **❌** | **Phase 4 선행** |
| 11 | Tools 포트 동적화 | ✅ | `TOOLS_*_PORT` 환경변수 |
| 12 | GPU 감지 compose | ✅ | `hasNvidiaGpu()` + NVIDIA device reservation |
| 13 | Metrics 포트 | ✅ | DB 할당 + compose 연동 |
| 14 | Deployer exit code | ✅ | bridge/proposer 주소 null 검증 |

---

## 7. 새 앱 추가 시 워크플로우

```
1. Guest Program 구현 (또는 기존 앱 사용)
2. 앱 등록 (resolve_program_type_id, db.js, prover registry)
3. Docker Compose 프로필 추가 (compose-generator.js)
4. Docker 배포 + ETHREX_DUMP_FIXTURES 활성화
5. E2E 테스트로 트랜잭션 생성
6. merge-fixtures.sh로 fixture 병합
7. tests/fixtures/<my-app>/ 에 JSON 복사
8. cargo test → 자동 통과 (discover_all_apps)
9. PR → CI 자동 검증 (pr_fixture_tests.yml)
```

상세 가이드: `adding-new-app-fixtures.md`
