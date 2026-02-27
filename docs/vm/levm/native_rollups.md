# Native Rollups: EXECUTE Precompile (EIP-8079 PoC)

## Overview

[EIP-8079](https://github.com/ethereum/EIPs/pull/9608) proposes "native rollups" — a mechanism where L1 verifies L2 state transitions by re-executing them inside the EVM via an `EXECUTE` precompile. This replaces complex proof systems (zkVM/fraud proofs) with direct execution, leveraging the fact that L1 already has an EVM capable of running the same transactions.

This is a Phase 1 proof-of-concept implementing the `apply_body` variant.

```
Individual Block Fields + Transactions (RLP) + ExecutionWitness (JSON) + L1 Anchor (Merkle root)
        |
  EXECUTE precompile (in LEVM) — apply_body variant
        |
  1. Parse ABI-encoded calldata (15 slots: 13 static fields + 2 dynamic pointers)
  2. Build GuestProgramState from witness
  3. Verify pre-state root
  4. Compute base fee from explicit parent fields (EIP-1559)
  5. Write l1Anchor to L1Anchor predeploy storage (system transaction)
  6. Build synthetic block header from individual fields
  7. Execute transactions via LEVM, collect logs and receipts
  8. Verify post-state root matches computed root
  9. Verify receipts root matches computed receipts
        |
  Returns abi.encode(postStateRoot, blockNumber, gasUsed, burnedFees, baseFeePerGas) or reverts
```

## EXECUTE Precompile

The [L2Beat native rollups book](https://native-rollups.l2beat.com/) defines two variants: `apply_body` (individual fields, skips header validation) and `state_transition` (full headers, complete STF). We implement `apply_body` — it receives individual block fields, skips full header validation (no parent hash chain, no timestamp ordering, no ommers hash check), and re-executes transactions to verify the resulting state root and receipts root.

The core logic lives in `execute_precompile.rs`. It parses ABI-encoded calldata, computes the base fee from explicit parent fields (EIP-1559), writes the l1Anchor to the L1Anchor predeploy's storage slot 0 (system transaction — step 5 of `apply_body`), builds a synthetic block header from individual fields, and orchestrates the full verification flow: state root checks, block execution, and receipts root verification.

### Calldata Format

```
abi.encode(
    uint256 chainId,                // slot 0  (static)
    bytes32 preStateRoot,           // slot 1  (static)
    bytes32 postStateRoot,          // slot 2  (static)
    bytes32 postReceiptsRoot,       // slot 3  (static)
    uint256 blockNumber,            // slot 4  (static)
    uint256 blockGasLimit,          // slot 5  (static)
    address coinbase,               // slot 6  (static, ABI-padded to 32 bytes)
    bytes32 prevRandao,             // slot 7  (static)
    uint256 timestamp,              // slot 8  (static)
    uint256 parentBaseFee,          // slot 9  (static)
    uint256 parentGasLimit,         // slot 10 (static)
    uint256 parentGasUsed,          // slot 11 (static)
    bytes32 l1Anchor,               // slot 12 (static — Merkle root of consumed L1 messages)
    bytes   transactions,           // slot 13 (dynamic -- offset pointer)
    bytes   witnessJson             // slot 14 (dynamic -- offset pointer)
)

Head: 15 x 32 = 480 bytes (slots 0-12 static, slots 13-14 dynamic offset pointers)
Tail: transactions RLP data, witness JSON data (each prefixed with 32-byte length)
```

The base fee is computed from explicit parent fields (`parentBaseFee`, `parentGasLimit`, `parentGasUsed`) using `calculate_base_fee_per_gas` (EIP-1559). A synthetic block header is built internally from the individual fields for block execution.

### Return Value

```
abi.encode(bytes32 postStateRoot, uint256 blockNumber, uint256 gasUsed, uint256 burnedFees, uint256 baseFeePerGas)  — 160 bytes
```

- **postStateRoot** — verified against the computed state root after execution
- **blockNumber** — extracted from `block.header.number`
- **gasUsed** — cumulative gas consumed by all transactions (pre-refund, matching `block.header.gas_used`)
- **burnedFees** — `base_fee_per_gas * block_gas_used` (total EIP-1559 base fees burned). The NativeRollup contract on L1 sends this to the relayer
- **baseFeePerGas** — the computed EIP-1559 base fee for the executed block, returned so the L1 contract can store it on-chain for subsequent blocks

No withdrawal root is returned — withdrawals are proven directly against the post-state root via MPT proofs on L1.

### Encoding Details

- **Transactions** use RLP list encoding (`Vec::<Transaction>::encode_to_vec()` / `Vec::<Transaction>::decode()`)
- **ExecutionWitness** uses JSON because it doesn't have RLP support — it uses serde/rkyv for serialization instead
- **Block fields** are individual ABI-encoded slots (bytes32, uint256, address)
- **L1 anchor** is a static `bytes32` — the Merkle root over consumed L1 message hashes, computed by the L1 NativeRollup contract

The NativeRollup contract fills in `preStateRoot` from its own storage, reads `blockNumber + 1`, `blockGasLimit`, `lastBaseFeePerGas`, and `lastGasUsed` from storage, passes the remaining block parameters via a 5-field `BlockParams` struct, computes the `l1Anchor` Merkle root from its pending L1 message queue, and forwards transactions/witness bytes unchanged (opaque to the contract). The contract decodes the precompile's 160-byte return value to extract the new state root, block number, gas used, burned fees, and base fee per gas.

### Transaction Type Validation

Native rollup blocks only allow standard L1 transaction types. The following are rejected before sender recovery (cheap check first):

- **EIP-4844 blob transactions** — blob data doesn't exist on L2
- **Privileged L2 transactions** — ethrex-specific L2 type, not valid in native rollups
- **Fee token transactions** — ethrex-specific L2 type, not valid in native rollups

Legacy, EIP-2930, EIP-1559, and EIP-7702 transactions are allowed.

> **Gap with book:** The spec references transactions via blob hashes; we embed them in the ABI calldata as an RLP-encoded list. The spec's output format is still TBD — we defined our own 160-byte return. Gas metering is a flat 100,000 gas placeholder (`EXECUTE_GAS_COST`); the book's metering is also TBD.

## L1 Anchoring

The L1Anchor predeploy lives at `0x00...fffe` with a single `bytes32 public l1MessagesRoot` at storage slot 0. No setter function — the EXECUTE precompile writes directly to slot 0 before executing regular transactions (system transaction — step 4 of `apply_body` in the book). The L2Bridge reads from this contract to verify Merkle proofs.

> **Gap with book:** The book proposes an `L1_ANCHOR` system contract that receives an arbitrary `bytes32` (typically an L1 block hash). Our `l1Anchor` is specifically an L1 messages Merkle root, not a generic `bytes32`. A production implementation could anchor an L1 block hash instead, enabling broader cross-chain proofs beyond L1 messages.

## L1 to L2 Messaging

The L1→L2 message flow uses a relayer, a prefunded L2 bridge contract, and Merkle proof verification against an anchored L1 messages root, following the book's recommended proof-based pattern (similar to Linea/Taiko):

1. Users call `NativeRollup.sendL1Message(to, gasLimit, data)` on L1 with ETH value — this records `keccak256(abi.encodePacked(from, to, value, gasLimit, keccak256(data), nonce))` (168-byte preimage) as an L1 message hash in the contract's `pendingL1Messages` array
2. When `advance()` is called with `_l1MessagesCount`, it computes a **Merkle root** over the consumed L1 message hashes (commutative Keccak256, OpenZeppelin-compatible)
3. The Merkle root is passed to the EXECUTE precompile as the `l1Anchor` parameter (static `bytes32`)
4. The EXECUTE precompile writes the `l1Anchor` to the **L1Anchor predeploy** (`0x00...fffe`) storage slot 0 before executing regular transactions (system transaction)
5. On L2, a relayer sends real transactions calling `L2Bridge.processL1Message(from, to, value, gasLimit, data, nonce, merkleProof)`, which verifies the Merkle inclusion proof against the anchored root in L1Anchor, then executes `to.call{value: value, gas: gasLimit}(data)` (transferring ETH and/or executing calldata) and emits `L1MessageProcessed` events
6. The **state root check** at the end of execution implicitly guarantees correct message processing (if claims aren't included or are wrong, the post-state root won't match)

**Block builder constraint:** The relayer chooses how many L1 messages to consume (`_l1MessagesCount` can be 0). The Merkle root over those messages is computed in `advance()` on L1 and anchored in the L1Anchor predeploy *before* the block transactions execute. This means the block builder must know the Merkle root at block construction time and include the matching `processL1Message()` transactions — if they don't match, the L2Bridge proof verification will produce a different state root and the EXECUTE precompile will revert.

The relayer pays gas for L1 message transactions, solving the "first deposit problem" (which the book lists as unresolved). L1 messages support arbitrary calldata, enabling not just ETH transfers but also arbitrary contract calls on L2.

> **Gap with book:** None — aligned with the book's Linea/Taiko-style proof-based messaging recommendation. We're ahead on the first deposit problem.

## L2 to L1 Messaging (Withdrawals)

Withdrawals allow users to move ETH from L2 back to L1. The flow uses **state root proofs** — since the EXECUTE precompile already exposes the post-state root, L2 contract storage can be proven directly against it, eliminating custom data structures from the precompile:

```
L2: User calls L2Bridge.withdraw(receiverOnL1) with ETH
     → keeps ETH locked in the bridge contract
     → writes sentMessages[keccak256(abi.encodePacked(from, receiver, amount, messageId))] = true
     → emits WithdrawalInitiated(from, receiver, amount, messageId)

EXECUTE precompile: Executes block and returns post-state root
     → no event scanning or custom Merkle tree needed
     → the state root captures everything, including L2Bridge's sentMessages storage

L1: NativeRollup.advance() stores stateRootHistory[blockNumber] = newStateRoot

L1: User calls NativeRollup.claimWithdrawal(from, receiver, amount, messageId, blockNumber, accountProof, storageProof)
     → looks up stateRootHistory[blockNumber] as the L2 state root
     → checks that block.timestamp >= stateRootTimestamps[blockNumber] + FINALITY_DELAY
     → account proof: state root → L2Bridge account → extracts storageRoot
     → storage proof: storageRoot → sentMessages[withdrawalHash] == true
     → marks withdrawal as claimed (prevents double-claiming)
     → transfers ETH to receiver
```

Each withdrawal is uniquely identified by `keccak256(abi.encodePacked(from, receiver, amount, messageId))`. The `messageId` is a counter maintained by the L2Bridge contract (`withdrawalNonce`), starting at 0 and incrementing per withdrawal. The MPT proof verification lives in a separate `MPTProof.sol` library (trie traversal, RLP decoding, account/storage proof verification), which NativeRollup.sol imports as an internal library (the compiler inlines all functions).

> **Gap with book:** None — aligned with the book's state root proof approach (similar to OP Stack's `L2ToL1MessagePasser`).

## Gas Token Deposits

The L2Bridge predeploy at `0x00...fffd` is deployed in genesis with a large preminted ETH balance (`U256::MAX / 2`) to cover all future L1 messages. A relayer calls `processL1Message()` to execute L1 messages (transferring ETH and/or executing calldata) from the bridge's balance. The relayer pays gas, solving the "first deposit problem" — users don't need gas to receive L1 messages. The NativeRollup contract on L1 accumulates ETH over time as users call `sendL1Message()`.

> **Gap with book:** ETH only — no support for custom gas tokens (ERC20, NFTs). The book supports arbitrary gas tokens via the preminted-token approach.

## Fee Market

EIP-1559 base fee is computed from explicit parent fields (`parentBaseFee`, `parentGasLimit`, `parentGasUsed`) via `calculate_base_fee_per_gas`. The coinbase is an explicit precompile input — priority fees go to the coinbase address. Burned fees are computed as `base_fee_per_gas * block_gas_used` and returned in the precompile output. The NativeRollup contract on L1 tracks `blockGasLimit`, `lastBaseFeePerGas`, and `lastGasUsed` on-chain from previous block executions, so the relayer does not need to provide these values. The contract sends the burned fees amount to the relayer (`msg.sender`) when `advance()` is called.

> **Gap with book:** No DA cost mechanism. Burned fees are credited to the relayer on L1, but the L2-side crediting is not yet implemented (a production solution would redirect burned fees to the bridge contract on L2, similar to the OP Stack's `BaseFeeVault`).

## Statelessness

We use `ExecutionWitness` / `GuestProgramState` to provide stateless execution. The precompile receives a JSON-serialized witness containing the state trie, storage tries, and code for all accounts touched during execution. This enables the precompile to re-execute without persistent L2 state, aligned with the book's EIP-7864 dependency.

The `GuestProgramStateDb` adapter implements LEVM's `Database` trait backed by `GuestProgramState`, bridging the gap between the stateless execution witness and LEVM's database interface. This direct adapter is needed because `ethrex-levm` cannot depend on `ethrex-vm` (it's the other way around) — it replaces the indirect `GuestProgramStateWrapper` → `VmDatabase` → `DynVmDatabase` path.

## Block Execution Validation

Both state root and receipts root are verified after execution:

- **State root** — the `post_state_root` input field is compared against the computed root after execution
- **Receipts root** — the `post_receipts_root` input field is compared against the root computed from execution receipts (built for all transactions, including reverted ones with empty logs)

The state root check implicitly guarantees both correct L1 message processing and correct L2→L1 withdrawal recording (since the L2Bridge writes withdrawal hashes to its `sentMessages` storage mapping).

> **Gap with book:** None — aligned.

## Contracts

### NativeRollup.sol

L1 contract that manages L2 state on-chain. Storage layout:

| Slot | Field | Description |
|------|-------|-------------|
| 0 | `stateRoot` | Current L2 state root |
| 1 | `blockNumber` | Latest committed L2 block number |
| 2 | `blockGasLimit` | L2 block gas limit (constant) |
| 3 | `lastBaseFeePerGas` | Base fee from the last executed block |
| 4 | `lastGasUsed` | Gas used in the last executed block |
| 5 | `pendingL1Messages` | Array of L1 message hashes |
| 6 | `l1MessageIndex` | Next L1 message index to consume |
| 7 | `stateRootHistory` | `mapping(uint256 => bytes32)` — state root per block number |
| 8 | `claimedWithdrawals` | `mapping(bytes32 => bool)` — prevents double-claiming |
| 9 | `stateRootTimestamps` | `mapping(uint256 => uint256)` — commit timestamp per block |
| 10 | `_reentrancyGuard` | Reentrancy protection |

Immutables: `CHAIN_ID` (L2 chain ID) and `FINALITY_DELAY` (minimum seconds before withdrawals can be claimed).

Constructor: `constructor(bytes32 _initialStateRoot, uint256 _blockGasLimit, uint256 _initialBaseFee, uint64 _chainId, uint256 _finalityDelay)` with `lastGasUsed = 0` for the first block.

Uses a 5-field `BlockParams` struct: `postStateRoot`, `postReceiptsRoot`, `coinbase`, `prevRandao`, `timestamp`.

Functions:

- **`sendL1Message(address _to, uint256 _gasLimit, bytes _data)`** — payable; records `keccak256(abi.encodePacked(from, to, value, gasLimit, keccak256(data), nonce))` as an L1 message hash
- **`receive()`** — payable fallback; sends an L1 message to `msg.sender` with empty data
- **`advance(uint256, BlockParams, bytes, bytes)`** — reads storage fields, computes L1 messages Merkle root, builds ABI-encoded precompile calldata (15 slots), calls EXECUTE at `0x0101`, decodes the 160-byte return, updates on-chain state, sends burned fees to relayer
- **`claimWithdrawal(address, address, uint256, uint256, uint256, bytes[], bytes[])`** — verifies MPT account + storage proofs against `stateRootHistory[blockNumber]`, enforces finality delay, transfers ETH

### L2Bridge.sol

L2 predeploy at `0x00...fffd` handling both L1 message processing and withdrawals. Storage: slot 0 = relayer, slot 1 = l1MessageNonce, slot 2 = withdrawalNonce, slot 3 = `sentMessages` mapping.

- **`processL1Message(..., bytes32[] merkleProof)`** — verifies Merkle inclusion proof against L1Anchor, executes `to.call{value, gas}(data)`
- **`withdraw(address receiver)`** — payable; writes `sentMessages[hash] = true` for MPT proving on L1

### L1Anchor.sol

L2 predeploy at `0x00...fffe`. Single `bytes32 public l1MessagesRoot` at storage slot 0. No setter — the EXECUTE precompile writes directly.

### MPTProof.sol

Solidity library for MPT proof verification: trie traversal, RLP decoding, account proof verification (state root → storageRoot), and storage proof verification (storageRoot → mapping value). Compiled as an internal library (inlined by the compiler).

### merkle_tree.rs

`crates/common/merkle_tree.rs` — OpenZeppelin-compatible Merkle tree using commutative Keccak256 hashing. Provides `compute_merkle_root()` and `compute_merkle_proof()`. Shared across the EXECUTE precompile, L2 networking RPC, and integration tests.

## Feature Flag

All native rollups code is gated behind the `native-rollups` feature flag:

```toml
# In crates/vm/levm/Cargo.toml
[features]
native-rollups = []

# In crates/vm/Cargo.toml (propagates to levm)
[features]
native-rollups = ["ethrex-levm/native-rollups"]

# In cmd/ethrex/Cargo.toml (enables for the binary)
[features]
native-rollups = ["ethrex-vm/native-rollups"]

# In test/Cargo.toml (enables for integration tests)
[features]
native-rollups = ["ethrex-levm/native-rollups"]
```

This ensures the precompile code is only compiled when explicitly opted in.

## Summary Table

| Aspect | Book | Us | Alignment |
|--------|------|-----|-----------|
| `apply_body` variant | Specified | Implemented | **Aligned** |
| State root validation | Required | Implemented | **Aligned** |
| Receipts root validation | Required | Implemented | **Aligned** |
| Base fee from parent params | Required | EIP-1559 computation | **Aligned** |
| Parent gas tracking on L1 | Implied | On-chain storage | **Aligned** |
| Coinbase as input | Required | Implemented | **Aligned** |
| Burned fees in output | Proposed | Implemented | **Aligned** |
| Tx filtering (blobs) | Required | Implemented (+ ethrex types) | **Aligned+** |
| Statelessness | Required (EIP-7864) | `ExecutionWitness`-based | **Aligned** |
| `prev_randao` configurable | Required | Implemented | **Aligned** |
| Preminted gas tokens | Recommended | Implemented | **Aligned** |
| No custom tx types | Design principle | Achieved (relayer txs) | **Aligned** |
| First deposit problem | Unresolved in book | Solved (relayer pays gas) | **Ahead** |
| `chain_id` as input | Explicit parameter | Explicit input (slot 0) | **Aligned** |
| System transaction | Step in `apply_body` | L1Anchor predeploy write | **Aligned** |
| L1 anchoring | System contract + system tx | L1Anchor predeploy + system write | **Aligned** |
| L1→L2 messaging | Proof-based (no custom tx) | Merkle proofs against anchored root | **Aligned** |
| L2→L1 messaging | State/receipts root proofs (TBD) | State root proofs (MPT account + storage) | **Aligned** |
| Finality delay | Implied for production | Configurable (`FINALITY_DELAY` immutable) | **Aligned** |
| Forced transactions | WIP (FOCIL, threshold) | Not implemented | **Gap** |
| Gas metering | TBD | Flat 100k gas | **Both TBD** |
| Serialization | Blob references (TBD) | RLP calldata + JSON witness | **Different** |
| EXECUTE output format | TBD | 160 bytes (5 fields) | **We defined** |
| `parent_beacon_block_root` | TBD, configurable | Not included | **Minor gap** |
| L2-side burned fee handling | Not specified | Not implemented | **Gap** |
| DA cost pricing | WIP | Not implemented | **Both WIP** |
| Custom gas tokens | Supported (ERC20, NFTs) | ETH only | **Partial** |

## Limitations (Phase 1)

This PoC intentionally omits several things that would be needed for production:

- **Fixed gas cost** — Uses a flat 100,000 gas cost instead of real metering
- **No blob data support** — Only calldata-based input (spec proposes blob references for transactions)
- **No generic L1 block hash anchoring** — L1Anchor stores an L1 messages Merkle root, not a generic L1 block hash (which would enable broader cross-chain proofs)
- **Configurable finality delay for withdrawals** — The PoC includes a configurable `FINALITY_DELAY` that enforces a minimum time before withdrawals can be claimed (set to 1 second in tests). Production would use a longer challenge period (e.g., 7 days)
- **No forced transaction mechanism** — No censorship resistance guarantees
- **L2 ETH supply drain** — EIP-1559 base fees are burned on every L2 transaction, permanently removing ETH from circulation. Burned fees are now tracked and credited to the relayer on L1 (via `advance()`), but the L2-side crediting mechanism is not yet implemented. A production solution would redirect burned fees to the bridge contract on L2 (similar to the OP Stack's `BaseFeeVault`)
- **RLP transaction list** — Transactions are passed as an RLP-encoded list in ABI calldata (spec envisions blob-referenced transactions)

These are all Phase 2+ concerns.

## Files

| File | Description |
|------|-------------|
| `crates/vm/levm/src/execute_precompile.rs` | EXECUTE precompile logic (ABI calldata parsing) |
| `crates/vm/levm/src/db/guest_program_state_db.rs` | GuestProgramState → LEVM Database adapter |
| `crates/vm/levm/src/precompiles.rs` | Precompile registration (modified) |
| `crates/vm/levm/src/db/mod.rs` | Module export (modified) |
| `crates/vm/levm/src/lib.rs` | Module export (modified) |
| `crates/vm/levm/Cargo.toml` | Feature flag (modified) |
| `crates/vm/Cargo.toml` | Feature flag propagation (modified) |
| `cmd/ethrex/Cargo.toml` | Feature flag for ethrex binary (modified) |
| `crates/common/merkle_tree.rs` | Shared OpenZeppelin-compatible Merkle tree (commutative Keccak256) |
| `crates/vm/levm/contracts/NativeRollup.sol` | L1 contract: L2 state manager with L1 message hash queue, Merkle root advance, and withdrawal claiming |
| `crates/vm/levm/contracts/MPTProof.sol` | Solidity library: MPT trie traversal, RLP decoding, account/storage proof verification |
| `crates/vm/levm/contracts/L1Anchor.sol` | L2 predeploy: stores L1 messages Merkle root anchored by EXECUTE (system write) |
| `crates/vm/levm/contracts/L2Bridge.sol` | L2 contract: unified bridge for L1 messages (processL1Message with Merkle proof) and withdrawals |
| `test/tests/l2/native_rollup.rs` | Integration test: deposit, withdraw, and counter roundtrip with live L2 |
| `test/Cargo.toml` | Feature flag for test crate (modified) |
| `crates/l2/Makefile` | `init-l1` supports `NATIVE_ROLLUPS=1` (modified) |
