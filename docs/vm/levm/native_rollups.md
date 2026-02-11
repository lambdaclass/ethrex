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

**`execute_precompile.rs`** — The core precompile logic. Contains `execute_inner()` which orchestrates the full verification flow: witness deserialization, state root checks, deposit application, block execution, and final state root verification. Also contains helpers for block execution (`execute_block`), gas price calculation, withdrawal processing, and deposit application.

**`guest_program_state_db.rs`** — A thin adapter that implements LEVM's `Database` trait backed by `GuestProgramState`. This bridges the gap between the stateless execution witness (which provides account/storage/code data via tries) and LEVM's database interface. Uses a `Mutex` for interior mutability since `GuestProgramState` requires `&mut self` while `Database` methods take `&self`.

**`precompiles.rs` (modified)** — Registers the EXECUTE precompile at address `0x0101`, dispatched at runtime before the standard const precompile table lookup.

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

# In crates/vm/Cargo.toml
[features]
native-rollups = ["ethrex-levm/native-rollups"]
```

This ensures the precompile code is only compiled when explicitly opted in.

## Running the Test

```bash
cargo test -p ethrex-levm --features native-rollups --test native_rollups -- --nocapture
```

The integration test (`crates/vm/levm/tests/native_rollups.rs`) demonstrates the full flow:

1. Creates a genesis state: Alice (10 ETH), Bob (0), Charlie (0), Coinbase (0)
2. Signs an EIP-1559 transfer: Alice sends 1 ETH to Bob
3. Builds an `ExecutionWitness` from the state trie
4. Defines a deposit: 5 ETH to Charlie
5. Computes the expected post-state root (accounting for the transfer, gas costs, and deposit)
6. Calls `execute_inner()` with the witness, block, and deposits
7. The precompile re-executes the block, applies the deposit, and verifies the final state root matches

Expected output:

```
EXECUTE precompile succeeded!
  Pre-state root:  0x453c...5c13
  Post-state root: 0x615c...49de
  Alice sent 1 ETH to Bob
  Charlie received 5 ETH deposit
```

## Files

| File | Description |
|------|-------------|
| `crates/vm/levm/src/execute_precompile.rs` | EXECUTE precompile logic |
| `crates/vm/levm/src/db/guest_program_state_db.rs` | GuestProgramState → LEVM Database adapter |
| `crates/vm/levm/src/precompiles.rs` | Precompile registration (modified) |
| `crates/vm/levm/src/db/mod.rs` | Module export (modified) |
| `crates/vm/levm/src/lib.rs` | Module export (modified) |
| `crates/vm/levm/Cargo.toml` | Feature flag (modified) |
| `crates/vm/Cargo.toml` | Feature flag propagation (modified) |
| `crates/vm/levm/tests/native_rollups.rs` | Integration test |

## Limitations (Phase 1)

This PoC intentionally omits several things that would be needed for production:

- **No calldata deserialization** — `execute_inner()` is called directly with structured input; the `execute_precompile()` entrypoint stubs out calldata parsing
- **Fixed gas cost** — Uses a flat 100,000 gas cost instead of real metering
- **No blob data support** — Only calldata-based input
- **No anchoring predeploy** — Deposits modify state directly instead of going through a contract
- **No L2 contract integration** — OnChainProposer is unchanged
- **No L2 sequencer changes** — No integration with the L2 commit flow

These are all Phase 2+ concerns.
