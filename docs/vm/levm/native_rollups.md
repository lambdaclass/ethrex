# Native Rollups: EXECUTE Precompile (PoC)

## Background

[EIP-8079](https://github.com/ethereum/EIPs/pull/9608) proposes "native rollups" — a mechanism where L1 verifies L2 state transitions by re-executing them inside the EVM via an `EXECUTE` precompile. This replaces complex proof systems (zkVM/fraud proofs) with direct execution, leveraging the fact that L1 already has an EVM capable of running the same transactions.

This is a Phase 1 proof-of-concept implementation that demonstrates the concept works at the EVM level.

## Architecture

```
L2 Block + ExecutionWitness + Deposits
        |
  EXECUTE precompile (in LEVM)
        |
  1. Build GuestProgramState from witness
  2. Verify pre-state root
  3. Apply deposits to state (anchor)
  4. Execute block transactions via LEVM
  5. Verify post-state root matches
        |
  Returns 0x01 (success) or reverts
```

### Components

**`execute_precompile.rs`** — The core precompile logic. Takes a single `ExecutePrecompileInput` containing one L2 block, its execution witness, deposits, and expected state roots. Orchestrates the full verification flow: witness deserialization, state root checks, deposit application, block execution, and final state root verification. Also contains helpers for block execution (`execute_block`), gas price calculation, transaction type validation, and deposit application.

**`guest_program_state_db.rs`** — A thin adapter that implements LEVM's `Database` trait backed by `GuestProgramState`. This bridges the gap between the stateless execution witness (which provides account/storage/code data via tries) and LEVM's database interface. Uses a `Mutex` for interior mutability since `GuestProgramState` requires `&mut self` while `Database` methods take `&self`.

**`precompiles.rs` (modified)** — Registers the EXECUTE precompile at address `0x0101`, dispatched at runtime before the standard const precompile table lookup.

**`NativeRollup.sol`** — A Solidity contract that simulates an L2 on-chain. Maintains `stateRoot` (slot 0) and `blockNumber` (slot 1). Exposes `advance(bytes32, uint256, bytes)` which calls the EXECUTE precompile at `0x0101` and updates its state on success. This demonstrates how an L1 contract would track and verify L2 state transitions.

### Why a Separate Database Adapter?

The existing `GuestProgramStateWrapper` in `crates/vm/witness_db.rs` bridges `GuestProgramState` to the `VmDatabase` trait, which then gets adapted to LEVM's `Database` trait via `DynVmDatabase`. However, `ethrex-levm` cannot depend on `ethrex-vm` (it's the other way around), so a direct adapter is needed. The `GuestProgramStateDb` is this direct bridge — about 100 lines, doing the same job without the intermediate layer.

### Deposit/Anchor Mechanism

Per EIP-8079, an "anchor" injects L1 data into L2 state before block execution. For this PoC:

- Anchor data is a list of `(address, amount)` deposits
- Before executing the block body, the precompile credits each deposit recipient's balance directly in the state trie
- The expected `post_state_root` must account for these credits
- No predeploy contract is needed — direct state modification suffices for the PoC

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

Legacy, EIP-2930, EIP-1559, and EIP-7702 transactions are allowed. Withdrawals are also rejected since native rollup L2 blocks don't have validator withdrawals.

### Calldata Serialization

The `execute_precompile()` entrypoint deserializes `ExecutePrecompileInput` from JSON (serde_json) calldata. A future version will define a proper ABI encoding/decoding scheme.

## Running the Tests

### In-process tests (no L1 needed)

```bash
cargo test -p ethrex-levm --features native-rollups --test native_rollups -- --nocapture
```

### Integration test (requires running L1)

Start the L1 with native-rollups enabled, then run the integration test:

```bash
# Terminal 1: start L1
NATIVE_ROLLUPS=1 make -C crates/l2 init-l1

# Terminal 2: run the integration test
cargo test -p ethrex-test --features native-rollups -- native_rollups_integration --ignored --nocapture
```

The integration test deploys the NativeRollup contract on L1, calls `advance()` with a valid L2 state transition, and verifies the contract's storage was updated.

## Test Descriptions

### Direct precompile test (`test_execute_precompile_transfer_and_deposit`)

1. Creates a genesis state: Alice (10 ETH), Bob (0), Charlie (0), Coinbase (0)
2. Signs an EIP-1559 transfer: Alice sends 1 ETH to Bob
3. Builds an `ExecutionWitness` from the state trie
4. Defines a deposit: 5 ETH to Charlie
5. Computes the expected post-state root (accounting for the transfer, gas costs, and deposit)
6. Calls `execute_inner()` with the witness, block, and deposits
7. The precompile re-executes the block, applies the deposit, and verifies the final state root matches

### NativeRollup contract test (`test_native_rollup_contract`)

Demonstrates the full end-to-end flow where an L1 contract calls the EXECUTE precompile:

1. Builds the same L2 state transition (Alice->Bob transfer + Charlie deposit)
2. Serializes `ExecutePrecompileInput` as JSON calldata
3. Deploys a NativeRollup contract on L1 with the pre-state root in storage
4. Executes an L1 transaction calling `advance(postStateRoot, 1, precompileInput)`
5. The NativeRollup contract CALLs the EXECUTE precompile at `0x0101`
6. Asserts the transaction succeeds and the contract's storage was updated

```
L1 tx -> NativeRollup.advance() -> CALL -> EXECUTE precompile (0x0101)
  -> deserialize JSON -> re-execute L2 block -> verify state roots -> 0x01
  -> NativeRollup updates stateRoot (slot 0) and blockNumber (slot 1)
```

### Integration test (`test_native_rollup_integration`)

Same flow as the NativeRollup contract test, but against a real running L1:

1. Deploys NativeRollup contract on L1 via `EthClient`
2. Sends `advance()` transaction with a valid L2 state transition
3. Reads storage slots via `eth_getStorageAt` to verify the contract updated correctly

### Rejection tests

- `test_execute_precompile_rejects_blob_transactions` — verifies EIP-4844 transactions are rejected
- `test_execute_precompile_rejects_withdrawals` — verifies non-empty withdrawals are rejected

### Expected output

```
EXECUTE precompile succeeded!
  Pre-state root:  0x453c...5c13
  Post-state root: 0x615c...49de
  Alice sent 1 ETH to Bob
  Charlie received 5 ETH deposit

Serialized EXECUTE calldata: 8848 bytes
NativeRollup contract test succeeded!
  L2 state transition verified via NativeRollup.advance():
    Pre-state root:  0x453c...5c13
    Post-state root: 0x615c...49de
  Gas used: ...
```

## Files

| File | Description |
|------|-------------|
| `crates/vm/levm/src/execute_precompile.rs` | EXECUTE precompile logic |
| `crates/vm/levm/src/db/guest_program_state_db.rs` | GuestProgramState -> LEVM Database adapter |
| `crates/vm/levm/src/precompiles.rs` | Precompile registration (modified) |
| `crates/vm/levm/src/db/mod.rs` | Module export (modified) |
| `crates/vm/levm/src/lib.rs` | Module export (modified) |
| `crates/vm/levm/Cargo.toml` | Feature flag (modified) |
| `crates/vm/Cargo.toml` | Feature flag propagation (modified) |
| `cmd/ethrex/Cargo.toml` | Feature flag for ethrex binary (modified) |
| `crates/vm/levm/contracts/NativeRollup.sol` | L2-simulator Solidity contract |
| `crates/vm/levm/tests/native_rollups.rs` | In-process tests |
| `test/tests/levm/native_rollups_integration.rs` | Integration test (requires running L1) |
| `test/Cargo.toml` | Feature flag for test crate (modified) |
| `crates/l2/Makefile` | `init-l1` supports `NATIVE_ROLLUPS=1` (modified) |

## Limitations (Phase 1)

This PoC intentionally omits several things that would be needed for production:

- **Fixed gas cost** — Uses a flat 100,000 gas cost instead of real metering
- **JSON serialization** — Uses serde_json for calldata; a production version would use proper ABI encoding
- **No blob data support** — Only calldata-based input
- **No anchoring predeploy** — Deposits modify state directly instead of going through a contract
- **No L2 contract integration** — OnChainProposer is unchanged
- **No L2 sequencer changes** — No integration with the L2 commit flow

These are all Phase 2+ concerns.
