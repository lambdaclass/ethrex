# Native Rollups: Gap Analysis & Implementation Plan

## Context

Our PoC implements the EXECUTE precompile from the [L2Beat native rollups book](https://native-rollups.l2beat.com/) (EIP-8079). The core execution flow — state root validation, receipts root validation, transaction filtering, base fee verification, stateless execution — is aligned with the spec. However, the surrounding infrastructure (anchoring, messaging, deposits, fee tracking) diverges from the book's proposed design. This document analyzes each gap and proposes a plan to close it.

## Gaps 1-4 (Unified): Deposit & Messaging Redesign — IMPLEMENTED

Gaps 1 (L1 anchoring), 2 (L1→L2 messaging), 3 (gas token deposits), and 4 (L2→L1 messaging) are deeply coupled. We redesigned them together, reusing the **rolling hash pattern from CommonBridge + OnChainProposer** (our existing L2 stack).

### Current State (After Implementation)

- **L1 NativeRollup.sol**: Stores deposit hashes (`keccak256(abi.encodePacked(recipient, amount, nonce))`), computes rolling hash over consumed deposits, passes it as `bytes32` to the EXECUTE precompile
- **L2 L2Bridge.sol**: Unified predeploy at `0x00...fffd` with preminted ETH. Relayer calls `processDeposit()` to transfer ETH to recipients. Users call `withdraw()` to burn ETH and emit withdrawal events.
- **EXECUTE precompile**: Verifies deposits rolling hash by scanning `DepositProcessed` events from L2Bridge. No more direct state manipulation or l1Anchor parsing.
- Withdrawals: unchanged (event scanning + Merkle tree)
- No general-purpose L1→L2 messaging beyond ETH deposits (future work)

### Book's Recommendations

- **L1 Anchoring**: `L1_ANCHOR` system contract on L2, receives `bytes32` via system transaction
- **L1→L2 Messaging**: No custom tx types; messages claimed via inclusion proofs against anchored hash
- **Gas Token Deposits**: Preminted tokens in L2 predeploy, unlocked when messages claimed
- **L2→L1 Messaging**: State root / receipts root proofs against L2 state

### New Design

The key insight: the sequencer already monitors L1 and builds L2 blocks. It can include deposit-processing transactions as regular signed transactions — no custom tx types, no system transactions needed. The "first deposit problem" (users need gas to claim deposits) is solved because the **sequencer pays gas** for the deposit tx, not the user.

**L1 — NativeRollup.sol (reusing CommonBridge patterns):**

```solidity
// Deposit tracking (adapted from CommonBridge)
bytes32[] public pendingDeposits;      // hash queue
uint256 public depositIndex;           // next deposit to process

function deposit(address recipient) external payable {
    bytes32 hash = keccak256(abi.encodePacked(recipient, msg.value, depositIndex));
    pendingDeposits.push(hash);
    depositIndex++;
}

function getDepositsVersionedHash(uint16 count) public view returns (bytes32) {
    // Same pattern as CommonBridge.getPendingTransactionsVersionedHash()
    // Returns (count || rolling_hash_of_first_N_deposits)
}

function advance(uint256 depositsProcessed, bytes calldata _block, bytes calldata _witness) external {
    // 1. Verify deposit inclusion (like OnChainProposer.commitBatch)
    if (depositsProcessed > 0) {
        bytes32 claimed = getDepositsVersionedHash(uint16(depositsProcessed));
        // Verify matches expectation
    }

    // 2. Call EXECUTE — NO deposit data in calldata, just block + witness
    (bool ok, bytes memory result) = address(0x0101).staticcall(
        abi.encode(stateRoot, _block, _witness)
    );

    // 3. Remove processed deposits from queue
    removeProcessedDeposits(depositsProcessed);

    // 4. Store new state root, block number, withdrawal root
}
```

**L2 — DepositBridge predeploy (at `0x00...fffe`, preminted ETH in genesis):**

```solidity
contract DepositBridge {
    address public sequencer;  // permissioned for PoC

    function processDeposits(address[] calldata recipients, uint256[] calldata amounts) external {
        require(msg.sender == sequencer);
        for (uint i = 0; i < recipients.length; i++) {
            (bool ok,) = recipients[i].call{value: amounts[i]}("");
            require(ok);
        }
    }
}
```

**L2 — L2WithdrawalBridge (unchanged):**
- Keep current approach: `withdraw(receiver)` emits `WithdrawalInitiated` event
- Precompile scans events and builds Merkle tree
- Storage-based message passer (OP Stack style) can be considered later

**EXECUTE precompile (simplified):**
- **Remove** `apply_deposits()` — no more direct state manipulation
- **Remove** l1Anchor deposit parsing — no deposit data in calldata
- **Simplify** ABI to: `abi.encode(bytes32 preStateRoot, bytes blockRlp, bytes witnessJson)`
- Keep: state root verification, receipts root verification, withdrawal event scanning
- Return: `abi.encode(postStateRoot, blockNumber, withdrawalRoot, gasUsed)` — 128 bytes (unchanged)

**Sequencer (relayer):**
- Monitors L1 for pending deposits (like L1Watcher in existing stack)
- Includes a signed deposit-processing tx as the **first** tx in each L2 block
- Sequencer address prefunded in L2 genesis
- Calls `DepositBridge.processDeposits(recipients, amounts)`

### Deposit Enforcement (from CommonBridge/OnChainProposer)

| Mechanism | Source | How it applies |
|-----------|--------|----------------|
| **Versioned hash** | `CommonBridge.getPendingTransactionsVersionedHash()` | L1 NativeRollup verifies `depositsProcessed` against pending queue |
| **Forced inclusion** | `CommonBridge.hasExpiredPrivilegedTransactions()` | Optional: block `advance()` if deposits exceed deadline |
| **FIFO ordering** | `CommonBridge.pendingTxIndex` | Deposits must be processed in order |
| **State root check** | EXECUTE precompile | Block (including deposit tx) must produce correct state root |

### Trust Model

- The sequencer is trusted to include deposit txs (same as all centralized-sequencer rollups)
- The L1 versioned hash check ensures the sequencer can't lie about WHICH deposits were processed
- EXECUTE ensures the block was validly executed (state root captures deposit effects)
- Optional forced inclusion prevents indefinite deposit censorship
- Full trust-minimization (receipt proofs, anchor-based verification) is a Phase 2+ concern

### What This Achieved

| Book requirement | Before | After (Implemented) |
|-----------------|--------|-------|
| No custom transaction types | N/A (direct state crediting) | Regular txs from relayer ✓ |
| Preminted gas token contract | Direct ETH crediting | L2Bridge with preminted ETH ✓ |
| L1 deposit enforcement | None (trust precompile) | Rolling hash verification ✓ |
| First deposit problem | Avoided (direct crediting) | Solved (relayer pays gas) ✓ |
| EXECUTE simplicity | Handles deposits internally | Verifies rolling hash only ✓ |
| Forced inclusion | Not implemented | Not implemented (future work) |

### Files (Changed)

| File | Change |
|------|--------|
| `crates/vm/levm/contracts/NativeRollup.sol` | Rewritten: deposit hashes with rolling hash in `advance()` |
| `crates/vm/levm/contracts/L2Bridge.sol` | New: unified L2 predeploy (deposits + withdrawals) with preminted ETH |
| `crates/vm/levm/src/execute_precompile.rs` | Rewritten: removed `apply_deposits()`, simplified ABI, added rolling hash verification via DepositProcessed events |
| `test/tests/levm/native_rollups.rs` | Rewritten: LEVM-based execution with relayer + L2Bridge |
| `test/tests/l2/native_rollups.rs` | Rewritten: LEVM-based execution with relayer + L2Bridge |

---

## Gap 5: L2 Fee Market — Burned Fee Tracking

**Status:** Partial
**Current:** Base fee verified, but burned fees not tracked/exposed
**Spec:** Expose burned fees in block output for L1 bridge crediting

### Plan

1. After block execution, compute `burned_fees` = sum over all txs of `base_fee_per_gas * gas_used_after_refund`.
2. Add `burnedFees` to the precompile return value (160 bytes total).
3. In `NativeRollup.sol`, decode and store the burned fees.

### Files

- Modified: `crates/vm/levm/src/execute_precompile.rs`
- Modified: `crates/vm/levm/contracts/NativeRollup.sol`
- Modified: tests

---

## Gap 6: EXECUTE Variant — Individual Field Inputs

**Status:** Partial
**Current:** Full RLP block + witness JSON
**Spec (`apply_body` variant):** Individual fields (chain_id, number, pre/post state, receipts, gas_limit, coinbase, prev_randao, transactions, parent gas info)

### Plan

1. Change ABI encoding to individual fields instead of RLP block.
2. Precompile receives fields directly — no RLP block decode.
3. `NativeRollup.sol` constructs these fields from its state.

### Files

- Modified: `crates/vm/levm/src/execute_precompile.rs`
- Modified: `crates/vm/levm/contracts/NativeRollup.sol`
- Modified: tests

---

## Gap 7: Withdrawal Finality Delay

**Status:** Gap
**Current:** Withdrawals claimable immediately
**Spec:** Implied requirement for production

### Plan

1. Add `FINALITY_DELAY` constant to `NativeRollup.sol`.
2. In `claimWithdrawal`, require `block.number >= withdrawalBlockNumber + FINALITY_DELAY`.

### Files

- Modified: `crates/vm/levm/contracts/NativeRollup.sol`
- Modified: tests

---

## Not Addressable / OK as-is

| Gap | Why it's OK |
|-----|-------------|
| **Forced transactions** | Spec is WIP. No concrete design to implement yet. |
| **Gas metering** | Spec says TBD. Flat cost is the right placeholder. |
| **Blob data support** | Requires L2 blob infrastructure we don't have. |
| **Serialization (blob refs)** | Same — blob references need blob support. |

---

## Recommended Implementation Order

| Order | Gap | Effort | Impact | Status |
|-------|-----|--------|--------|--------|
| 1 | Gaps 1-4 (unified) | High | High — foundational redesign | **Done** |
| 2 | Gap 5: Burned fees | Low | Medium | Pending |
| 3 | Gap 7: Finality delay | Low | Low | Pending |
| 4 | Gap 6: Individual fields | Medium-High | Medium | Pending |

Gaps 1-4 are implemented. Gaps 5, 6, 7 are independent and can be done in any order.
