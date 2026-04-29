# Least Authority Security Audit -- Verification Response

**Project:** ethrex
**Audit date:** January 12 -- January 30, 2026
**Audit report delivered:** February 3, 2026
**Audit revision:** `e88175e2d49f1192cc9f2fdeae6fde1392d0759d`
**Response date:** April 6, 2026
**Original report:** [Least Authority - LambdaClass Ethrex Initial Audit Report (PDF)](https://github.com/lambdaclass/ethrex/blob/main/audits/ethrex_v9.0.0_e88175e2/Least%20Authority%20-%20LambdaClass%20Ethrex%20Initial%20Audit%20Report.pdf)

---

## Summary

The audit identified 20 issues (A--T) and 8 suggestions (1--8). Each finding is addressed below with links to the relevant PRs.

| Category | Count |
|----------|-------|
| Fixed (with PR) | 20 issues + 5 suggestions |
| Not applicable | 3 issues + 1 suggestion |
| Acknowledged (ongoing) | 2 suggestions |
| Remaining (latent, zero impact) | 1 issue (partial) |

---

## Issues

### Issue A: Aligned Layer Service Interruptions Can Result in Denial of Service

| Field | Value |
|-------|-------|
| Severity | High |
| Status | **Fixed** |
| PRs | [#6313](https://github.com/lambdaclass/ethrex/pull/6313), [#5869](https://github.com/lambdaclass/ethrex/pull/5869) |

**Finding:** When running in aligned-mode, the sequencer advanced its proof cursor upon sending to Aligned Layer rather than upon on-chain confirmation. A proof lost by Aligned would never be resent, stalling batch verification indefinitely.

**Resolution:** PR #6313 introduces a dual-cursor architecture:

- `latest_sent_to_aligned` -- tracks what was dispatched to the Aligned gateway (written by `L1ProofSender`).
- `latest_verified_batch_proof` -- tracks what was confirmed on-chain (written only by `L1ProofVerifier` after on-chain verification succeeds).

A configurable `--aligned.resubmission-timeout` triggers automatic resend when aggregation is not observed within the timeout window, resetting the aligned cursor to `last_verified_batch + 1`. Checkpoint directories are only cleaned after on-chain verification.

PR #5869 adds operator-facing documentation covering 5 failure scenarios (temporary outage, lost proof, permanent shutdown, insufficient balance, invalid proof) with recovery procedures.

---

### Issue B: Intrinsic Gas Error Can Result in Loss of Funds

| Field | Value |
|-------|-------|
| Severity | High |
| Status | **Fixed** |
| PR | [#6324](https://github.com/lambdaclass/ethrex/pull/6324) |

**Finding:** In `prepare_execution_privileged` (`l2_hook.rs`), the sender's balance was debited via `decrease_account_balance` before `add_intrinsic_gas()` validation completed. If that check failed, `msg_value` was zeroed, making `undo_value_transfer` a no-op and permanently burning the sender's funds.

**Resolution:** The balance debit is now deferred to after all `tx_should_fail` checks complete (including `add_intrinsic_gas`). A regression test (`privileged_tx_intrinsic_gas_failure_preserves_sender_balance`) confirms that a privileged tx with insufficient gas preserves the sender's full balance.

---

### Issue C: Nonatomic Finalization Can Lead to Inconsistent State

| Field | Value |
|-------|-------|
| Severity | High |
| Status | **Fixed** |
| PR | [#6330](https://github.com/lambdaclass/ethrex/pull/6330) |

**Finding:** `finalize_non_privileged_execution` applied sequential state mutations and returned early on error, leaving partial mutations committed without rollback.

**Resolution:** The function now has two phases:

1. **Phase 1 (fallible computations):** All operations that can fail (e.g., `get_fee_token_ratio`) run before any state mutations.
2. **Phase 2 (state mutations with rollback):** All mutations are grouped in `apply_finalize_mutations`. If any step fails, `restore_cache_state()` reverts all partial mutations.

`transfer_fee_token` now backs up original storage slot values before overwriting, and `restore_cache_state` also restores `status` and `has_storage` fields. Regression tests cover arithmetic overflow and contract revert failure modes.

---

### Issue D: ERC-20 AssetDiffs Omitted in BalanceDiff Aggregation

| Field | Value |
|-------|-------|
| Severity | High |
| Status | **Fixed** |
| PR | [#5882](https://github.com/lambdaclass/ethrex/pull/5882) |

**Finding:** In `get_balance_diffs` (`messages.rs`), decoded ERC-20 `AssetDiff` entries were never inserted into the `value_per_token` vector when no matching entry existed, leaving it empty.

**Resolution:** The aggregation logic now uses `find()` to locate an existing matching entry. If found, the value is incremented; otherwise, the decoded `AssetDiff` is pushed into the vector via an `else` branch. An integration test verifies L1 deposit accounting for ERC-20 cross-chain transfers.

---

### Issue E: Permissionless Privileged Message Parameters Can Stall Verification

| Field | Value |
|-------|-------|
| Severity | High |
| Status | **Fixed** |
| PR | [#6442](https://github.com/lambdaclass/ethrex/pull/6442) |

**Finding:** The `sendToL2` function in `CommonBridge.sol` accepts user-controlled `gasLimit` and `data` without validation against L2 inclusion constraints. A malicious actor could create non-includable privileged transactions that expire and block verification.

**Existing mitigation:** `_burnGas(sendValues.gasLimit)` on L1 caps the effective `gasLimit` to approximately the L1 block gas limit (~30M gas per EIP-7825), providing economic deterrence.

**Resolution:** PR #6442 adds an `l2GasLimit` storage variable to `CommonBridge`, set via an `initialize()` parameter (deployer default: 30,000,000) and updatable post-deployment by the owner via `setL2GasLimit()`. `_sendToL2()` enforces `sendValues.gasLimit <= l2GasLimit`, rejecting privileged transactions that exceed the limit. The `--block-producer.block-gas-limit` CLI flag is removed; the sequencer now fetches `l2GasLimit` from the contract on startup, keeping the on-chain constraint and sequencer limit in sync.

---

### Issue F: Underflow and Off-by-One in `regenerate_state` Target Handling

| Field | Value |
|-------|-------|
| Severity | High |
| Status | **Fixed** |
| PR | [#6331](https://github.com/lambdaclass/ethrex/pull/6331) |

**Finding:** In `regenerate_state` (`l1_committer.rs`), `target_block_number - 1` underflows when target is 0, and always shifts the replay range by one block.

**Resolution:** The function now uses a `match` expression with an explicit `Some(0)` arm that returns `Ok(())` early, preventing the underflow. Documentation was updated to clarify the semantics: `Some(n)` regenerates state up to block `n - 1` (exclusive), `None` regenerates up to the latest block (inclusive).

---

### Issue G: Stateless L1 Validation Omits Transactions Root Check

| Field | Value |
|-------|-------|
| Severity | High |
| Status | **Fixed** |
| PR | [#5608](https://github.com/lambdaclass/ethrex/pull/5608) |

**Finding:** The prover's `stateless_validation_l1` validated block headers without verifying that the header's transactions root commits to the provided block body.

**Resolution:** A new `validate_block_body` function runs in the guest program before `validate_block_pre_execution`. It computes the transactions root from `block_body.transactions` and compares it to `block_header.transactions_root`, and validates the withdrawals root and ommers emptiness. `validate_block` was renamed to `validate_block_pre_execution` for clarity.

---

### Issue H: Incomplete Gas Used Validation Allows Nonzero Gas in Empty Blocks

| Field | Value |
|-------|-------|
| Severity | High |
| Status | **Fixed** |
| PR | [#5996](https://github.com/lambdaclass/ethrex/pull/5996) |

**Finding:** `validate_gas_used` relied on `receipts.last().cumulative_gas_used`. When the receipts list was empty (no transactions), the function returned `Ok(())` without verifying that `block_header.gas_used` was 0.

**Resolution:** As part of the EIP-7778 implementation, `validate_gas_used` was redesigned to take `block_gas_used: u64` directly from the VM execution result, rather than deriving it from receipts. The comparison `block_gas_used != block_header.gas_used` is now unconditional, so empty blocks with nonzero `gas_used` headers are correctly rejected.

---

### Issue I: Missing Domain Separation on Guest Output Digest

| Field | Value |
|-------|-------|
| Severity | High |
| Status | **Not applicable** |

**Finding:** Guest programs (OpenVM, ZisK) hash `ProgramOutput::encode()` without a domain-separation tag, potentially allowing cross-context replay of output digests.

**Response:** The zkVM verification key (`vkey`) already provides domain separation. The `vkey` is a cryptographic commitment to the exact guest program binary, and the on-chain verifier binds each proof to a pinned `vkey` stored in `OnChainProposer`. A replayed output from a different program version would have a different `vkey` and be rejected. Adding an explicit domain tag would be redundant with this binding.

---

### Issue J: Privileged Transaction Failure Path Can Be Bypassed

| Field | Value |
|-------|-------|
| Severity | Medium |
| Status | **Fixed** |
| PR | [#6044](https://github.com/lambdaclass/ethrex/pull/6044) |

**Finding:** Privileged transactions forced to fail via INVALID opcode injection could still execute successfully when the destination was a precompile (which bypasses bytecode interpretation). Additionally, negative `gas_remaining` was cast to `u64` via wrapping conversion, giving an effectively unbounded gas allowance.

**Resolution:** An early check at the top of `run_execution()` in `vm.rs` returns an `OutOfGas` revert when `gas_remaining < 0`, consuming the full gas limit. This fires before the precompile dispatch branch, preventing negative gas from reaching the `as u64` cast. Three regression tests cover the failure and success paths.

---

### Issue K: Privileged Transaction Inclusion Not Guaranteed in ALIGNED_MODE

| Field | Value |
|-------|-------|
| Severity | Medium |
| Status | **Fixed** |
| PR | [#6332](https://github.com/lambdaclass/ethrex/pull/6332) |

**Finding:** `verifyBatchesAligned()` did not enforce the privileged transaction inclusion deadline, unlike `verifyBatch()`.

**Resolution:** The same `ExpiredPrivilegedTransactionDeadline` check was added inside the `verifyBatchesAligned()` per-batch loop, mirroring `verifyBatch()`. Every batch in a multi-batch aligned verification is individually checked.

---

### Issue L: Fee-Token Fees Can Be Locked for Transactions That Fail Validation

| Field | Value |
|-------|-------|
| Severity | Medium |
| Status | **Fixed** |
| PRs | [#6330](https://github.com/lambdaclass/ethrex/pull/6330), [#6333](https://github.com/lambdaclass/ethrex/pull/6333), [#6417](https://github.com/lambdaclass/ethrex/pull/6417) |

**Finding:** Fee-token deduction via `db.get_account_mut` bypassed the call-frame backup mechanism. If later validation failed, the fee-token lock persisted for the rejected transaction.

**Resolution:** The core fix (PR #6330) restructures finalization into two phases (see Issue C). `transfer_fee_token` now backs up changed storage slots via `backup_storage_slot()`, enabling rollback through `restore_cache_state()`. PRs #6333 and #6417 add regression tests verifying that fee-token storage slots revert when validation fails (nonce mismatch, priority fee exceeds max fee).

---

### Issue M: Unvalidated Proof Persistence Halts Liveness

| Field | Value |
|-------|-------|
| Severity | Medium |
| Status | **Not applicable (mitigated at contract level)** |
| PR | [#6334](https://github.com/lambdaclass/ethrex/pull/6334) (documentation) |

**Finding:** The proof coordinator stores submitted proofs without validation, potentially allowing a corrupted proof to be "sticky" and halt rollup progression.

**Response:** On-chain verification handles this. `_verifyBatchInternal` in `OnChainProposer.sol` wraps each verifier call in try/catch -- invalid proofs revert with `InvalidRisc0Proof`, `InvalidSp1Proof`, or `InvalidTdxProof`. `L1ProofSender` has fallback logic (`try_delete_invalid_proof`) that detects invalid proofs from RPC error data, deletes them, and triggers re-proving.

An invalid proof wastes one L1 transaction and causes a delay, but cannot produce an invalid state transition or permanently halt liveness. PR #6334 documents this behavior.

---

### Issue N: L1 Watcher Cursor Advances Before Processing, Dropping Privileged Logs

| Field | Value |
|-------|-------|
| Severity | Medium |
| Status | **Fixed** |
| PR | [#6335](https://github.com/lambdaclass/ethrex/pull/6335) |

**Finding:** `get_logs_l1()` advanced `last_block_fetched_l1` before processing logs. Failed `add_transaction_to_pool` calls were silently skipped via `continue`, permanently dropping those privileged transactions.

**Resolution:** `get_logs_l1()` now returns `(new_cursor, logs)` instead of setting the cursor immediately. The cursor is only assigned after `process_privileged_transactions(logs)` succeeds. Failed `add_transaction_to_pool` calls now propagate the error (`map_err` + `?` instead of `inspect_err` + `continue`), so the batch retries from the same cursor. The `privileged_transaction_already_processed` check prevents duplicates on retry.

---

### Issue O: Unbounded Read and Nonatomic Write in `write_elf_file` Enables Local DoS

| Field | Value |
|-------|-------|
| Severity | Medium |
| Status | **Fixed** |
| PR | [#6441](https://github.com/lambdaclass/ethrex/pull/6441) |

**Finding:** `write_elf_file` reads the entire ELF file and writes it non-atomically via `std::fs::write`. A local attacker with control of `ELF_PATH` could cause resource exhaustion or file corruption.

**Resolution:** PR #6441 replaces the direct write with atomic write-to-temp-then-rename: the ELF is written to `{ELF_PATH}.tmp`, then `std::fs::rename()` swaps it into place. A metadata size check before reading prevents unbounded I/O on tampered files.

---

### Issue P: Block Execution Pre-Rejects Transactions Based on Declared Gas Limit

| Field | Value |
|-------|-------|
| Severity | Medium |
| Status | **Not applicable** |

**Finding:** The executor rejects transactions when `cumulative_gas_used + tx.gas_limit()` exceeds the block gas limit, even if actual gas usage would fit.

**Response:** This matches Ethereum consensus rules. Both the [execution specs](https://github.com/ethereum/execution-specs/blob/b7fe32567fbeb33ec5d2a73b2b28a339c891df51/src/ethereum/forks/prague/fork.py#L449-L453) and geth enforce `cumulative_gas_used + tx.gas_limit <= block.gas_limit` as a block validity condition. The auditor's example of two 30M-limit transactions in a 30M block is also rejected by geth.

---

### Issue Q: Fee-Token Ratio Fetched in Both Prepare and Finalize

| Field | Value |
|-------|-------|
| Severity | Low |
| Status | **Fixed** |
| PR | [#6351](https://github.com/lambdaclass/ethrex/pull/6351) |

**Finding:** `fee_token_ratio` was fetched separately during transaction preparation and finalization. If the ratio changed during execution, lock and settlement amounts would be inconsistent.

**Resolution:** A `cached_fee_token_ratio: Option<U256>` field was added to `L2Hook`. The ratio is fetched once during `prepare_execution_fee_token` and reused in finalization. A regression test deploys a contract that modifies the ratio mid-execution and verifies the cached value is used.

---

### Issue R: Unsanitized Boolean Leads to Nearby-Memory Blake2b Oracle

| Field | Value |
|-------|-------|
| Severity | Low |
| Status | **Fixed** |
| PR | [#6439](https://github.com/lambdaclass/ethrex/pull/6439) |

**Finding:** The x86_64 Blake2b assembly does not sanitize the `f` (finalization flag) parameter. If `f` contains garbage in upper bits, a large offset could cause a read from nearby memory.

**Existing mitigation:** The Rust precompile code (`precompiles.rs:893-897`) validates `f` before calling the assembly. `f` is a Rust `bool` (guaranteed 0 or 1), and the function is `unsafe` and private.

**Resolution:** PR #6439 adds `movzx r8d, r8b` in the assembly to zero-extend the boolean parameter, clearing any garbage in upper bits. Defense-in-depth: the assembly is now safe regardless of caller validation.

---

### Issue S: Zero Length Leads to Buffer Overflow Write in SHA3 Squeeze

| Field | Value |
|-------|-------|
| Severity | Low |
| Status | **Fixed** |
| PR | [#6440](https://github.com/lambdaclass/ethrex/pull/6440) |

**Finding:** `SHA3_squeeze` and `SHA3_squeeze_cext` write out of bounds if the length passed is zero.

**Existing mitigation:** The Rust wrapper handles zero-length at the call site. `SHA3_squeeze` is only called with `len = 32`, and the function is `unsafe` and private.

**Resolution:** PR #6440 adds `cbz` early-exit instructions at the beginning of both `SHA3_squeeze` and `SHA3_squeeze_cext`, branching to the epilogue when output length is zero. Defense-in-depth: the assembly is now safe regardless of caller.

---

### Issue T: Usage of Vulnerable Dependencies

| Field | Value |
|-------|-------|
| Severity | Undetermined |
| Status | **Fixed (5/6), 1 remaining (zero impact)** |
| PR | [#6352](https://github.com/lambdaclass/ethrex/pull/6352) and dependency updates |

**Finding:** Six vulnerable crates were identified: protobuf, ring, rkyv, rsa, ruint, tracing-subscriber.

**Resolution:**

| Crate | Vulnerable Version | Current Version | Status |
|-------|-------------------|-----------------|--------|
| protobuf | 2.28.0 | 3.7.2 | Fixed (PR #6352, prometheus bump) |
| ring | 0.16.20 | 0.17.14 | Fixed (dependency update) |
| rkyv | 0.8.12 | 0.8.14 | Fixed (dependency update) |
| rsa | 0.9.9 | 0.9.10 | Fixed (dependency update) |
| ruint | 1.17.0 | 1.17.2 | Fixed (dependency update) |
| tracing-subscriber | 0.2.25 | 0.2.25 | **Remaining** (transitive) |

**Remaining:** `tracing-subscriber` v0.2.25 is a transitive dependency via `ark-relations` 0.5.1. The advisory concerns ANSI escape injection in logged user input; `ark-relations` only logs static compile-time metadata, never attacker-controlled text. Zero impact. Cannot update without an upstream `ark-relations` release.

---

## Suggestions

### Suggestion 1: Verify That Tokens Match in WithdrawERC20

| Field | Value |
|-------|-------|
| Status | **Fixed** |
| PR | [#6003](https://github.com/lambdaclass/ethrex/pull/6003) |

**Finding:** `withdrawERC20` accepts both `tokenL1` and `tokenL2` but does not verify they correspond. Users could lock their L2 funds with an unclaimable withdrawal proof.

**Resolution:** PR #6003 adds `require(token.l1Address() == tokenL1, "CommonBridgeL2: L1 address mismatch")` to `withdrawERC20`.

---

### Suggestion 2: Add Router-Only Access Control to `receiveETHFromSharedBridge`

| Field | Value |
|-------|-------|
| Status | **Fixed** |
| PR | [#6002](https://github.com/lambdaclass/ethrex/pull/6002) |

**Finding:** `receiveETHFromSharedBridge` accepted ETH from any caller, unlike the ERC-20 counterpart which restricted calls to `SHARED_BRIDGE_ROUTER`.

**Resolution:** Added `require(msg.sender == SHARED_BRIDGE_ROUTER)` to `receiveETHFromSharedBridge`, mirroring the access control in `receiveERC20FromSharedBridge`.

---

### Suggestion 3: Accumulate the Substate Property Instead of Overwriting It

| Field | Value |
|-------|-------|
| Status | **Fixed** |
| PR | [#6037](https://github.com/lambdaclass/ethrex/pull/6037) |

**Finding:** `eip7702_set_access_code` assigned `substate.refunded_gas` directly instead of accumulating, risking silent overwrite if execution order changed.

**Resolution:** Changed from direct assignment to `checked_add` with overflow protection:
```rust
self.substate.refunded_gas = self.substate.refunded_gas
    .checked_add(refunded_gas)
    .ok_or(InternalError::Overflow)?;
```

---

### Suggestion 4: Remove Deprecated "State Diffs" Feature

| Field | Value |
|-------|-------|
| Status | **Fixed** |
| PR | [#5135](https://github.com/lambdaclass/ethrex/pull/5135) |

**Finding:** The prover maintained a `HashMap` of accounts modified by each transaction block, updated in `stateless_validation_l1` and `execute_stateless`, as part of the deprecated "state diffs" feature.

**Resolution:** PR #5135 changed data availability from state diffs to full block commits. The `StateDiff` module (including `modified_accounts: BTreeMap<Address, AccountStateDiff>`) and the `account_updates` HashMap in `StatelessResult` were deleted. Blobs now carry RLP-encoded blocks instead of state diffs.

---

### Suggestion 5: Improve Prover Code Quality

| Field | Value |
|-------|-------|
| Status | **Partially fixed** |
| PR | [#6355](https://github.com/lambdaclass/ethrex/pull/6355) |

**Finding:** Three code quality items: (1) incorrect error message in `request_new_input`, (2) redundant l2 feature flag check, (3) comment mismatch in `to_calldata`.

**Resolution:** PR #6355 corrects the error messages (item 1). The `l2` feature check (item 2) is intentional -- it guards against misconfiguration where the prover binary is built without L2 support. Item 3 was not found in the current code (likely resolved in a prior refactor).

---

### Suggestion 6: Improve Test Coverage

| Field | Value |
|-------|-------|
| Status | **Acknowledged** |

Ongoing effort. The audit fix PRs include regression tests for the found vulnerabilities. We continue expanding coverage, particularly for VM edge cases and contract invariants.

---

### Suggestion 7: Improve Code Comments

| Field | Value |
|-------|-------|
| Status | **Acknowledged** |

Ongoing effort; too broad to attribute to a single PR.

---

### Suggestion 8: Improve Blake2b Implementation

| Field | Value |
|-------|-------|
| Status | **Not applicable** |

**Finding:** Three items: (1) clamp rounds `r` to 0..12, (2) sanitize `r=0` edge case, (3) zeroize shuffle buffer for keyed mode.

**Response:**

1. **Rounds clamping:** The rounds parameter `r` can be arbitrarily large in the Ethereum Blake2 precompile ([EIP-152](https://eips.ethereum.org/EIPS/eip-152)). The gas cost scales linearly with `r` (`rounds * 1 gas`), and `r` is parsed as a `u32` from calldata. Clamping to 0..12 would break spec compliance.
2. **`r=0` handling:** The assembly handles `r=0` correctly: `sub rdi, 0x01` wraps, the carry flag jumps to exit, skipping all rounds. The final merge still executes, which is correct (Blake2 with 0 rounds still performs initialization and finalization).
3. **Shuffle buffer zeroization:** We do not use Blake2b in keyed mode. The Ethereum precompile takes `h`, `m`, `t`, `f`, and `rounds` as inputs -- there is no key material. The first block never contains a key because keyed Blake2b is not part of the EIP-152 interface.
