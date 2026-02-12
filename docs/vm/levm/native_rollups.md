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
  1. Parse ABI-encoded calldata (pre-state root, block RLP, witness JSON, deposits)
  2. Build GuestProgramState from witness
  3. Verify pre-state root
  4. Apply deposits to state (anchor)
  5. Execute block transactions via LEVM
  6. Verify post-state root (from block header) matches computed root
        |
  Returns abi.encode(postStateRoot, blockNumber) or reverts
```

### Components

**`execute_precompile.rs`** — The core precompile logic. Parses ABI-encoded calldata containing the pre-state root, an RLP-encoded block, a JSON-serialized witness, and packed deposit data. Orchestrates the full verification flow: calldata parsing, state root checks, deposit application, block execution, and final state root verification. The post-state root and block number are extracted from the block header and returned as `abi.encode(postStateRoot, blockNumber)`. Also contains helpers for block execution (`execute_block`), gas price calculation, transaction type validation, and deposit application.

**`guest_program_state_db.rs`** — A thin adapter that implements LEVM's `Database` trait backed by `GuestProgramState`. This bridges the gap between the stateless execution witness (which provides account/storage/code data via tries) and LEVM's database interface. Uses a `Mutex` for interior mutability since `GuestProgramState` requires `&mut self` while `Database` methods take `&self`.

**`precompiles.rs` (modified)** — Registers the EXECUTE precompile at address `0x0101`, dispatched at runtime before the standard const precompile table lookup.

**`NativeRollup.sol`** — A Solidity contract that manages L2 state on-chain. Maintains `stateRoot` (slot 0), `blockNumber` (slot 1), a `pendingDeposits` array (slot 2), and `depositIndex` (slot 3). Exposes:
- `deposit(address)` — payable function that records pending deposits
- `receive()` — payable fallback that deposits for `msg.sender`
- `advance(uint256, bytes, bytes)` — consumes pending deposits, builds ABI-encoded precompile calldata via `abi.encode(stateRoot, _block, _witness, depositsData)`, calls EXECUTE at `0x0101`, decodes the returned `(postStateRoot, blockNumber)`, and updates state

### Why a Separate Database Adapter?

The existing `GuestProgramStateWrapper` in `crates/vm/witness_db.rs` bridges `GuestProgramState` to the `VmDatabase` trait, which then gets adapted to LEVM's `Database` trait via `DynVmDatabase`. However, `ethrex-levm` cannot depend on `ethrex-vm` (it's the other way around), so a direct adapter is needed. The `GuestProgramStateDb` is this direct bridge — about 100 lines, doing the same job without the intermediate layer.

### Deposit/Anchor Mechanism

Per EIP-8079, an "anchor" injects L1 data into L2 state before block execution. The deposit flow works as follows:

1. Users call `NativeRollup.deposit(recipient)` with ETH value — this records a `PendingDeposit{recipient, amount}` in the contract's storage array
2. When `advance()` is called with `_depositsCount`, it pops that many deposits from the queue (advancing `depositIndex`) and includes them in the binary calldata sent to the EXECUTE precompile
3. The precompile parses the deposits from calldata and credits each recipient's balance directly in the state trie before executing the block
4. The expected `post_state_root` must account for these credits

### Calldata Format (ABI-encoded)

The EXECUTE precompile uses standard ABI encoding:

```
abi.encode(bytes32 preStateRoot, bytes blockRlp, bytes witnessJson, bytes deposits)

Offset  Contents
0x00    bytes32 preStateRoot           (static, 32 bytes)
0x20    uint256 offset_to_block        (points to block data)
0x40    uint256 offset_to_witness      (points to witness data)
0x60    uint256 offset_to_deposits     (points to deposits data)
0x80+   dynamic data:
          block:    [32 length][block RLP bytes][padding]
          witness:  [32 length][witness JSON bytes][padding]
          deposits: [32 length][packed deposit data: (20 addr + 32 amount) * N][padding]
```

### Return Value

```
abi.encode(bytes32 postStateRoot, uint256 blockNumber)  — 64 bytes
```

The post-state root is extracted from `block.header.state_root` and verified against the computed state root after execution. The block number is extracted from `block.header.number`.

### Encoding Details

- **Block** uses RLP encoding (already implemented in ethrex via `RLPEncode`/`RLPDecode`)
- **ExecutionWitness** uses JSON because it doesn't have RLP support — it uses serde/rkyv for serialization instead
- **Deposits** use packed binary encoding: each deposit is 52 bytes (20-byte address + 32-byte amount)

The NativeRollup contract fills in `preStateRoot` from its own storage, deposits from its pending queue, and passes through block/witness bytes unchanged (opaque to the contract). The contract decodes the precompile's return value to extract the new state root and block number.

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

The integration test deploys the NativeRollup contract on L1, deposits 5 ETH for Charlie, calls `advance()` with a valid L2 state transition, and verifies the contract's storage was updated (stateRoot, blockNumber, depositIndex).

## Test Descriptions

### Direct precompile test (`test_execute_precompile_transfer_and_deposit`)

1. Creates a genesis state: Alice (10 ETH), Bob (0), Charlie (0), Coinbase (0)
2. Signs an EIP-1559 transfer: Alice sends 1 ETH to Bob
3. Builds an `ExecutionWitness` from the state trie
4. Defines a deposit: 5 ETH to Charlie
5. Computes the expected post-state root (accounting for the transfer, gas costs, and deposit)
6. Builds ABI-encoded calldata (`abi.encode(preStateRoot, blockRlp, witnessJson, deposits)`)
7. Calls `execute_precompile()` with the ABI-encoded calldata
8. The precompile parses the calldata, re-executes the block, applies the deposit, and verifies the final state root
9. Asserts the return value is `abi.encode(postStateRoot, blockNumber)` (64 bytes)

### NativeRollup contract test (`test_native_rollup_contract`)

Demonstrates the full end-to-end flow with deposit + advance:

1. Builds the L2 state transition (Alice->Bob transfer + Charlie deposit)
2. Deploys a NativeRollup contract on L1 with the pre-state root in storage
3. Executes a deposit TX: `deposit(charlie)` with 5 ETH
4. Executes an advance TX: `advance(1, blockRlp, witnessJson)`
5. The contract builds ABI-encoded calldata from its stored state root, deposits, and the parameters
6. The contract CALLs the EXECUTE precompile at `0x0101`
7. The contract decodes the returned `(postStateRoot, blockNumber)` and updates its state
8. Asserts the transaction succeeds and the contract's storage was updated (stateRoot, blockNumber, depositIndex)

```
L1 tx1 -> NativeRollup.deposit(charlie) {value: 5 ETH}
  -> records PendingDeposit in storage

L1 tx2 -> NativeRollup.advance(1, blockRlp, witnessJson)
  -> builds ABI-encoded calldata: abi.encode(stateRoot, block, witness, depositsData)
  -> CALL -> EXECUTE precompile (0x0101)
  -> parse ABI calldata -> re-execute L2 block -> verify state roots
  -> returns abi.encode(postStateRoot, blockNumber)
  -> NativeRollup decodes return, updates stateRoot (slot 0), blockNumber (slot 1), depositIndex (slot 3)
```

### Integration test (`test_native_rollup_on_l1`)

Same flow as the NativeRollup contract test, but against a real running L1:

1. Deploys NativeRollup contract on L1 via `EthClient`
2. Sends `deposit(charlie)` transaction with 5 ETH
3. Sends `advance()` transaction with a valid L2 state transition
4. Reads storage slots via `eth_getStorageAt` to verify the contract updated correctly (stateRoot, blockNumber, depositIndex)

### Rejection tests

- `test_execute_precompile_rejects_blob_transactions` — verifies EIP-4844 transactions are rejected
- `test_execute_precompile_rejects_withdrawals` — verifies non-empty withdrawals are rejected

### Expected output

```
ABI-encoded EXECUTE calldata: ... bytes
EXECUTE precompile succeeded!
  Pre-state root:  0x453c...5c13
  Post-state root: 0x615c...49de
  Alice sent 1 ETH to Bob
  Charlie received 5 ETH deposit

Deposit TX succeeded (5 ETH for charlie)
NativeRollup contract demo succeeded!
  L2 state transition verified via deposit() + advance():
    Pre-state root:  0x453c...5c13
    Post-state root: 0x615c...49de
    Block number:    1
    Deposit index:   1
  Gas used: ...
```

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
| `crates/vm/levm/contracts/NativeRollup.sol` | L2-simulator Solidity contract (with deposit mechanism) |
| `crates/vm/levm/tests/native_rollups.rs` | In-process tests |
| `test/tests/levm/native_rollups_integration.rs` | Integration test (requires running L1) |
| `test/Cargo.toml` | Feature flag for test crate (modified) |
| `crates/l2/Makefile` | `init-l1` supports `NATIVE_ROLLUPS=1` (modified) |

## Limitations (Phase 1)

This PoC intentionally omits several things that would be needed for production:

- **Fixed gas cost** — Uses a flat 100,000 gas cost instead of real metering
- **No blob data support** — Only calldata-based input
- **No anchoring predeploy** — Deposits modify state directly instead of going through a contract
- **No L2 contract integration** — OnChainProposer is unchanged
- **No L2 sequencer changes** — No integration with the L2 commit flow
- **Hybrid serialization** — ABI envelope + RLP block + JSON witness (ExecutionWitness lacks RLP support)

These are all Phase 2+ concerns.
