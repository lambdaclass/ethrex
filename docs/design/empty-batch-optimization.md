# Empty Batch Optimization — ZK Proof-Free Verification

## Problem

L2 블록 프로듀서가 트랜잭션이 없어도 매 2초마다 빈 블록을 생성한다.
빈 블록이 수백~수천 개 쌓이면:

1. **Blob overflow**: 빈 블록 헤더(~685 bytes/block)가 blob 용량(127KB)을 초과
2. **Batch size limit**: `generate_blobs_bundle` 실패 → 빈 블록만으로 배치가 꽉 참
3. **불필요한 증명**: 빈 배치마다 SP1 증명 생성(~3분) → 순수 낭비
4. **무한 루프**: 빈 배치를 스킵하면 배치 번호가 안 넘어가서 커미터가 멈춤

### 비용 분석 (Docker 테스트 기준)

| 시나리오 | 빈 배치 수 | 증명 시간 | L1 가스 |
|---------|-----------|----------|---------|
| 배치 3 이후 1000 빈 블록 | ~10 배치 | ~30분 | ~10 verify tx |
| 최적화 후 | 0 배치 | 0분 | 0 verify tx |

## Solution: Contract-Level Empty Batch Auto-Verification

### Core Idea

빈 배치는 상태 변경이 없으므로 ZK 증명 없이 **컨트랙트에서 직접 검증**할 수 있다.

검증 조건 (모두 충족 시 빈 배치):
```
newStateRoot == previousBatch.newStateRoot     // 상태 불변
nonPrivilegedTransactions == 0                  // 일반 트랜잭션 없음
withdrawalsLogsMerkleRoot == bytes32(0)         // 출금 없음
processedPrivilegedTransactionsRollingHash == bytes32(0)  // 입금 없음
balanceDiffs.length == 0                        // 잔액 변동 없음
l2InMessageRollingHashes.length == 0            // 크로스체인 메시지 없음
```

### Security Argument

빈 배치 자동 검증은 안전하다:
- **상태 루트 불변**: `newStateRoot == prev.newStateRoot`이므로 자금 이동 불가
- **메시지 없음**: 크로스체인 효과 없음
- **트랜잭션 없음**: 어떤 상태 변경도 발생하지 않음
- **컨트랙트 검증**: 조건은 온체인에서 강제됨, 오프체인 신뢰 불필요

## Architecture

```
                    ┌──────────────┐
                    │ Block Producer│  빈 블록 생산 (2초마다)
                    └──────┬───────┘
                           │
                    ┌──────▼───────┐
                    │  Committer   │  배치 어셈블리
                    └──────┬───────┘
                           │
                ┌──────────┼──────────┐
                │          │          │
         빈 블록만의      혼합 배치      트랜잭션 배치
           배치        (빈+트랜잭션)
                │          │          │
         ┌──────▼──────┐  │    ┌─────▼─────┐
         │ Fast-forward │  │    │  Normal   │
         │ (blob 없이) │  │    │ (blob+증명)│
         └──────┬──────┘  │    └─────┬─────┘
                │         │          │
         ┌──────▼──────────▼──────────▼──────┐
         │         L1 OnChainProposer        │
         ├───────────────────────────────────┤
         │  commitBatch()                    │
         │    └─ 빈 배치: validium 모드 허용  │
         │                                   │
         │  verifyBatch()                    │
         │    ├─ 빈 배치? → 자동 검증 (증명X) │
         │    └─ 일반 배치? → SP1 증명 검증   │
         └───────────────────────────────────┘
```

## Implementation Plan

### Phase 1: L1 Contract — `OnChainProposer.sol`

#### 1-1. `verifyBatch()` 수정

```solidity
function verifyBatch(...) external override whenNotPaused {
    // ... 기존 require 체크 (batchNumber, committed 여부) ...
    // ... 기존 privileged tx / L2 message 처리 ...

    // ── NEW: Empty batch auto-verification ──
    bool isEmptyBatch = _isEmptyBatch(batchNumber);

    if (!isEmptyBatch) {
        // 기존 증명 검증 로직 (SP1, RISC0, TDX)
        bytes memory publicInputs = _getPublicInputsFromCommitment(batchNumber);
        if (REQUIRE_SP1_PROOF) { ... }
        if (REQUIRE_RISC0_PROOF) { ... }
        if (REQUIRE_TDX_PROOF) { ... }
    }

    // publishL2Messages, lastVerifiedBatch 업데이트 등은 동일
    ...
    emit BatchVerified(lastVerifiedBatch);
}
```

#### 1-2. `_isEmptyBatch()` 내부 함수 추가

```solidity
function _isEmptyBatch(uint256 batchNumber) internal view returns (bool) {
    BatchCommitmentInfo storage current = batchCommitments[batchNumber];
    BatchCommitmentInfo storage previous = batchCommitments[lastVerifiedBatch];

    return (
        current.newStateRoot == previous.newStateRoot &&
        current.nonPrivilegedTransactions == 0 &&
        current.withdrawalsLogsMerkleRoot == bytes32(0) &&
        current.processedPrivilegedTransactionsRollingHash == bytes32(0) &&
        current.balanceDiffs.length == 0 &&
        current.l2InMessageRollingHashes.length == 0
    );
}
```

#### 1-3. `commitBatch()` 수정 — 빈 배치 blob 면제

현재 rollup 모드에서 blob이 필수(`blobhash(0) != 0`).
빈 배치는 blob 데이터가 필요 없으므로 면제:

```solidity
// 기존:
if (!VALIDIUM) {
    require(blobVersionedHash != 0, "007");
}

// 수정:
bool isEmptyCommit = (
    nonPrivilegedTransactions == 0 &&
    processedPrivilegedTransactionsRollingHash == bytes32(0) &&
    withdrawalsLogsMerkleRoot == bytes32(0) &&
    balanceDiffs.length == 0 &&
    l2MessageRollingHashes.length == 0
);

if (!VALIDIUM && !isEmptyCommit) {
    require(blobVersionedHash != 0, "007");
}
```

### Phase 2: L2 Committer — `l1_committer.rs`

#### 2-1. 배치 어셈블리 빈 블록 fast-forward

`prepare_batch_from_block()` 루프에서 빈 블록 최적화:

```rust
// 빈 블록: blob 인코딩, receipt 조회, 메시지 추출 건너뛰기
if potential_batch_block.body.transactions.is_empty() {
    new_state_root = checkpoint_store
        .state_trie(potential_batch_block.hash())?
        .ok_or(...)?
        .hash_no_commit();
    last_added_block_number += 1;
    acc_blocks.push((last_added_block_number, potential_batch_block.hash()));
    continue;  // blob size limit, gas limit 체크 안 함
}
```

이렇게 하면:
- 1000개 빈 블록 → blob 0 bytes (limit에 안 걸림)
- 11개 tx 블록만 blob에 포함
- 한 배치로 모두 처리 가능

#### 2-2. 빈 배치 blob 없이 커밋

빈 블록만의 배치인 경우, `send_commitment()`에서 blob 없이 L1 커밋:

```rust
// Batch가 완전히 빈 경우 (non_privileged_transactions == 0 && no messages)
let is_empty_batch = batch.non_privileged_transactions == 0
    && batch.l1_in_messages_rolling_hash == H256::zero()
    && batch.l1_out_message_hashes.is_empty()
    && batch.balance_diffs.is_empty();

if is_empty_batch {
    // Blob 없이 일반 트랜잭션으로 commitBatch 호출
    send_commitment_without_blob(batch);
} else {
    // 기존: blob 포함 EIP-4844 트랜잭션
    send_commitment_with_blob(batch);
}
```

### Phase 3: L2 Proof Sender — `l1_proof_sender.rs`

#### 3-1. 빈 배치 증명 건너뛰기

`verify_and_send_proof()`에서 빈 배치를 감지하고 빈 증명으로 즉시 verify:

```rust
async fn verify_and_send_proof(&self) -> Result<(), ProofSenderError> {
    // ... 기존 batch_to_send 계산 ...

    // 빈 배치 감지: DB에서 배치 정보 조회
    if let Some(batch) = self.rollup_store.get_batch_info(batch_to_send).await? {
        if batch.is_empty() {
            // 빈 증명으로 verifyBatch 호출 → 컨트랙트가 자동 검증
            self.send_empty_batch_verification(batch_to_send).await?;
            self.rollup_store.set_latest_sent_batch_proof(batch_to_send).await?;
            return Ok(());
        }
    }

    // ... 기존 증명 대기 및 제출 로직 ...
}
```

## File Changes Summary

| File | Change | Description |
|------|--------|-------------|
| `OnChainProposer.sol` | MODIFY | `_isEmptyBatch()` 추가, `verifyBatch()` 자동검증, `commitBatch()` blob 면제 |
| `IOnChainProposer.sol` | NO CHANGE | 인터페이스 변경 없음 (verifyBatch 시그니처 동일) |
| `l1_committer.rs` | MODIFY | 빈 블록 fast-forward, 빈 배치 blob 없이 커밋 |
| `l1_proof_sender.rs` | MODIFY | 빈 배치 감지 → 빈 증명으로 즉시 verify |

## Verification Plan

1. **Unit Test**: `_isEmptyBatch()` 조건 테스트 (Solidity)
2. **Integration Test**: 빈 배치 commit → verify (증명 없이) → 다음 배치 commit → verify (증명 포함)
3. **E2E Test**: Docker 환경에서 빈 블록 갭 → 트랜잭션 → 전체 flow 확인
4. **Security Check**: 빈 배치로 위장한 악성 배치가 자동 검증되지 않는지 확인

## Expected Performance

| Metric | Before | After |
|--------|--------|-------|
| 빈 배치 증명 시간 | ~175초/배치 | **0초** |
| 빈 배치 L1 가스 | ~300K (verify + blob) | **~50K** (verify only, no blob) |
| 1000 빈 블록 처리 | ~30분 (10 배치 × 3분) | **~5초** (blob 없이 commit+verify) |
| 프루버 리소스 | 100% 낭비 | **0% 낭비** |
