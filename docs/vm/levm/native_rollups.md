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

## How to Test

All tests live in the shared test crate (`test/`) and require the `native-rollups` feature flag.

There are three levels of testing, from fastest to most comprehensive:

### 1. Unit tests (no infrastructure needed)

These tests call the EXECUTE precompile directly in-process — no L1 node, no network, no Solidity compilation. They verify the core precompile logic works correctly.

```bash
cargo test -p ethrex-test --features native-rollups -- levm::native_rollups --nocapture
```

This runs all 4 offline tests:

| Test | What it verifies |
|------|-----------------|
| `test_execute_precompile_transfer_and_deposit` | Full precompile flow: transfer + deposit + state root verification |
| `test_native_rollup_contract` | NativeRollup.sol running in LEVM calling the EXECUTE precompile |
| `test_execute_precompile_rejects_blob_transactions` | EIP-4844 blob transactions are rejected |
| `test_execute_precompile_rejects_withdrawals` | Blocks with withdrawals are rejected |

Expected output:

```
running 4 tests
test levm::native_rollups::test_execute_precompile_rejects_blob_transactions ... ok
test levm::native_rollups::test_execute_precompile_rejects_withdrawals ... ok
ABI-encoded EXECUTE calldata: 7648 bytes
EXECUTE precompile succeeded!
  Pre-state root:  0x453ce276913130ed26928c276ae51759ff45ba62c4ab3389452355d56a485c13
  Post-state root: 0x615cd8914a432a898d1d9998c8b8bce16c0bed49cd9e241cd3aca2ff41a449de
  Alice sent 1 ETH to Bob
  Charlie received 5 ETH deposit
test levm::native_rollups::test_execute_precompile_transfer_and_deposit ... ok
Deposit TX succeeded (5 ETH for charlie)
NativeRollup contract demo succeeded!
  L2 state transition verified via deposit() + advance():
    Pre-state root:  0x453ce276913130ed26928c276ae51759ff45ba62c4ab3389452355d56a485c13
    Post-state root: 0x615cd8914a432a898d1d9998c8b8bce16c0bed49cd9e241cd3aca2ff41a449de
    Block number:    1
    Deposit index:   1
  Gas used: 303139
test levm::native_rollups::test_native_rollup_contract ... ok

test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; ...
```

#### What the main test does (`test_execute_precompile_transfer_and_deposit`)

This test simulates a complete L2 block verification:

1. **Setup genesis state** — Creates 4 accounts: Alice (10 ETH), Bob (0), Charlie (0), Coinbase (0). Inserts them into a state trie and computes the pre-state root.
2. **Build L2 transaction** — Signs an EIP-1559 transfer: Alice sends 1 ETH to Bob (gas limit 21,000, 1 gwei priority fee, 2 gwei max fee).
3. **Compute expected post-state** — Calculates the final balances after the transfer (Alice loses 1 ETH + gas, Bob gains 1 ETH, Coinbase gets priority fee) and after the deposit (Charlie gains 5 ETH). Computes the expected post-state root.
4. **Build witness** — Creates an `ExecutionWitness` containing the state trie, chain config, and block headers. This is the minimal data needed to re-execute the block without full node state.
5. **Build ABI-encoded calldata** — Encodes `abi.encode(preStateRoot, blockRlp, witnessJson, deposits)` matching what the NativeRollup contract would produce.
6. **Call the precompile** — Invokes `execute_precompile()` directly with the calldata. Inside, the precompile parses the ABI data, rebuilds the state from the witness, applies the deposit, re-executes the block, and verifies the computed state root matches the expected one.
7. **Verify result** — Asserts the precompile returns `abi.encode(postStateRoot, blockNumber)` (64 bytes).

#### What the contract test does (`test_native_rollup_contract`)

This test runs the NativeRollup.sol contract inside LEVM, which in turn calls the EXECUTE precompile:

```
L1 tx1 -> NativeRollup.deposit(charlie) {value: 5 ETH}
  -> records PendingDeposit{charlie, 5 ETH} in contract storage

L1 tx2 -> NativeRollup.advance(1, blockRlp, witnessJson)
  -> reads stateRoot from slot 0
  -> pops 1 deposit from pendingDeposits array
  -> builds calldata: abi.encode(stateRoot, block, witness, depositsData)
  -> CALL to 0x0101 (EXECUTE precompile)
    -> precompile re-executes the L2 block and verifies state roots
  -> returns abi.encode(postStateRoot, blockNumber)
  -> contract decodes return value
  -> updates storage: stateRoot (slot 0), blockNumber (slot 1), depositIndex (slot 3)
```

The test verifies all three storage slots were updated correctly after `advance()` succeeds.

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
cargo test -p ethrex-test --features native-rollups -- l2::native_rollups --ignored --nocapture
```

Expected output:

```
Connected to L1 at http://localhost:8545
Deployer: 0x3d1e15a1a55578f7c920884a9943b3b35d0d885b
Compiler run successful. Artifact(s) can be found in directory ".../crates/vm/levm/contracts/solc_out".
NativeRollup deployed at: 0x...
  Deploy tx: 0x...
  Initial stateRoot verified: 0x453ce276913130ed26928c276ae51759ff45ba62c4ab3389452355d56a485c13
  deposit() tx: 0x...
  advance() tx: 0x...
  Gas used: 305970

NativeRollup integration test passed!
  Pre-state root:  0x453ce276913130ed26928c276ae51759ff45ba62c4ab3389452355d56a485c13
  Post-state root: 0x615cd8914a432a898d1d9998c8b8bce16c0bed49cd9e241cd3aca2ff41a449de
  Block number:    1
  Deposit index:   1
  Contract:        0x...
test l2::native_rollups::test_native_rollup_on_l1 ... ok
```

#### What the integration test does (`test_native_rollup_on_l1`)

1. **Connect to L1** — Creates an `EthClient` pointing at `localhost:8545` and loads the pre-funded signer from the Makefile's private key.
2. **Compile NativeRollup.sol** — Calls `solc` via `compile_contract()` to produce the deployment bytecode. This avoids hardcoding hex bytecode in the test.
3. **Deploy contract** — Sends a CREATE transaction with `deployBytecode + abi.encode(preStateRoot)` as constructor arg. Waits for the receipt and verifies deployment succeeded.
4. **Verify initial state** — Reads storage slot 0 via `eth_getStorageAt` and asserts it matches the pre-state root passed to the constructor.
5. **Deposit** — Sends `deposit(charlie)` with 5 ETH via `build_generic_tx` + `send_generic_transaction` (SDK helpers). Waits for receipt.
6. **Advance** — Sends `advance(1, blockRlp, witnessJson)` with gas estimated via `eth_estimateGas` (the precompile re-executes an entire L2 block). The calldata is built with `encode_calldata()` from the SDK. Waits for receipt.
7. **Verify final state** — Reads storage slots 0, 1, and 3 via RPC and asserts:
   - Slot 0 (`stateRoot`) = expected post-state root
   - Slot 1 (`blockNumber`) = 1
   - Slot 3 (`depositIndex`) = 1

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
| `test/tests/levm/native_rollups.rs` | Unit tests and contract-based test |
| `test/tests/l2/native_rollups.rs` | Integration test (requires running L1) |
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
