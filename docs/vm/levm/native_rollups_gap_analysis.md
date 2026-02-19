# Native Rollups: Gap Analysis & Implementation Plan

## Context

Our PoC implements the EXECUTE precompile from the [L2Beat native rollups book](https://native-rollups.l2beat.com/) (EIP-8079). The core execution flow — state root validation, receipts root validation, transaction filtering, base fee verification, stateless execution — is aligned with the spec. However, the surrounding infrastructure (anchoring, messaging, L1→L2 messages, fee tracking) diverges from the book's proposed design. This document analyzes each gap and proposes a plan to close it.

## Gaps 1-4 (Unified): L1 Message & Messaging Redesign — IMPLEMENTED

Gaps 1 (L1 anchoring), 2 (L1→L2 messaging), 3 (gas token deposits), and 4 (L2→L1 messaging) are deeply coupled. We redesigned them together, reusing the **rolling hash pattern from CommonBridge + OnChainProposer** (our existing L2 stack).

### Current State (After Implementation)

- **L1 NativeRollup.sol**: Stores L1 message hashes (`keccak256(abi.encodePacked(from, to, value, gasLimit, keccak256(data), nonce))` — 168 bytes preimage), computes rolling hash over consumed L1 messages, passes it as `bytes32` to the EXECUTE precompile
- **L2 L2Bridge.sol**: Unified predeploy at `0x00...fffd` with preminted ETH. Relayer calls `processL1Message()` to execute arbitrary L2 calls (ETH transfers, contract calls with calldata). Users call `withdraw()` to burn ETH and emit withdrawal events.
- **EXECUTE precompile**: Verifies L1 messages rolling hash by scanning `L1MessageProcessed` events from L2Bridge. No more direct state manipulation or l1Anchor parsing.
- Withdrawals: unchanged (event scanning + Merkle tree)
- Supports general-purpose L1→L2 messaging with arbitrary calldata (not just ETH transfers)

### Book's Recommendations

- **L1 Anchoring**: `L1_ANCHOR` system contract on L2, receives `bytes32` via system transaction
- **L1→L2 Messaging**: No custom tx types; messages claimed via inclusion proofs against anchored hash
- **Gas Token Deposits**: Preminted tokens in L2 predeploy, unlocked when messages claimed
- **L2→L1 Messaging**: State root / receipts root proofs against L2 state

### New Design

The key insight: the sequencer already monitors L1 and builds L2 blocks. It can include L1 message-processing transactions as regular signed transactions — no custom tx types, no system transactions needed. The "first deposit problem" (users need gas to claim deposits) is solved because the **sequencer pays gas** for the L1 message tx, not the user.

**L1 — NativeRollup.sol (reusing CommonBridge patterns):**

```solidity
// L1 message tracking (adapted from CommonBridge)
bytes32[] public pendingL1Messages;      // hash queue
uint256 public l1MessageIndex;           // next message to process
uint256 constant DEFAULT_GAS_LIMIT = 100_000;

function sendL1Message(address _to, uint256 _gasLimit, bytes calldata _data) external payable {
    _recordL1Message(msg.sender, _to, msg.value, _gasLimit, _data);
}

receive() external payable {
    _recordL1Message(msg.sender, msg.sender, msg.value, DEFAULT_GAS_LIMIT, "");
}

function _recordL1Message(address from, address to, uint256 value, uint256 gasLimit, bytes memory data) internal {
    bytes32 hash = keccak256(abi.encodePacked(from, to, value, gasLimit, keccak256(data), l1MessageIndex));
    pendingL1Messages.push(hash);
    l1MessageIndex++;
}

function advance(uint256 l1MessagesCount, bytes calldata _block, bytes calldata _witness) external {
    // 1. Compute rolling hash over consumed L1 messages
    bytes32 l1MessagesRollingHash = bytes32(0);
    for (uint i = 0; i < l1MessagesCount; i++) {
        l1MessagesRollingHash = keccak256(abi.encodePacked(l1MessagesRollingHash, pendingL1Messages[i]));
    }

    // 2. Call EXECUTE — passes rolling hash for L2-side verification
    (bool ok, bytes memory result) = address(0x0101).staticcall(
        abi.encode(stateRoot, l1MessagesRollingHash, _block, _witness)
    );

    // 3. Remove processed L1 messages from queue
    removeProcessedL1Messages(l1MessagesCount);

    // 4. Store new state root, block number, withdrawal root
}
```

**L2 — L2Bridge predeploy (at `0x00...fffd`, preminted ETH in genesis):**

```solidity
contract L2Bridge {
    address public relayer;  // permissioned for PoC

    function processL1Message(
        address from, address to, uint256 value, uint256 gasLimit,
        bytes calldata data, uint256 nonce
    ) external {
        require(msg.sender == relayer);
        require(nonce == l1MessageNonce);
        l1MessageNonce++;
        to.call{value: value, gas: gasLimit}(data);
        emit L1MessageProcessed(from, to, value, gasLimit, keccak256(data), nonce);
    }
}
```

**L2 — Withdrawals (unchanged):**
- Keep current approach: `withdraw(receiver)` emits `WithdrawalInitiated` event
- Precompile scans events and builds Merkle tree
- Storage-based message passer (OP Stack style) can be considered later

**EXECUTE precompile (simplified):**
- **Remove** `apply_deposits()` — no more direct state manipulation
- **Remove** l1Anchor deposit parsing — no deposit data in calldata
- **Simplify** ABI to: `abi.encode(bytes32 preStateRoot, bytes32 l1MessagesRollingHash, bytes blockRlp, bytes witnessJson)`
- **Verify** L1 messages by scanning `L1MessageProcessed` events from L2Bridge and computing rolling hash
- Keep: state root verification, receipts root verification, withdrawal event scanning
- Return: `abi.encode(postStateRoot, blockNumber, withdrawalRoot, gasUsed)` — 128 bytes (unchanged)

**Sequencer (relayer):**
- Monitors L1 for pending L1 messages (like L1Watcher in existing stack)
- Includes a signed L1 message-processing tx as the **first** tx in each L2 block
- Sequencer address prefunded in L2 genesis
- Calls `L2Bridge.processL1Message(from, to, value, gasLimit, data, nonce)`

### L1 Message Enforcement (from CommonBridge/OnChainProposer)

| Mechanism | Source | How it applies |
|-----------|--------|----------------|
| **Rolling hash** | `CommonBridge.getPendingTransactionsVersionedHash()` | L1 NativeRollup computes rolling hash over consumed L1 messages, EXECUTE verifies it on L2 |
| **Forced inclusion** | `CommonBridge.hasExpiredPrivilegedTransactions()` | Optional: block `advance()` if L1 messages exceed deadline |
| **FIFO ordering** | `CommonBridge.pendingTxIndex` | L1 messages must be processed in order |
| **State root check** | EXECUTE precompile | Block (including L1 message tx) must produce correct state root |

### Trust Model

- The sequencer is trusted to include L1 message txs (same as all centralized-sequencer rollups)
- The L1 rolling hash check ensures the sequencer can't lie about WHICH L1 messages were processed
- EXECUTE ensures the block was validly executed (state root captures L1 message effects)
- Optional forced inclusion prevents indefinite L1 message censorship
- Full trust-minimization (receipt proofs, anchor-based verification) is a Phase 2+ concern

### What This Achieved

| Book requirement | Before | After (Implemented) |
|-----------------|--------|-------|
| No custom transaction types | N/A (direct state crediting) | Regular txs from relayer ✓ |
| Preminted gas token contract | Direct ETH crediting | L2Bridge with preminted ETH ✓ |
| L1 message enforcement | None (trust precompile) | Rolling hash verification ✓ |
| First deposit problem | Avoided (direct crediting) | Solved (relayer pays gas) ✓ |
| EXECUTE simplicity | Handles deposits internally | Verifies rolling hash only ✓ |
| General L1→L2 messaging | ETH-only deposits | Arbitrary calldata support ✓ |
| Forced inclusion | Not implemented | Not implemented (future work) |

### Files (Changed)

| File | Change |
|------|--------|
| `crates/vm/levm/contracts/NativeRollup.sol` | Rewritten: `sendL1Message()` with 168-byte hash, rolling hash in `advance()` |
| `crates/vm/levm/contracts/L2Bridge.sol` | New: unified L2 predeploy (L1 messages + withdrawals) with preminted ETH, `processL1Message()` executes arbitrary L2 calls |
| `crates/vm/levm/src/execute_precompile.rs` | Rewritten: removed `apply_deposits()`, simplified ABI, added rolling hash verification via `L1MessageProcessed` events |
| `test/tests/levm/native_rollups.rs` | Rewritten: LEVM-based execution with relayer + L2Bridge |
| `test/tests/l2/native_rollups.rs` | Rewritten: LEVM-based execution with relayer + L2Bridge |

---

## Gap 5: L2 Fee Market — Burned Fee Tracking & ETH Supply Drain

**Status:** Partial
**Current:** Base fee verified, but burned fees not tracked/exposed. L2 ETH supply slowly drains as EIP-1559 base fees are burned on every transaction.
**Spec:** Expose burned fees in block output for L1 bridge crediting

### Problem

The L2Bridge has a finite preminted ETH balance. Every L2 transaction burns base fees (standard EIP-1559), permanently removing ETH from circulation. Since there is no minting mechanism on L2, the total supply decreases over time. Deposits cannot replenish this because `processL1Message()` transfers ETH from the bridge's own balance — it redistributes, not creates.

### Possible Solutions

1. **Configure fee destination in the EVM** (recommended) — redirect burned base fees to the L2Bridge contract instead of destroying them, similar to the OP Stack's `BaseFeeVault` predeploy. This keeps total L2 supply constant without changing EVM opcode semantics.
2. **Post-block bridge crediting** — after standard EVM execution, both the sequencer and the EXECUTE precompile apply a post-processing step crediting the bridge with `sum(base_fee * gas_used)`. The EVM itself is untouched, but the block state transition has an extra step.
3. **Over-premint** — premint enough ETH that drain is negligible for the PoC lifetime. Does not solve the problem, only delays it.

### Plan (Burned Fee Tracking)

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
