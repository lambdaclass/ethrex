# Native Rollups: EXECUTE Precompile (PoC)

## Background

[EIP-8079](https://github.com/ethereum/EIPs/pull/9608) proposes "native rollups" — a mechanism where L1 verifies L2 state transitions by re-executing them inside the EVM via an `EXECUTE` precompile. This replaces complex proof systems (zkVM/fraud proofs) with direct execution, leveraging the fact that L1 already has an EVM capable of running the same transactions.

This is a Phase 1 proof-of-concept implementation that demonstrates the concept works at the EVM level.

## Architecture

```
Individual Block Fields + Transactions (RLP) + ExecutionWitness (JSON) + L1 Anchor (Merkle root)
        |
  EXECUTE precompile (in LEVM) — apply_body variant
        |
  1. Parse ABI-encoded calldata (14 slots: 12 static fields + 2 dynamic pointers)
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

### Components

**`execute_precompile.rs`** — The core precompile logic implementing the `apply_body` variant. Parses ABI-encoded calldata with 14 slots: 12 static fields (pre/post state roots, receipts root, block number, gas limit, coinbase, prev_randao, timestamp, parent gas parameters, l1Anchor) and 2 dynamic parameters (RLP-encoded transactions, JSON-serialized witness). Computes the base fee from explicit parent fields (EIP-1559), writes the l1Anchor to the L1Anchor predeploy's storage slot 0 (system transaction — step 5 of `apply_body`), builds a synthetic block header from individual fields, and orchestrates the full verification flow: state root checks, block execution, and receipts root verification. The l1Anchor is a Merkle root over consumed L1 message hashes, computed by the L1 NativeRollup contract and written to the L1Anchor predeploy so that L2 contracts can verify individual messages via Merkle inclusion proofs. The state root check at the end implicitly guarantees correct L1 message processing and correct L2→L1 withdrawal recording (since the L2Bridge writes withdrawal hashes to its `sentMessages` storage mapping). The post-state root, block number, gas used, burned fees, and base fee per gas are returned as `abi.encode(postStateRoot, blockNumber, gasUsed, burnedFees, baseFeePerGas)` — 160 bytes. The burned fees are computed as `base_fee_per_gas * block_gas_used` (EIP-1559 base fee is constant per block). The `baseFeePerGas` is the computed base fee for the executed block (from parent gas parameters via EIP-1559), returned so the L1 contract can track it on-chain for subsequent blocks. Also contains helpers for block execution (`execute_block`), gas price calculation, transaction type validation, Merkle tree construction (for L1→L2 messaging), and Merkle proof generation.

**`guest_program_state_db.rs`** — A thin adapter that implements LEVM's `Database` trait backed by `GuestProgramState`. This bridges the gap between the stateless execution witness (which provides account/storage/code data via tries) and LEVM's database interface. Uses a `Mutex` for interior mutability since `GuestProgramState` requires `&mut self` while `Database` methods take `&self`.

**`precompiles.rs` (modified)** — Registers the EXECUTE precompile at address `0x0101`, dispatched at runtime before the standard const precompile table lookup.

**`NativeRollup.sol`** — A Solidity contract that manages L2 state on-chain. Maintains `stateRoot` (slot 0), `blockNumber` (slot 1), `blockGasLimit` (slot 2), `lastBaseFeePerGas` (slot 3), `lastGasUsed` (slot 4), a `pendingL1Messages` array of L1 message hashes (slot 5), `l1MessageIndex` (slot 6), `stateRootHistory` mapping (slot 7), `claimedWithdrawals` mapping (slot 8), and a reentrancy guard (slot 9). The contract tracks parent gas parameters on-chain from previous block executions — `blockGasLimit` (constant), `lastBaseFeePerGas`, and `lastGasUsed` — instead of trusting the relayer to provide them. Uses a 5-field `BlockParams` struct (`postStateRoot`, `postReceiptsRoot`, `coinbase`, `prevRandao`, `timestamp`). Constructor: `constructor(bytes32 _initialStateRoot, uint256 _blockGasLimit, uint256 _initialBaseFee)` with `lastGasUsed = _blockGasLimit / 2` to keep base fee stable for the first block. Includes inlined MPT (Merkle Patricia Trie) proof verification for L2→L1 withdrawal claiming. Exposes:
- `sendL1Message(address _to, uint256 _gasLimit, bytes _data)` — payable function that records `keccak256(abi.encodePacked(from, to, value, gasLimit, keccak256(data), nonce))` as an L1 message hash (168-byte preimage)
- `receive()` — payable fallback that sends an L1 message to `msg.sender` with `DEFAULT_GAS_LIMIT` (100,000) and empty data
- `advance(uint256, BlockParams, bytes, bytes)` — reads `blockNumber + 1`, `blockGasLimit`, `lastBaseFeePerGas`, `lastGasUsed` from storage, computes a Merkle root over consumed L1 message hashes (commutative Keccak256, OpenZeppelin-compatible), builds ABI-encoded precompile calldata via `abi.encode(stateRoot, blockParams fields..., storageFields..., l1Anchor, _transactions, _witness)` (14 slots), calls EXECUTE at `0x0101`, decodes the returned `(postStateRoot, blockNumber, gasUsed, burnedFees, baseFeePerGas)` (160 bytes), stores the state root in `stateRootHistory[blockNumber]` for withdrawal proving, updates `lastGasUsed` and `lastBaseFeePerGas` from the precompile return, sends burned fees ETH to `msg.sender` (the relayer), and updates state
- `claimWithdrawal(address, address, uint256, uint256, uint256, bytes[], bytes[])` — allows users to claim withdrawals initiated on L2 by providing MPT account proof + storage proof against the L2 state root stored in `stateRootHistory[blockNumber]`. Verifies that `L2Bridge.sentMessages[withdrawalHash] == true` via account proof (state root → L2Bridge storageRoot) and storage proof (storageRoot → mapping value). Uses checks-effects-interactions pattern with reentrancy guard.

**`L1Anchor.sol`** — A minimal L2 predeploy at `0x00...fffe` (one above L2Bridge) that stores the L1 messages Merkle root anchored by the EXECUTE precompile. Has a single `bytes32 public l1MessagesRoot` at storage slot 0. No setter function — the EXECUTE precompile writes directly to slot 0 before executing regular transactions (system transaction). The L2Bridge reads from this contract to verify Merkle proofs.

**`L2Bridge.sol`** — A unified L2 bridge contract deployed at `0x00...fffd` that handles both L1 message processing and withdrawals. Storage layout: slot 0 = relayer address, slot 1 = l1MessageNonce, slot 2 = withdrawalNonce, slot 3 = `sentMessages` mapping (`mapping(bytes32 => bool)`). The relayer calls `processL1Message(address from, address to, uint256 value, uint256 gasLimit, bytes data, uint256 nonce, bytes32[] merkleProof)` to execute L1 messages on L2 — verifying a Merkle inclusion proof against the L1 messages root anchored in the L1Anchor predeploy, then transferring ETH and executing arbitrary calldata via `to.call{value: value, gas: gasLimit}(data)` — and emitting `L1MessageProcessed` events. Users call `withdraw(address receiver)` with ETH value to initiate L2→L1 withdrawals, which writes `sentMessages[keccak256(abi.encodePacked(sender, receiver, amount, nonce))] = true` to contract storage and emits `WithdrawalInitiated` events. The storage write makes the withdrawal provable via MPT proofs against the L2 state root on L1; the event is kept for off-chain indexing.

### Why a Separate Database Adapter?

The existing `GuestProgramStateWrapper` in `crates/vm/witness_db.rs` bridges `GuestProgramState` to the `VmDatabase` trait, which then gets adapted to LEVM's `Database` trait via `DynVmDatabase`. However, `ethrex-levm` cannot depend on `ethrex-vm` (it's the other way around), so a direct adapter is needed. The `GuestProgramStateDb` is this direct bridge — about 100 lines, doing the same job without the intermediate layer.

### L1 Message Mechanism (Proof-Based)

The L1→L2 message flow uses a relayer, a prefunded L2 bridge contract, and Merkle proof verification against an anchored L1 messages root, following the book's recommended proof-based pattern (similar to Linea/Taiko):

1. Users call `NativeRollup.sendL1Message(to, gasLimit, data)` on L1 with ETH value — this records `keccak256(abi.encodePacked(from, to, value, gasLimit, keccak256(data), nonce))` (168-byte preimage) as an L1 message hash in the contract's `pendingL1Messages` array
2. When `advance()` is called with `_l1MessagesCount`, it computes a **Merkle root** over the consumed L1 message hashes (commutative Keccak256, OpenZeppelin-compatible)
3. The Merkle root is passed to the EXECUTE precompile as the `l1Anchor` parameter (static `bytes32`)
4. The EXECUTE precompile writes the `l1Anchor` to the **L1Anchor predeploy** (`0x00...fffe`) storage slot 0 before executing regular transactions (system transaction)
5. On L2, a relayer sends real transactions calling `L2Bridge.processL1Message(from, to, value, gasLimit, data, nonce, merkleProof)`, which verifies the Merkle inclusion proof against the anchored root in L1Anchor, then executes `to.call{value: value, gas: gasLimit}(data)` (transferring ETH and/or executing calldata) and emits `L1MessageProcessed` events
6. The **state root check** at the end of execution implicitly guarantees correct message processing (if claims aren't included or are wrong, the post-state root won't match)

**Block builder constraint:** The relayer chooses how many L1 messages to consume (`_l1MessagesCount` can be 0). The Merkle root over those messages is computed in `advance()` on L1 and anchored in the L1Anchor predeploy *before* the block transactions execute. This means the block builder must know the Merkle root at block construction time and include the matching `processL1Message()` transactions — if they don't match, the L2Bridge proof verification will produce a different state root and the EXECUTE precompile will revert.

The relayer pays gas for L1 message transactions, solving the "first deposit problem". L1 messages support arbitrary calldata, enabling not just ETH transfers but also arbitrary contract calls on L2.

### Calldata Format (ABI-encoded — apply_body variant)

The EXECUTE precompile uses the `apply_body` variant with individual field inputs:

```
abi.encode(
    bytes32 preStateRoot,           // slot 0  (static)
    bytes32 postStateRoot,          // slot 1  (static)
    bytes32 postReceiptsRoot,       // slot 2  (static)
    uint256 blockNumber,            // slot 3  (static)
    uint256 blockGasLimit,          // slot 4  (static)
    address coinbase,               // slot 5  (static, ABI-padded to 32 bytes)
    bytes32 prevRandao,             // slot 6  (static)
    uint256 timestamp,              // slot 7  (static)
    uint256 parentBaseFee,          // slot 8  (static)
    uint256 parentGasLimit,         // slot 9  (static)
    uint256 parentGasUsed,          // slot 10 (static)
    bytes32 l1Anchor,               // slot 11 (static — Merkle root of consumed L1 messages)
    bytes   transactions,           // slot 12 (dynamic -- offset pointer)
    bytes   witnessJson             // slot 13 (dynamic -- offset pointer)
)

Head: 14 x 32 = 448 bytes (slots 0-11 static, slots 12-13 dynamic offset pointers)
Tail: transactions RLP data, witness JSON data (each prefixed with 32-byte length)
```

The base fee is computed from explicit parent fields (`parentBaseFee`, `parentGasLimit`, `parentGasUsed`) using `calculate_base_fee_per_gas` (EIP-1559). A synthetic block header is built internally from the individual fields for block execution.

### Return Value

```
abi.encode(bytes32 postStateRoot, uint256 blockNumber, uint256 gasUsed, uint256 burnedFees, uint256 baseFeePerGas)  — 160 bytes
```

The post-state root is extracted from `block.header.state_root` and verified against the computed state root after execution. The block number is extracted from `block.header.number`. The gas used is the cumulative gas consumed by all transactions in the block (pre-refund, matching `block.header.gas_used`). The burned fees are `base_fee_per_gas * block_gas_used` — the total EIP-1559 base fees burned during the block. The NativeRollup contract on L1 sends this amount to the relayer (msg.sender of advance()). The base fee per gas is the computed EIP-1559 base fee for the executed block, returned so the L1 contract can store it on-chain and use it as `parentBaseFee` for the next block (avoiding relayer trust for this value). No withdrawal root is returned — withdrawals are proven directly against the post-state root via MPT proofs on L1.

### Encoding Details

- **Transactions** use RLP list encoding (`Vec::<Transaction>::encode_to_vec()` / `Vec::<Transaction>::decode()`)
- **ExecutionWitness** uses JSON because it doesn't have RLP support — it uses serde/rkyv for serialization instead
- **Block fields** are individual ABI-encoded slots (bytes32, uint256, address)
- **L1 anchor** is a static `bytes32` — the Merkle root over consumed L1 message hashes, computed by the L1 NativeRollup contract

The NativeRollup contract fills in `preStateRoot` from its own storage, reads `blockNumber + 1`, `blockGasLimit`, `lastBaseFeePerGas`, and `lastGasUsed` from storage, passes the remaining block parameters via a 5-field `BlockParams` struct, computes the `l1Anchor` Merkle root from its pending L1 message queue, and forwards transactions/witness bytes unchanged (opaque to the contract). The contract decodes the precompile's 160-byte return value to extract the new state root, block number, gas used, burned fees, and base fee per gas. It updates `lastGasUsed` and `lastBaseFeePerGas` from the return, stores the state root in `stateRootHistory[blockNumber]` for withdrawal proving, and sends the burned fees amount to `msg.sender` (the relayer) as ETH.

### Withdrawal Mechanism (State Root Proof-Based)

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
     → account proof: state root → L2Bridge account → extracts storageRoot
     → storage proof: storageRoot → sentMessages[withdrawalHash] == true
     → marks withdrawal as claimed (prevents double-claiming)
     → transfers ETH to receiver
```

Each withdrawal is uniquely identified by `keccak256(abi.encodePacked(from, receiver, amount, messageId))`. The `messageId` is a counter maintained by the L2Bridge contract (`withdrawalNonce`), starting at 0 and incrementing per withdrawal. The MPT proof verification is inlined directly in the NativeRollup contract (~200 lines of Solidity) — no external libraries needed.

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

### Transaction Type Validation

Native rollup blocks only allow standard L1 transaction types. The following are rejected before sender recovery (cheap check first):

- **EIP-4844 blob transactions** — blob data doesn't exist on L2
- **Privileged L2 transactions** — ethrex-specific L2 type, not valid in native rollups
- **Fee token transactions** — ethrex-specific L2 type, not valid in native rollups

Legacy, EIP-2930, EIP-1559, and EIP-7702 transactions are allowed.

## How to Test

All tests live in the shared test crate (`test/`) and require the `native-rollups` feature flag.

There are three levels of testing, from fastest to most comprehensive:

### 1. Unit tests (no infrastructure needed)

These tests call the EXECUTE precompile directly in-process — no L1 node, no network, no Solidity compilation. They verify the core precompile logic works correctly.

```bash
cargo test -p ethrex-test --features native-rollups -- levm::native_rollups --nocapture
```

This runs all 3 offline tests:

| Test | What it verifies |
|------|-----------------|
| `test_execute_precompile_transfer_and_l1_message` | Full precompile flow: relayer processL1Message (with Merkle proof) + transfer + state root verification + L1Anchor system write |
| `test_native_rollup_contract` | NativeRollup.sol running in LEVM calling the EXECUTE precompile (with Merkle root for L1 messages) |
| `test_execute_precompile_rejects_blob_transactions` | EIP-4844 blob transactions are rejected |

Expected output:

```
running 3 tests
test levm::native_rollups::test_execute_precompile_rejects_blob_transactions ... ok
ABI-encoded EXECUTE calldata: ... bytes
EXECUTE precompile succeeded!
  Pre-state root:  0xd0af...
  Post-state root: 0x042b...
  Relayer processed L1 message: 5 ETH to charlie
  Alice sent 1 ETH to Bob
  Gas used: ...
test levm::native_rollups::test_execute_precompile_transfer_and_l1_message ... ok
sendL1Message TX succeeded (5 ETH to charlie)
NativeRollup contract demo succeeded!
  L2 state transition verified via sendL1Message() + advance():
    Pre-state root:  0x151a...
    Post-state root: 0x1988...
    Block number:    1
    L1 message index: 1
  Gas used: ...
test levm::native_rollups::test_native_rollup_contract ... ok

test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; ...
```

#### What the main test does (`test_execute_precompile_transfer_and_l1_message`)

This test simulates a complete L2 block verification with the relayer-based L1 message flow:

1. **Setup genesis state** — Creates accounts: Alice (10 ETH), Relayer (1 ETH for gas), L2Bridge at `0x00...fffd` (100 ETH preminted, storage: slot 0 = relayer address), L1Anchor at `0x00...fffe` (with l1Anchor in storage slot 0), Bob (0), Charlie (0), Coinbase (0), address(0). Inserts them into a state trie and computes the pre-state root.
2. **Build L2 transactions** — Two EIP-1559 transactions: (1) Relayer calls `L2Bridge.processL1Message(l1_sender, charlie, 5 ETH, 100k, "", 0, proof)` with Merkle proof (gas limit 200k), (2) Alice sends 1 ETH to Bob (gas limit 21k).
3. **Execute through LEVM** — Runs both transactions via `GuestProgramState → GuestProgramStateDb → VM` to get exact gas usage, receipts with logs, and the post-state root. No manual balance computation needed.
4. **Build witness** — Creates an `ExecutionWitness` containing the state trie, bridge and anchor storage tries, bridge and anchor bytecode, chain config, and block headers.
5. **Compute l1Anchor** — `l1Anchor = compute_merkle_root([keccak256(abi.encodePacked(from[20], to[20], value[32], gasLimit[32], keccak256(data)[32], nonce[32]))])` (168-byte preimage per message, Merkle root over message hashes).
6. **Build ABI-encoded calldata** — Encodes 14 ABI slots: individual block fields (pre/post state roots, receipts root, block number, gas limit, coinbase, prev_randao, timestamp, parent gas parameters, l1Anchor) plus dynamic transactions (RLP list) and witness (JSON), matching what the NativeRollup contract would produce.
7. **Call the precompile** — Invokes `execute_precompile()` directly. The precompile writes l1Anchor to L1Anchor storage (system transaction), builds a synthetic block header from the individual fields, re-executes the block, and verifies state/receipts roots.
8. **Verify result** — Asserts the precompile returns `abi.encode(postStateRoot, blockNumber, gasUsed, burnedFees, baseFeePerGas)` (160 bytes).

#### What the contract test does (`test_native_rollup_contract`)

This test runs the NativeRollup.sol contract inside LEVM, which in turn calls the EXECUTE precompile:

```
L1 tx1 -> NativeRollup.sendL1Message(charlie, 100000, "") {value: 5 ETH}
  -> records keccak256(abi.encodePacked(sender, charlie, 5 ETH, 100000, keccak256(""), 0)) as L1 message hash

L1 tx2 -> NativeRollup.advance(1, blockParams, transactions, witnessJson)
  -> reads stateRoot from slot 0
  -> computes Merkle root over 1 consumed L1 message hash
  -> builds calldata: abi.encode(stateRoot, blockParams fields..., l1Anchor, transactions, witness)
  -> CALL to 0x0101 (EXECUTE precompile)
    -> precompile writes l1Anchor to L1Anchor predeploy storage (system tx)
    -> builds synthetic block header from individual fields
    -> computes base fee from parent fields (EIP-1559)
    -> re-executes the L2 block (which includes relayer's processL1Message tx with Merkle proof)
    -> verifies state roots and receipts root (state root implicitly covers L1 message correctness)
  -> returns abi.encode(postStateRoot, blockNumber, gasUsed, burnedFees, baseFeePerGas)
  -> contract decodes return value
  -> updates lastGasUsed (slot 4) and lastBaseFeePerGas (slot 3)
  -> sends burnedFees ETH to msg.sender (relayer)
  -> updates storage: stateRoot (slot 0), blockNumber (slot 1), l1MessageIndex (slot 6), stateRootHistory[blockNumber] (slot 7 mapping)
```

The test verifies all three storage slots were updated correctly after `advance()` succeeds. The contract stores the state root in `stateRootHistory[blockNumber]` for withdrawal proving via MPT proofs. Additionally, the test verifies that the sender (relayer) received the burned fees ETH from the contract.

### 2. Integration test (requires running L1)

This test deploys the NativeRollup contract on a real L1 node (ethrex with the EXECUTE precompile enabled), sends real transactions, and reads storage via RPC.

**Prerequisites:** `solc` (Solidity compiler) must be installed for contract compilation.

**Terminal 1** — Start the L1:

```bash
NATIVE_ROLLUPS=1 make -C crates/l2 init-l1
```

This starts an ethrex node on `localhost:8545` with the EXECUTE precompile registered at address `0x0101` and a pre-funded account for testing.

**Terminal 2** — Run the test:

```bash
cargo test -p ethrex-test --features native-rollups -- l2::native_rollups --nocapture
```

Expected output:

```
Connected to L1 at http://localhost:8545
Deployer: 0x3d1e15a1a55578f7c920884a9943b3b35d0d885b
NativeRollup deployed at: 0x...
  Deploy tx: 0x...
  Initial stateRoot verified: 0x...
  sendL1Message() tx: 0x...
  advance() tx: 0x...
  Gas used: ...

  Phase 1 passed: transfer + L1 message
  ...

  advance(block 2) tx: 0x...
  Gas used: ...
  stateRootHistory[2]: 0x...
  Receiver balance before claim: 0
  claimWithdrawal() tx: 0x...
  Receiver balance after claim: 1000000000000000000
  Withdrawal amount: 1000000000000000000

NativeRollup integration test passed (transfer + L1 message + withdrawal + claim)!
  Contract:        0x...
  L2 blocks:       2
  L1 message:      5 ETH to charlie
  Withdrawal:      1 ETH from alice to 0x...
test l2::native_rollups::test_native_rollup_on_l1 ... ok
```

#### What the integration test does (`test_native_rollup_on_l1`)

The test exercises the full lifecycle: deploy, send L1 message, advance (2 blocks), and withdrawal claiming.

**Phase 1 — Transfer + L1 Message (L2 block 1):**

1. **Connect to L1** — Creates an `EthClient` pointing at `localhost:8545` and loads the pre-funded signer from the Makefile's private key.
2. **Build L2 state transitions** — Calls `build_l2_state_transition(l1_sender)` to create L2 block 1 (relayer processL1Message + Alice→Bob transfer) and `build_l2_withdrawal_block()` to create L2 block 2 (Alice withdraws 1 ETH via the L2Bridge).
3. **Compile contracts** — Calls `solc` via `compile_contract()` for both `NativeRollup.sol` and `L2Bridge.sol`.
4. **Deploy NativeRollup** — Sends a CREATE transaction with `deployBytecode + abi.encode(preStateRoot)` as constructor arg.
5. **Verify initial state** — Reads storage slot 0 via `eth_getStorageAt` and asserts it matches the pre-state root.
6. **Send L1 message** — Sends `sendL1Message(charlie, 100000, "")` with 5 ETH via SDK helpers.
7. **Advance (block 1)** — Sends `advance(1, blockParams, transactions, witnessJson)` to process the L2 block with the L1 message.
8. **Verify block 1 state** — Asserts stateRoot, blockNumber=1, l1MessageIndex=1, and stateRootHistory[1]=post_state_root.

**Phase 2 — Withdrawal + Claim (L2 block 2):**

9. **Advance (block 2)** — Sends `advance(0, block2Params, transactions2, witness2Json)` with 0 L1 messages. Block 2 contains Alice calling `L2Bridge.withdraw(receiver)` with 1 ETH.
10. **Verify block 2 state** — Asserts stateRoot updated, blockNumber=2, and stateRootHistory[2]=block2_post_state_root.
11. **Check receiver balance** — Reads the L1 receiver's balance (should be 0 before claiming).
12. **Claim withdrawal** — Sends `claimWithdrawal(aliceL2, receiver, 1 ETH, messageId=0, blockNumber=2, accountProof, storageProof)` with MPT proofs generated from the L2 post-state trie.
13. **Verify receiver got ETH** — Asserts the receiver's balance increased by exactly 1 ETH.

**Building L2 block 2 (`build_l2_withdrawal_block`):**

The withdrawal block requires computing exact `gas_used` and `post_state_root` before building the final block (since the EXECUTE precompile validates both). The helper:

1. Reconstructs the block 1 post-state (including the L2Bridge contract at `0x00...fffd` with storage: slot 0 = relayer, slot 1 = l1MessageNonce = 1, and balance = 95 ETH after the 5 ETH L1 message)
2. Builds a withdrawal transaction: Alice → L2Bridge.withdraw(receiver) with 1 ETH
3. Executes through LEVM via `GuestProgramState → GuestProgramStateDb → GeneralizedDatabase → VM` to get gas_used and state transitions
4. Applies state transitions and computes the post-state root
5. Builds the final block and witness with correct header values

## Files

| File | Description |
|------|-------------|
| `crates/vm/levm/src/execute_precompile.rs` | EXECUTE precompile logic (ABI calldata parsing) |
| `crates/vm/levm/src/db/guest_program_state_db.rs` | GuestProgramState -> LEVM Database adapter |
| `crates/vm/levm/src/precompiles.rs` | Precompile registration (modified) |
| `crates/vm/levm/src/db/mod.rs` | Module export (modified) |
| `crates/vm/levm/src/lib.rs` | Module export (modified) |
| `crates/vm/levm/Cargo.toml` | Feature flag (modified) |
| `crates/vm/Cargo.toml` | Feature flag propagation (modified) |
| `cmd/ethrex/Cargo.toml` | Feature flag for ethrex binary (modified) |
| `crates/vm/levm/contracts/NativeRollup.sol` | L1 contract: L2 state manager with L1 message hash queue, Merkle root advance, and withdrawal claiming |
| `crates/vm/levm/contracts/L1Anchor.sol` | L2 predeploy: stores L1 messages Merkle root anchored by EXECUTE (system write) |
| `crates/vm/levm/contracts/L2Bridge.sol` | L2 contract: unified bridge for L1 messages (processL1Message with Merkle proof) and withdrawals (withdraw) |
| `test/tests/levm/native_rollups.rs` | Unit tests and contract-based test |
| `test/tests/l2/native_rollups.rs` | Integration test (requires running L1) |
| `test/Cargo.toml` | Feature flag for test crate (modified) |
| `crates/l2/Makefile` | `init-l1` supports `NATIVE_ROLLUPS=1` (modified) |

## Spec Comparison: Native Rollups Book vs Our Implementation

This section compares [the L2Beat native rollups book](https://native-rollups.l2beat.com/) against our PoC implementation, aspect by aspect.

### EXECUTE Precompile Variant

**Book:** Defines two variants. The `apply_body` variant receives individual execution parameters (chain_id, number, pre/post state roots, receipts root, gas limit, coinbase, prev_randao, transactions, parent gas info, l1_anchor) and skips header validation — it only re-executes transactions and verifies the resulting state root and receipts root match. The `state_transition` variant receives full current and parent block headers, reconstructs canonical `Block` and `BlockChain` objects, and runs the complete Ethereum state transition including all header-level consensus checks (parent hash, timestamp, gas limit bounds, etc.).

**Us:** Implements the `apply_body` variant. We receive individual block fields (pre/post state roots, receipts root, block number, gas limit, coinbase, prev_randao, timestamp, parent gas parameters), an L1 anchor (Merkle root over consumed L1 message hashes), an RLP-encoded transaction list, and a JSON execution witness — 14 ABI slots total. We skip full header validation (no parent hash chain, no timestamp ordering, no ommers hash check). We compute the base fee from explicit parent fields (EIP-1559 `calculate_base_fee_per_gas`) and build a synthetic block header internally. We write the L1 anchor to the L1Anchor predeploy's storage (system transaction) before executing regular transactions. We verify: pre-state root, post-state root, and receipts root. The state root check implicitly guarantees both L1 message correctness and L2→L1 withdrawal recording. We return `abi.encode(postStateRoot, blockNumber, gasUsed, burnedFees, baseFeePerGas)` — 160 bytes. The L1 NativeRollup contract tracks `blockGasLimit`, `lastBaseFeePerGas`, and `lastGasUsed` on-chain from previous executions, so the relayer only provides 5 block parameters (postStateRoot, postReceiptsRoot, coinbase, prevRandao, timestamp).

**Gaps:** The spec references transactions via blob hashes; we embed them in the ABI calldata as an RLP-encoded list. The spec's output format is still TBD — we defined our own 160-byte return.

### L1 Anchoring

**Book:** Proposes an `L1_ANCHOR` system contract deployed on L2 that receives an arbitrary `bytes32` value from L1 (typically an L1 block hash). This value is written to L2 storage via a "system transaction" — a special unchecked transaction processed before regular block transactions. The format and validation are left to the rollup contract on L1. Higher-level messaging is built on top by passing roots and providing inclusion proofs on L2.

**Us:** We implement the L1Anchor predeploy at `0x00...fffe` with a single `bytes32 public l1MessagesRoot` at storage slot 0. The EXECUTE precompile writes the `l1Anchor` value (a Merkle root over consumed L1 message hashes) directly to this storage slot before executing regular transactions — this is the "system transaction" from step 4 of `apply_body`. The L2Bridge reads from L1Anchor to verify Merkle inclusion proofs for individual L1 messages.

**Gaps:** Our `l1Anchor` is specifically an L1 messages Merkle root, not a generic `bytes32` (e.g., L1 block hash). A production implementation could anchor an L1 block hash instead, enabling broader cross-chain proofs beyond L1 messages.

### L1 to L2 Messaging

**Book:** Recommends avoiding custom transaction types (design principle). Instead, messages should be claimed against anchored hashes using inclusion proofs. L1 contracts store message hashes; after the L1 block hash is anchored on L2 via the `L1_ANCHOR` contract, L2 contracts can verify message inclusion against that root. This is similar to Linea/Taiko's approach. The book surveys four existing stacks (OP Stack, Linea, Taiko, Orbit) and favors the proof-based approach over custom deposit transaction types.

**Us:** We use a proof-based approach aligned with the book's recommendation (Linea/Taiko style). The L1 NativeRollup contract computes a Merkle root over consumed L1 message hashes and passes it to EXECUTE as the `l1Anchor`. The EXECUTE precompile writes this root to the L1Anchor predeploy's storage. On L2, a relayer sends regular signed transactions calling `L2Bridge.processL1Message()` with Merkle inclusion proofs against the anchored root. The state root check at the end of EXECUTE implicitly guarantees correct message processing. No custom transaction types are used — all L1 messages are regular signed transactions. The relayer pays gas, solving the "first deposit problem".

**Match:** Aligned with the spec's proof-based messaging pattern. Our approach uses the anchored Merkle root for inclusion proofs, matching the book's Linea/Taiko recommendation.

### L2 to L1 Messaging (Withdrawals)

**Book:** Acknowledges uncertainty about exposing custom data structures from L2 to L1. The `EXECUTE` precompile naturally exposes the state root, and potentially the receipts root. The book suggests statelessness (EIP-7864) will reduce the cost of inclusion proofs against the state root. Existing stacks use different approaches: OP Stack uses a `L2ToL1MessagePasser` with output roots; Linea uses a custom Merkle tree; Taiko uses `SignalService` with storage proofs.

**Us:** We use state root proofs, aligned with the book's recommendation. The L2Bridge contract at `0x00...fffd` writes `sentMessages[withdrawalHash] = true` to storage when users call `withdraw(receiver)`. The EXECUTE precompile returns the post-state root (which captures the L2Bridge storage), and the NativeRollup contract stores it in `stateRootHistory[blockNumber]`. Users claim on L1 by providing MPT account proof (state root → L2Bridge storageRoot) and storage proof (storageRoot → `sentMessages[hash] == true`). No custom data structures in the precompile — the state root is the single source of truth for all L2 state, including pending withdrawals. Similar to the OP Stack's `L2ToL1MessagePasser` pattern.

**Match:** Aligned with the spec's state root proof approach. No finality delay before claims (production would add one).

### Gas Token Deposits

**Book:** Favors the "preminted token" approach (like Linea/Taiko) — a predeployed L2 contract holds preminted gas tokens that are unlocked when L1→L2 messages are processed. This avoids custom transaction types and supports arbitrary gas tokens (ETH, ERC20, NFTs). The book identifies the "first deposit problem" as unresolved: users need gas to claim their initial deposit, but they don't have gas tokens yet.

**Us:** We use the preminted-token approach (aligned with the book's recommendation, similar to Taiko/Linea). The L2Bridge predeploy at `0x00...fffd` is deployed in genesis with a large preminted ETH balance to cover all future L1 messages. A relayer calls `processL1Message()` to execute L1 messages (transferring ETH and/or executing calldata) from the bridge's balance. The relayer pays gas, solving the "first deposit problem" — users don't need gas to receive L1 messages. The NativeRollup contract on L1 accumulates ETH over time as users call `sendL1Message()` — it does not need to be pre-funded to match the L2 bridge premint.

**Gaps:** No support for custom gas tokens (ERC20, NFTs) — only native ETH. Relayer gas reimbursement is not yet implemented (future work). The relayer is permissioned (centralized sequencer model) for the PoC.

### L2 Fee Market

**Book:** Priority fees are collected by the rollup via a configurable `coinbase` address (exposed as an EXECUTE input). Base fees are burned per EIP-1559. The spec proposes exposing cumulative burned fees in the `block_output` so the L1 bridge contract can credit burned amounts to a designated address. DA cost handling is marked WIP.

**Us:** We verify the base fee computation from explicit parent fields (`parentBaseFee`, `parentGasLimit`, `parentGasUsed`) via EIP-1559 `calculate_base_fee_per_gas`. The coinbase is an explicit precompile input (individual field). Priority fees go to the coinbase address. We return `gasUsed`, `burnedFees` (`base_fee_per_gas * block_gas_used`), and `baseFeePerGas` in the precompile output — 160 bytes total. The NativeRollup contract on L1 tracks `blockGasLimit`, `lastBaseFeePerGas`, and `lastGasUsed` on-chain from previous block executions, so the relayer does not need to provide these values — they are read from contract storage and fed to the EXECUTE precompile automatically. The contract sends the burned fees amount to the relayer (`msg.sender`) when `advance()` is called.

**Gaps:** No DA cost mechanism. Burned fees are credited to the relayer on L1 but the L2-side crediting is not yet implemented.

### Transaction Type Filtering

**Book:** The precompile must reject type-3 blob-carrying transactions before calling the state transition function, since L2 blocks have no blob consensus layer. `BLOBHASH` and point evaluation precompile work identically to L1 blocks without blobs.

**Us:** We reject EIP-4844 blob transactions. We also reject ethrex-specific L2 types (Privileged L2 transactions, Fee token transactions) and blocks with validator withdrawals. Legacy, EIP-2930, EIP-1559, and EIP-7702 transactions are allowed.

**Match:** Aligned with the spec. Our additional rejection of ethrex-specific types is a superset of the spec's requirements.

### Block Execution Validation

**Book:** Both variants verify: (1) computed state root equals expected `post_state`, (2) computed receipts root equals expected `post_receipts`. Mismatches trigger `ExecuteError`.

**Us:** We verify both. State root is checked by comparing the `post_state_root` input field against the computed root after execution. Receipts root is checked by comparing the `post_receipts_root` input field against the root computed from execution receipts. Receipts are built for all transactions (including reverted ones, with empty logs).

**Match:** Aligned with the spec.

### RANDAO and Beacon Root

**Book:** `prev_randao` is left configurable — different rollups use different values (OP Stack uses latest L1 value, Orbit returns constant 1, Linea uses 2, etc.). `parent_beacon_block_root` is not universally supported since rollups lack beacon chain connections.

**Us:** `prev_randao` is an explicit individual field in the precompile input. We don't enforce any particular value.

**Match:** Compatible — we allow any value, which is consistent with the spec's "configurable" approach.

### Forced Transactions

**Book:** Describes mechanisms for censorship resistance: threshold detection (reject blocks if forced transactions from L1 queue are too old), storage/calldata reference (extend EXECUTE to read from L1 storage), and FOCIL (EIP-7805) inclusion lists from smart contracts. All marked as WIP.

**Us:** Not implemented. The PoC has no forced transaction mechanism.

**Gap:** Full gap — not in scope for Phase 1.

### Statelessness

**Book:** Lists statelessness (EIP-7864) as a critical dependency. L1 validators must verify precompile execution without storing L2 state, which requires execution witnesses.

**Us:** We use `ExecutionWitness` / `GuestProgramState` to provide stateless execution. The precompile receives a JSON-serialized witness containing the state trie, storage tries, and code for all accounts touched during execution. This enables the precompile to re-execute without persistent L2 state.

**Match:** Aligned — stateless execution via witness is implemented.

### Gas Metering

**Book:** Gas charging for the EXECUTE precompile is TBD. The precompile needs to meter execution properly to prevent DoS.

**Us:** We use a flat 100,000 gas cost (`EXECUTE_GAS_COST`). This is a placeholder — real metering would need to account for the cost of re-executing the L2 block.

**Gap:** Placeholder cost, not production-ready.

### Summary Table

| Aspect | Spec Status | Our Status | Alignment |
|--------|-------------|------------|-----------|
| EXECUTE variant | Two variants defined | `apply_body` with individual fields | Aligned |
| L1 anchoring | System contract + system tx | L1Anchor predeploy + system write | Aligned |
| L1→L2 messaging | Proof-based, no custom tx types | Merkle proof verification against anchored root | Aligned |
| L2→L1 messaging | State/receipts root proofs (WIP) | State root proofs (MPT account + storage) | Aligned |
| Gas token deposits | Preminted tokens in predeploy | Preminted L2Bridge + relayer | Aligned |
| L2 fee market | Configurable coinbase, burned fees | Base fee verified, coinbase as input field, burned fees tracked | Aligned |
| Transaction filtering | Reject blob txs | Reject blob + ethrex-specific txs | Aligned+ |
| State root validation | Required | Implemented | Aligned |
| Receipts root validation | Required | Implemented | Aligned |
| Base fee verification | From parent gas params | From explicit parent fields (EIP-1559) | Aligned |
| Parent gas tracking | Stored per block on L1 | On-chain: blockGasLimit, lastBaseFeePerGas, lastGasUsed | Aligned |
| RANDAO / beacon root | Configurable | Explicit individual field | Aligned |
| Forced transactions | WIP (FOCIL, threshold) | Not implemented | Gap |
| Statelessness | Required (EIP-7864) | ExecutionWitness-based | Aligned |
| Gas metering | TBD | Flat 100k gas | Placeholder |
| EXECUTE output format | TBD | 160 bytes (5 fields) | We defined |
| Serialization | Blob references for txs | Individual ABI fields + RLP tx list + JSON witness | Different |

## Limitations (Phase 1)

This PoC intentionally omits several things that would be needed for production:

- **Fixed gas cost** — Uses a flat 100,000 gas cost instead of real metering
- **No blob data support** — Only calldata-based input (spec proposes blob references for transactions)
- **No generic L1 block hash anchoring** — L1Anchor stores an L1 messages Merkle root, not a generic L1 block hash (which would enable broader cross-chain proofs)
- **No finality delay for withdrawals** — Withdrawals can be claimed immediately after the block is processed (production would require a challenge period)
- **No forced transaction mechanism** — No censorship resistance guarantees
- **L2 ETH supply drain** — EIP-1559 base fees are burned on every L2 transaction, permanently removing ETH from circulation. Burned fees are now tracked and credited to the relayer on L1 (via `advance()`), but the L2-side crediting mechanism is not yet implemented. A production solution would redirect burned fees to the bridge contract on L2 (similar to the OP Stack's `BaseFeeVault`)
- **No L2 production stack changes** — The existing L2 sequencer, OnChainProposer, and production genesis are unchanged; this PoC only adds the L1-side precompile and contracts
- **RLP transaction list** — Transactions are passed as an RLP-encoded list in ABI calldata (spec envisions blob-referenced transactions)

These are all Phase 2+ concerns.
