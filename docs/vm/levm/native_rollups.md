# Native Rollups

## Overview

[EIP-8079](https://eips.ethereum.org/EIPS/eip-8079) proposes "native rollups" — a mechanism where L1 verifies L2 state transitions by re-executing them inside the EVM via an `EXECUTE` precompile. This replaces complex proof systems (zkVM/fraud proofs) with direct execution, leveraging the fact that L1 already has an EVM capable of running the same transactions.

The spec defines the EXECUTE precompile as a thin wrapper around `verify_stateless_new_payload` — the same function the L1 ZK-EVM effort uses. Our implementation reuses the stateless-validation infrastructure landed with EIP-8025 (#6427). The EXECUTE precompile code is **always-compiled**; the precompile fires only at runtime when `fork >= Fork::LStar && vm_type == VMType::L1`.

> **Implementation note — spec divergences:** The implementation tracks the [l2beat native-rollups spec book](https://github.com/l2beat/l2beat) rather than the published EIP-8079 text. The two diverge in three places:
>
> - **Anchoring**: l2beat uses `parent_beacon_block_root` as an arbitrary `bytes32` to carry the L1 messages Merkle root. Published EIP-8079 specifies an `ANCHOR_ADDRESS` precompile for this purpose.
> - **EXECUTE signature**: The l2beat signature passes `(uint16 _l1MessagesCount, bytes _sszStatelessInput)`. EIP-8079 specifies a different calling convention.
> - **`burned_fees`**: EIP-8079 makes `burned_fees` a new execution-payload header field. Our implementation includes it as a recompute-only header field gated at `Fork::LStar` (EIP-8079-aligned, §G).
>
> Where EIP-8079 and the l2beat spec agree, "EIP-8079 conformance" also means l2beat conformance.

```
SSZ-encoded StatelessInput (NewPayloadRequest + ExecutionWitness + ChainConfig)
        |
  EXECUTE precompile (in LEVM) — thin wrapper
        |
  1. Deserialize SSZ once (read gas_limit; keep the decoded input)
  2. Validate L2 constraints (no blobs, no withdrawals, no execution_requests)
  3. Charge gas = gas_limit + calldata_len·EXECUTE_GAS_PER_WITNESS_BYTE
  4. Delegate the decoded input to StatelessValidator → verify_stateless_new_payload()
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

The core logic lives in `execute_precompile.rs`. It parses SSZ-encoded `StatelessInput` once, validates L2-specific constraints, charges gas (`gas_limit + calldata_len · EXECUTE_GAS_PER_WITNESS_BYTE`), and delegates the already-decoded input to `verify_stateless_new_payload` via the `StatelessValidator` trait. Gas is charged on `gas_limit` (the true upper bound on re-execution), never on the attacker-controlled `gas_used`.

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

Gas is charged as:

```
charge = gas_limit + calldata_len · EXECUTE_GAS_PER_WITNESS_BYTE
```

where `gas_limit` is from `ExecutionPayload.gas_limit` and `calldata_len` is the length of the raw `_sszStatelessInput` bytes. The precompile **fails closed** (CALL-level `ExceptionalHalt`) if the input is invalid or the charge cannot be applied — the `advance()` call reverts rather than committing an unvalidated state.

## L1 Anchoring

L1 messages are anchored via **`parent_beacon_block_root`** in the block header. The NativeBlockProducer on L2 sets this field to the Merkle root of consumed L1 messages. During L2 block processing, the EIP-4788 BEACON_ROOTS system contract stores this root at `0x000F3df6D732807Ef1319fB7B8bB8522d0Beac02`, making it available to L2 contracts.

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

EIP-1559 base fee is computed by the L2 node from the parent block's header fields. The NativeRollup contract on L1 does not track fee market parameters — this is handled entirely by the L2 node's standard block production logic, and the EXECUTE precompile re-derives the base fee from the block header in the `NewPayloadRequest`. In this PoC, the demo wires the coinbase to the relayer address; a production deployment would choose its own coinbase policy.

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
| 3 | `l2GasLimit` | L2 block gas limit |
| 4 | `chainId` | L2 chain ID |
| 5 | `pendingL1Messages` | Array of L1 message hashes |
| 6 | `l1MessageIndex` | Next L1 message index to consume |
| 7 | `totalDeposited` | Total ETH bridged in via `sendL1Message` (escrow solvency) |
| 8 | `totalClaimed` | Total ETH paid out by `claimWithdrawal` |
| 9 | `stateRootHistory` | `mapping(uint256 => bytes32)` — state root per block number |
| 10 | `claimedWithdrawals` | `mapping(bytes32 => bool)` — prevents double-claiming |
| 11 | `stateRootTimestamps` | `mapping(uint256 => uint256)` — commit timestamp per block |
| 12 | `_locked` | Reentrancy protection |
| 13 | `lastFetchedL1Block` | Deploy block; seeds the L1 watcher cursor |
| 14 | `advancer` | Address authorized to call `advance()` |
| 15 | `pendingAdvancer` | Nominated next advancer, pending acceptance (two-step handoff) |

`CHAIN_ID` and `FINALITY_DELAY` are `immutable` (stored in code, not storage). `FINALITY_DELAY` is the minimum seconds a state root must age before its withdrawals can be claimed; it is set at deploy time from `--native-rollups.finality-delay` (default 0 = instant finality for the local demo; production must pass a reorg-safe value).

Constructor: `constructor(bytes32 _initialStateRoot, bytes32 _initialBlockHash, uint256 _blockGasLimit, uint256 _chainId, address _advancer, uint256 _finalityDelay)`.

Functions:

- **`sendL1Message(address _to, uint256 _gasLimit, bytes _data)`** — payable; records `keccak256(abi.encodePacked(from, to, value, gasLimit, keccak256(data), nonce))` as an L1 message hash, burns `_gasLimit` gas on L1
- **`receive()`** — payable fallback; accepts ETH without recording an L1 message (used to fund the contract)
- **`advance(uint16 _l1MessagesCount, bytes _sszStatelessInput)`** — calls the EXECUTE precompile, decodes the block-level fields and the L1 messages Merkle root (`parent_beacon_block_root`) from the SSZ input, recomputes the root over `pendingL1Messages[l1MessageIndex .. +count]` and reverts on mismatch, then commits the new L2 state. Mirrors `OnChainProposer.commitBatch`'s `processedPrivilegedTransactionsRollingHash` check.
- **`claimWithdrawal(address, address, uint256, uint256, uint256, bytes[], bytes[])`** — verifies MPT account + storage proofs against `stateRootHistory[blockNumber]`, enforces finality delay, transfers ETH
- **`getPendingL1MessagesRoot(uint16 number)`** — view; recomputes the Merkle root over the next `number` unconsumed L1 messages, mirroring `CommonBridge.getPendingTransactionsVersionedHash`

### L2Bridge.sol

L2 predeploy at `0x00...fffd` handling both L1 message processing and withdrawals. Storage: slot 0 = relayer, slot 1 = l1MessageNonce, slot 2 = withdrawalNonce, slot 3 = `sentMessages` mapping.

- **`processL1Message(..., bytes32[] merkleProof)`** — verifies Merkle inclusion proof against the L1 messages root read from the EIP-4788 BEACON_ROOTS contract at `0x000F3df6D732807Ef1319fB7B8bB8522d0Beac02`, executes `to.call{value, gas}(data)`
- **`withdraw(address receiver)`** — payable; writes `sentMessages[hash] = true` for MPT proving on L1

### MPTProof.sol

Solidity library for MPT proof verification: trie traversal, RLP decoding, account proof verification (state root → storageRoot), and storage proof verification (storageRoot → mapping value). Compiled as an internal library (inlined by the compiler).

### merkle_tree.rs

`crates/l2/common/src/merkle_tree.rs` — Merkle tree using commutative Keccak256 hashing. Provides `compute_merkle_root()` and `compute_merkle_proof()`. Used by the L2 actors (block producer, advancer) and the L2 withdrawal proof RPC endpoint.

## Compilation and Feature Flags

The native-rollup EXECUTE precompile, the stateless-validation pipeline, and all SSZ types are **always-compiled** — there is no cargo feature that gates them out. (The old unified devnet feature flag was removed.)

**Runtime gate:** the EXECUTE precompile body executes only when `fork >= Fork::LStar && vm_type == VMType::L1`. On any other fork or VM type the precompile is a no-op CALL.

The only remaining cargo feature is **`eip-8025`**, which controls the guest-program input/output format. The SSZ dep chain is:

```toml
# crates/guest-program/Cargo.toml  (guest only — not the host)
eip-8025 = [
    "ethrex-common/eip-8025",
    "ethrex-vm/eip-8025",
    "dep:libssz",
    "dep:libssz-merkle",
    "dep:libssz-types",
    "dep:libssz-derive",
]

# crates/blockchain/Cargo.toml
eip-8025 = ["ethrex-common/eip-8025", "ethrex-vm/eip-8025"]

# crates/common/Cargo.toml
eip-8025 = ["ethrex-trie/eip-8025"]   # guest-only trie SSZ support; the always-compiled stateless SSZ types do not need it
```

When `eip-8025` is compiled into the guest program:
- Input format: `NewPayloadRequest` (SSZ) + `ExecutionWitness` (rkyv) — the EIP-8025 wire format
- Output format: `ProgramOutput { new_payload_request_root: [u8; 32], valid: bool }` — 33 bytes

Without `eip-8025`:
- Input/output use the legacy rkyv `ProgramInput`/`ProgramOutput` format
- The host (node) always has the SSZ types compiled in for the EXECUTE precompile

## Summary Table (vs l2beat native-rollups spec; see divergences note above)

| Aspect | l2beat spec | Us | Alignment |
|--------|------------|-----|-----------|
| Core function | `verify_stateless_new_payload` | Implemented (blockchain/stateless.rs) | **Aligned** |
| Input format | SSZ `StatelessInput` | SSZ types via libssz | **Aligned** |
| Output format | SSZ `StatelessValidationResult` | Implemented | **Aligned** |
| Gas charging | `gas_limit + calldata·EXECUTE_GAS_PER_WITNESS_BYTE` | Implemented | **Aligned** |
| Fail-closed on invalid input | CALL-level ExceptionalHalt | Implemented | **Aligned** |
| L2 preprocessing | Explicit layer (no blobs, no withdrawals) | Implemented | **Aligned** |
| Serialization | SSZ (execution-specs types) | SSZ via libssz | **Aligned** |
| L1 anchoring | `parent_beacon_block_root` (arbitrary bytes32) | Merkle root via `parent_beacon_block_root` | **Aligned** |
| L1→L2 messaging | Proof-based (no custom tx types) | Merkle proofs against BEACON_ROOTS | **Aligned** |
| L2→L1 messaging | State root proofs (MPT) | MPT account + storage proofs | **Aligned** |
| Gas token deposits | Preminted predeploy | L2Bridge with `U256::MAX / 2` | **Aligned** |
| No custom tx types | Design principle | Achieved (relayer txs) | **Aligned** |
| First deposit problem | WIP in spec | Solved (relayer pays gas) | **Ahead** |
| `burned_fees` header field | EIP-8079 header field (§G) | Implemented — LStar-gated recompute-only field | **Aligned** |
| Contract state | blockHash, stateRoot, blockNumber, gasLimit, chainId | Implemented | **Aligned** |
| StatelessValidator trait | Implied (precompile wraps standard function) | Implemented in LEVM | **Aligned** |
| Cycle-free architecture | Implied | Trait injection pattern | **Aligned** |
| ZK variant | Specified (proof-carrying tx + PROOFROOT) | Not implemented (re-execution only) | **Gap (by design)** |
| Forced transactions | WIP (FOCIL) | Not implemented | **Gap** |
| DA cost pricing | WIP | Not implemented | **Both WIP** |
| `public_keys` | Pre-recovered tx keys | Empty tuple (stub) | **Stub** |

### EIP-8079 divergences

| Aspect | Published EIP-8079 | Our implementation |
|--------|-------------------|-------------------|
| L1 anchoring mechanism | `ANCHOR_ADDRESS` precompile | `parent_beacon_block_root` field (l2beat model) |
| EXECUTE calling convention | EIP-8079 calling convention | `advance(uint16 _l1MessagesCount, bytes _sszStatelessInput)` (l2beat model) |
| `burned_fees` | New execution-payload header field | Implemented as LStar-gated recompute-only field (matches EIP-8079 §G) |

## Limitations

This PoC intentionally omits several things that would be needed for production:

- **ZK variant** — Only the re-execution variant is implemented. The ZK variant (proof-carrying transactions + PROOFROOT opcode) requires L1 consensus changes not yet available.
- **No forced transaction mechanism** — No censorship resistance guarantees (FOCIL integration is WIP in the spec)
- **No L1 message inclusion deadline** — `pendingL1Messages` have no per-message deadline, so the advancer can defer processing them indefinitely without on-chain consequence. `OnChainProposer.sol` enforces this for privileged transactions via `PRIVILEGED_TX_MAX_WAIT_BEFORE_INCLUSION` + `hasExpiredPrivilegedTransactions()`; the analogous mechanism for `NativeRollup.sol` is a TODO.
- **L2 ETH supply drain** — Base fees are burned but not credited back on L2. A production solution would use a `BaseFeeVault` pattern.
- **No blob data support** — Only calldata-based input (spec proposes blob references via EIP-8142)
- **`public_keys` empty** — Pre-recovered transaction public keys are not populated yet
