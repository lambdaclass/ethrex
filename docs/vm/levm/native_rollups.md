# Native Rollups: Stateless Validation (EIP-8079 + EIP-8025)

## Overview

[EIP-8079](https://github.com/ethereum/EIPs/pull/9608) proposes "native rollups" — a mechanism where L1 verifies L2 state transitions by re-executing them inside the EVM via an `EXECUTE` precompile. This replaces complex proof systems (zkVM/fraud proofs) with direct execution, leveraging the fact that L1 already has an EVM capable of running the same transactions.

The spec defines the EXECUTE precompile as a thin wrapper around `verify_stateless_new_payload` — the same function the L1 ZK-EVM effort uses. Our implementation unifies this with EIP-8025 (Execution Layer Triggerable Proofs) under a single `stateless-validation` feature flag.

```
SSZ-encoded StatelessInput (NewPayloadRequest + ExecutionWitness + ChainConfig)
        |
  EXECUTE precompile (in LEVM) — thin wrapper
        |
  1. Deserialize SSZ to extract gas_used
  2. Charge gas proportional to L2 block gas_used
  3. Validate L2 constraints (no blobs, no withdrawals, no execution_requests)
  4. Delegate to StatelessValidator trait → verify_stateless_new_payload()
        |
        v
  verify_stateless_new_payload (in crates/blockchain/stateless.rs)
        |
  1. Compute hash_tree_root of NewPayloadRequest
  2. Validate block headers from witness
  3. Build GuestProgramState from witness
  4. Convert NewPayloadRequest → Block
  5. Execute block via LEVM
  6. Return StatelessValidationResult (SSZ)
        |
  Returns SSZ-encoded (new_payload_request_root, successful_validation, chain_config) or reverts
```

## EXECUTE Precompile

The [L2Beat native rollups book](https://native-rollup.l2beat.com/) defines two variants: `apply_body` (individual fields, skips header validation) and `state_transition` (full headers, complete STF). We implement `apply_body` — it receives the full block as part of an SSZ-encoded `NewPayloadRequest`, skips full header validation, and re-executes transactions to verify the resulting state root.

The core logic lives in `execute_precompile.rs`. It parses SSZ-encoded `StatelessInput`, charges gas proportional to the L2 block's `gas_used`, validates L2-specific constraints, and delegates to `verify_stateless_new_payload` via the `StatelessValidator` trait.

### Input Format

SSZ-encoded `StatelessInput`:

```rust
pub struct SszStatelessInput {
    pub new_payload_request: NewPayloadRequest,  // Full block as SSZ
    pub witness: SszExecutionWitness,            // State trie, storage tries, codes
    pub chain_config: SszChainConfig,            // chain_id
    pub public_keys: SszList<...>,               // Pre-recovered tx public keys (stub)
}
```

The `NewPayloadRequest` contains:
- `execution_payload`: Full SSZ `ExecutionPayload` (header fields + transactions + withdrawals)
- `versioned_hashes`: Blob hashes (empty for L2)
- `parent_beacon_block_root`: Used to carry the L1 messages Merkle root
- `execution_requests`: Deposit/withdrawal/consolidation requests (empty for L2)

### Output Format

SSZ-encoded `StatelessValidationResult`:

```rust
pub struct StatelessValidationResult {
    pub new_payload_request_root: [u8; 32],  // hash_tree_root of NewPayloadRequest
    pub successful_validation: bool,          // Whether execution was valid
    pub chain_config: SszChainConfig,         // chain_id (echo back)
}
```

The contract checks `successful_validation == true` at byte 32 of the result.

### L2 Constraint Validation

Before delegating to `verify_stateless_new_payload`, the precompile validates:

- `blob_gas_used == 0` — no blob data on L2
- `excess_blob_gas == 0` — no blob fee market on L2
- `withdrawals` is empty — L2 doesn't have consensus-layer withdrawals
- `execution_requests` is empty — no deposit/withdrawal/consolidation requests
- No type-3 (blob) transactions in the transaction list

### Gas Charging

Gas is charged proportional to the L2 block's `gas_used` field from the `ExecutionPayload`. This means the L1 gas cost of `advance()` scales linearly with the L2 block's computational complexity.

## L1 Anchoring

L1 messages are anchored via **`parent_beacon_block_root`** in the block header. The NativeBlockProducer on L2 sets this field to the Merkle root of consumed L1 messages. During L2 block processing, the EIP-4788 BEACON_ROOTS system contract stores this root at `0x000F3df6D732807Ef1319fB7B8bB8522d0Beac02`, making it available to L2 contracts.

> **Note:** The old L1Anchor predeploy at `0x00...fffe` has been removed. The BEACON_ROOTS approach is more spec-aligned — it uses standard Ethereum infrastructure (EIP-4788) instead of a custom predeploy.

## L1 to L2 Messaging

The L1→L2 message flow uses a relayer, a prefunded L2 bridge contract, and Merkle proof verification against the anchored L1 messages root:

1. Users call `NativeRollup.sendL1Message(to, gasLimit, data)` on L1 with ETH value — this records `keccak256(abi.encodePacked(from, to, value, gasLimit, keccak256(data), nonce))` as an L1 message hash in the contract's `pendingL1Messages` array
2. When `advance()` is called with `_l1MessagesCount`, the L2 advancer computes a **Merkle root** over the consumed L1 message hashes (commutative Keccak256)
3. The Merkle root is embedded in the `parent_beacon_block_root` field of the SSZ `NewPayloadRequest`
4. During L2 block processing, the EIP-4788 system contract stores this root at `BEACON_ROOTS_ADDRESS`
5. On L2, a relayer sends transactions calling `L2Bridge.processL1Message(from, to, value, gasLimit, data, nonce, merkleProof)`, which reads the root from BEACON_ROOTS, verifies the Merkle inclusion proof, then executes `to.call{value: value, gas: gasLimit}(data)`
6. The **state root check** at the end of execution implicitly guarantees correct message processing

**Block builder constraint:** The relayer chooses how many L1 messages to consume (`_l1MessagesCount` can be 0). The Merkle root is encoded in `parent_beacon_block_root` *before* the block transactions execute. The block builder must include the matching `processL1Message()` transactions — if they don't match, the state root will differ and the EXECUTE precompile will return `successful_validation = false`.

The relayer pays gas for L1 message transactions. L1 messages support arbitrary calldata, enabling not just ETH transfers but also arbitrary contract calls on L2.

## L2 to L1 Messaging (Withdrawals)

Withdrawals allow users to move ETH from L2 back to L1. The flow uses **state root proofs** — since the EXECUTE precompile verifies the post-state root, L2 contract storage can be proven directly against it:

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

Each withdrawal is uniquely identified by `keccak256(abi.encodePacked(from, receiver, amount, messageId))`. The `messageId` is a counter maintained by the L2Bridge contract (`withdrawalNonce`), starting at 0 and incrementing per withdrawal. The MPT proof verification lives in `MPTProof.sol` (trie traversal, RLP decoding, account/storage proof verification), which NativeRollup.sol imports as an internal library.

## Gas Token Deposits

The L2Bridge predeploy at `0x00...fffd` is deployed in genesis with a large preminted ETH balance (`U256::MAX / 2`) to cover all future L1 messages. A relayer calls `processL1Message()` to execute L1 messages (transferring ETH and/or executing calldata) from the bridge's balance. The relayer pays gas, solving the "first deposit problem" — users don't need gas to receive L1 messages. The NativeRollup contract on L1 accumulates ETH over time as users call `sendL1Message()`.

## Fee Market

EIP-1559 base fee is computed by the L2 node from the parent block's header fields. The coinbase is the relayer address. The NativeRollup contract on L1 does not track fee market parameters — this is handled entirely by the L2 node's standard block production logic, and the EXECUTE precompile re-derives the base fee from the block header in the `NewPayloadRequest`.

## Statelessness

The precompile receives an SSZ-encoded `SszExecutionWitness` containing the state trie, storage tries, and code for all accounts touched during execution. This enables the precompile to re-execute without persistent L2 state.

The `GuestProgramStateDb` adapter (`crates/vm/levm/src/db/guest_program_state_db.rs`) implements LEVM's `Database` trait backed by `GuestProgramState`, bridging the gap between the stateless execution witness and LEVM's database interface. This direct adapter is needed because `ethrex-levm` cannot depend on `ethrex-vm` (it's the other way around).

## Block Execution Validation

`verify_stateless_new_payload` (`crates/blockchain/stateless.rs`) performs the following:

1. Computes the SSZ `hash_tree_root` of the `NewPayloadRequest`
2. Validates block headers from the witness
3. Builds `GuestProgramState` from the witness
4. Converts the SSZ `NewPayloadRequest` → ethrex `Block`
5. Executes the block via LEVM
6. Returns `StatelessValidationResult` with the hash tree root and a `successful_validation` flag

The state root check implicitly guarantees both correct L1 message processing and correct L2→L1 withdrawal recording.

## Contracts

### NativeRollup.sol

L1 contract that manages L2 state on-chain. Storage layout:

| Slot | Field | Description |
|------|-------|-------------|
| 0 | `blockHash` | Current L2 block hash |
| 1 | `stateRoot` | Current L2 state root |
| 2 | `blockNumber` | Latest committed L2 block number |
| 3 | `gasLimit` | L2 block gas limit |
| 4 | `chainId` | L2 chain ID |
| 5 | `pendingL1Messages` | Array of L1 message hashes |
| 6 | `l1MessageIndex` | Next L1 message index to consume |
| 7 | `stateRootHistory` | `mapping(uint256 => bytes32)` — state root per block number |
| 8 | `claimedWithdrawals` | `mapping(bytes32 => bool)` — prevents double-claiming |
| 9 | `stateRootTimestamps` | `mapping(uint256 => uint256)` — commit timestamp per block |
| 10 | `_locked` | Reentrancy protection |

Immutables: `CHAIN_ID` (L2 chain ID) and `FINALITY_DELAY` (minimum seconds before withdrawals can be claimed, set to 0 for PoC).

Constructor: `constructor(bytes32 _initialStateRoot, bytes32 _initialBlockHash, uint256 _blockGasLimit, uint256 _chainId)`.

Functions:

- **`sendL1Message(address _to, uint256 _gasLimit, bytes _data)`** — payable; records `keccak256(abi.encodePacked(from, to, value, gasLimit, keccak256(data), nonce))` as an L1 message hash, burns `_gasLimit` gas on L1
- **`receive()`** — payable fallback; accepts ETH without recording an L1 message (used to fund the contract)
- **`advance(uint256 _l1MessagesCount, bytes _sszStatelessInput, bytes32 _newBlockHash, bytes32 _newStateRoot)`** — calls EXECUTE precompile with SSZ input, decodes `StatelessValidationResult`, updates on-chain state
- **`claimWithdrawal(address, address, uint256, uint256, uint256, bytes[], bytes[])`** — verifies MPT account + storage proofs against `stateRootHistory[blockNumber]`, enforces finality delay, transfers ETH
- **`computeMerkleRoot(uint256 startIdx, uint256 count)`** — view function to compute L1 messages Merkle root

### L2Bridge.sol

L2 predeploy at `0x00...fffd` handling both L1 message processing and withdrawals. Storage: slot 0 = relayer, slot 1 = l1MessageNonce, slot 2 = withdrawalNonce, slot 3 = `sentMessages` mapping.

- **`processL1Message(..., bytes32[] merkleProof)`** — verifies Merkle inclusion proof against the L1 messages root read from the EIP-4788 BEACON_ROOTS contract at `0x000F3df6D732807Ef1319fB7B8bB8522d0Beac02`, executes `to.call{value, gas}(data)`
- **`withdraw(address receiver)`** — payable; writes `sentMessages[hash] = true` for MPT proving on L1

### MPTProof.sol

Solidity library for MPT proof verification: trie traversal, RLP decoding, account proof verification (state root → storageRoot), and storage proof verification (storageRoot → mapping value). Compiled as an internal library (inlined by the compiler).

### merkle_tree.rs

`crates/l2/common/src/merkle_tree.rs` — Merkle tree using commutative Keccak256 hashing. Provides `compute_merkle_root()` and `compute_merkle_proof()`. Used by the L2 actors (block producer, advancer) and the L2 withdrawal proof RPC endpoint.

## Feature Flag

All native rollups and EIP-8025 code is gated behind the unified `stateless-validation` feature flag:

```toml
# crates/common/Cargo.toml
stateless-validation = ["dep:ssz", "dep:ssz-types", "dep:ssz-merkle", "dep:ssz-derive"]

# crates/blockchain/Cargo.toml
stateless-validation = [
    "ethrex-common/stateless-validation", "ethrex-vm/stateless-validation",
    "ethrex-guest-program/stateless-validation",
    "dep:ethrex-prover", "ethrex-prover?/stateless-validation",
    ...
]

# cmd/ethrex/Cargo.toml
stateless-validation = ["ethrex-blockchain/stateless-validation", "ethrex-rpc/stateless-validation", ...]
```

## Summary Table (vs March 2026 spec rewrite)

| Aspect | Spec | Us | Alignment |
|--------|------|-----|-----------|
| Core function | `verify_stateless_new_payload` | Implemented (blockchain/stateless.rs) | **Aligned** |
| Input format | SSZ `StatelessInput` | SSZ types via libssz | **Aligned** |
| Output format | SSZ `StatelessValidationResult` | Implemented | **Aligned** |
| Gas charging | `execution_payload.gas_used` | Implemented | **Aligned** |
| L2 preprocessing | Explicit layer (no blobs, no withdrawals) | Implemented | **Aligned** |
| Serialization | SSZ (execution-specs types) | SSZ via libssz | **Aligned** |
| L1 anchoring | `parent_beacon_block_root` (arbitrary bytes32) | Merkle root via `parent_beacon_block_root` | **Aligned** |
| L1→L2 messaging | Proof-based (no custom tx types) | Merkle proofs against BEACON_ROOTS | **Aligned** |
| L2→L1 messaging | State root proofs (MPT) | MPT account + storage proofs | **Aligned** |
| Gas token deposits | Preminted predeploy | L2Bridge with `U256::MAX / 2` | **Aligned** |
| No custom tx types | Design principle | Achieved (relayer txs) | **Aligned** |
| First deposit problem | WIP in spec | Solved (relayer pays gas) | **Ahead** |
| Contract state | blockHash, stateRoot, blockNumber, gasLimit, chainId | Implemented | **Aligned** |
| StatelessValidator trait | Implied (precompile wraps standard function) | Implemented in LEVM | **Aligned** |
| Cycle-free architecture | Implied | Trait injection pattern | **Aligned** |
| EIP-8025 integration | Separate concern | Unified under `stateless-validation` feature | **Ahead** |
| ZK variant | Specified (proof-carrying tx + PROOFROOT) | Not implemented (re-execution only) | **Gap (by design)** |
| Forced transactions | WIP (FOCIL) | Not implemented | **Gap** |
| DA cost pricing | WIP | Not implemented | **Both WIP** |
| `public_keys` | Pre-recovered tx keys | Empty tuple (stub) | **Stub** |

## Limitations

This PoC intentionally omits several things that would be needed for production:

- **ZK variant** — Only the re-execution variant is implemented. The ZK variant (proof-carrying transactions + PROOFROOT opcode) requires L1 consensus changes not yet available.
- **No forced transaction mechanism** — No censorship resistance guarantees (FOCIL integration is WIP in the spec)
- **L2 ETH supply drain** — Base fees are burned but not credited back on L2. A production solution would use a `BaseFeeVault` pattern.
- **No blob data support** — Only calldata-based input (spec proposes blob references via EIP-8142)
- **`public_keys` empty** — Pre-recovered transaction public keys are not populated yet
