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
- Return: `abi.encode(postStateRoot, blockNumber, withdrawalRoot, gasUsed, burnedFees)` — 160 bytes

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

## Gap 5: L2 Fee Market — Burned Fee Tracking & ETH Supply Drain — IMPLEMENTED

**Status:** Implemented
**Spec:** Expose burned fees in block output for L1 bridge crediting

### Problem

The L2Bridge has a finite preminted ETH balance. Every L2 transaction burns base fees (standard EIP-1559), permanently removing ETH from circulation. Since there is no minting mechanism on L2, the total supply decreases over time. Deposits cannot replenish this because `processL1Message()` transfers ETH from the bridge's own balance — it redistributes, not creates.

### After Implementation

The EXECUTE precompile now computes `burnedFees = base_fee_per_gas * block_gas_used` after block execution and returns it as a 5th uint256 slot in the return value (160 bytes total). The NativeRollup contract on L1 decodes this value and sends it to `msg.sender` (the relayer) when `advance()` is called, effectively crediting the relayer on L1 for burned L2 fees.

A separate L2 process (out of scope for this PoC) would be responsible for crediting the relayer on L2. A production solution would redirect burned fees to the bridge contract on L2 (similar to the OP Stack's `BaseFeeVault`), keeping total L2 supply constant.

### Files Changed

- Modified: `crates/vm/levm/src/execute_precompile.rs` — compute `burnedFees`, expand return from 128 to 160 bytes
- Modified: `crates/vm/levm/contracts/NativeRollup.sol` — decode `burnedFees`, send to relayer, add to `StateAdvanced` event
- Modified: `test/tests/levm/native_rollups.rs` — verify 160-byte return, check burnedFees value, verify relayer receives ETH
- Modified: `docs/vm/levm/native_rollups.md` — update return value docs, spec comparison table, limitations

---

## Gap 6: EXECUTE Variant — Individual Field Inputs — IMPLEMENTED

**Status:** Implemented
**Spec (`apply_body` variant):** Individual fields (number, pre/post state, receipts, gas_limit, coinbase, prev_randao, transactions, parent gas info)

### After Implementation

The EXECUTE precompile now uses the `apply_body` variant with individual field inputs instead of a full RLP-encoded block. The ABI format has 14 slots (12 static + 2 dynamic offset pointers):

- Slots 0-2: `preStateRoot`, `postStateRoot`, `postReceiptsRoot` (bytes32)
- Slots 3-4: `blockNumber`, `blockGasLimit` (uint256)
- Slot 5: `coinbase` (address, ABI-padded to 32 bytes)
- Slot 6: `prevRandao` (bytes32)
- Slots 7-10: `timestamp`, `parentBaseFee`, `parentGasLimit`, `parentGasUsed` (uint256)
- Slot 11: `l1MessagesRollingHash` (bytes32)
- Slots 12-13: `transactions` (RLP list), `witnessJson` (dynamic offset pointers)

The precompile computes the base fee from explicit parent fields (`parentBaseFee`, `parentGasLimit`, `parentGasUsed`) using EIP-1559 `calculate_base_fee_per_gas`, builds a synthetic block header from the individual fields, and executes the block. No RLP block decoding is needed.

NativeRollup.sol uses a `BlockParams` struct to group block parameters and builds the 14-slot calldata via `abi.encode()`. The contract was compiled with `solc --via-ir --optimize` to handle the stack depth.

### Files Changed

- Modified: `crates/vm/levm/src/execute_precompile.rs` — replaced `ExecutePrecompileInput` struct (individual fields instead of `Block`), rewrote `parse_abi_calldata` (14-slot ABI), added `decode_transactions_from_rlp` and `read_u64_slot` helpers, rewrote `execute_inner` (synthetic header construction, base fee computation from parent fields)
- Modified: `crates/vm/levm/contracts/NativeRollup.sol` — added `BlockParams` struct, updated `advance()` signature and calldata construction
- Modified: `test/tests/levm/native_rollups.rs` — updated `build_precompile_calldata` (14-slot), `build_l2_state_transition_with_sender`, `encode_advance_call` (BlockParams struct), removed withdrawal rejection test
- Modified: `test/tests/l2/native_rollups.rs` — updated `build_l2_state_transition`, `build_l2_withdrawal_block`, advance call encoding
- Modified: `docs/vm/levm/native_rollups.md` — updated architecture, calldata format, encoding details, spec comparison

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
| 2 | Gap 5: Burned fees | Low | Medium | **Done** |
| 3 | Gap 6: Individual fields | Medium-High | Medium | **Done** |
| 4 | Gap 7: Finality delay | Low | Low | Pending |

Gaps 1-6 are implemented. Gap 7 is independent and can be done next.
