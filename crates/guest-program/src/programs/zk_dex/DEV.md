# ZK-DEX Full Circuit — Development Document

## Overview

The ZK-DEX circuit (`DexCircuit`) runs inside the SP1 zkVM guest program and
proves batch state transitions for the ZkDex smart contract on L2. It replicates
the exact EVM state changes without re-verifying Groth16 proofs (the L2 EVM
already verified them, and `to == DEX_CONTRACT_ADDRESS` guarantees this).

## Supported Operations (8 total)

| Op | Type           | Selector Signature                                                         | Gas (ref) |
|----|----------------|---------------------------------------------------------------------------|-----------|
| 0  | TokenTransfer  | `transfer(address,address,uint256)`                                       | 65,000    |
| 1  | Mint           | `mint(uint256[2],uint256[2][2],uint256[2],uint256[4],bytes)`              | 150,000   |
| 2  | Spend          | `spend(uint256[2],uint256[2][2],uint256[2],uint256[5],bytes,bytes)`       | 200,000   |
| 3  | Liquidate      | `liquidate(address,uint256[2],uint256[2][2],uint256[2],uint256[4])`       | 150,000   |
| 4  | ConvertNote    | `convertNote(uint256[2],uint256[2][2],uint256[2],uint256[4],bytes)`       | 150,000   |
| 5  | MakeOrder      | `makeOrder(uint256,uint256,uint256[2],uint256[2][2],uint256[2],uint256[3])` | 200,000 |
| 6  | TakeOrder      | `takeOrder(uint256,uint256[2],uint256[2][2],uint256[2],uint256[6],bytes)` | 200,000   |
| 7  | SettleOrder    | `settleOrder(uint256,uint256[2],uint256[2][2],uint256[2],uint256[14],bytes)` | 300,000 |

> Gas costs are reference values. Actual gas is derived from the block header's
> `gas_used` field in `app_execution.rs`.

## File Structure

```
crates/guest-program/src/programs/zk_dex/
├── mod.rs       # Module declarations + witness analyzer (serialize_input)
├── circuit.rs   # DexCircuit: classify_tx, execute_operation, generate_logs
├── storage.rs   # Storage slot computation (notes, orders, encryptedNotes)
├── notes.rs     # Note operations: mint, spend, liquidate, convertNote
├── orders.rs    # Order operations: makeOrder, takeOrder, settleOrder
└── events.rs    # Event topic constants and Log generation
```

## Architecture

```
classify_tx(tx) → AppOperation { op_type, params }
    ↓
execute_operation(state, from, op) → OperationResult { success, data }
    ↓
generate_logs(from, op, result) → Vec<Log>
```

### Data Flow

1. **classify_tx** parses calldata, skips the 256-byte Groth16 proof, extracts
   public inputs and dynamic `bytes` parameters, packs them into `params`.

2. **execute_operation** routes to the appropriate handler in `notes.rs` or
   `orders.rs`, which reads/writes `AppState` storage slots.

3. **generate_logs** constructs EVM-compatible `Log` entries from `params` and
   `result.data` to match the Solidity contract's event emissions exactly.

## Storage Layout

Based on ZkDex's Solidity inheritance chain (`ZkDaiBase → MintNotes → SpendNotes → LiquidateNotes → ZkDai → ZkDex`):

| Slot | Variable              | Type                           |
|------|-----------------------|--------------------------------|
| 0    | development + dai     | bool + address (packed)        |
| 1    | requestVerifier       | address                        |
| 2    | (gap)                 | —                              |
| 3    | encryptedNotes        | mapping(bytes32 => bytes)      |
| 4    | notes                 | mapping(bytes32 => State)      |
| 5    | requestedNoteProofs   | mapping(bytes32 => bytes)      |
| 6    | verifiedProofs        | mapping(bytes32 => bool)       |
| 7    | mintNoteVerifier      | address                        |
| 8    | spendNoteVerifier     | address                        |
| 9    | liquidateNoteVerifier | address                        |
| 10   | convertNoteVerifier   | address                        |
| 11   | makeOrderVerifier     | address                        |
| 12   | takeOrderVerifier     | address                        |
| 13   | settleOrderVerifier   | address                        |
| 14   | orders                | Order[]                        |

> Verify with `forge inspect ZkDex storage-layout` before production use.

### Storage Slot Computation

```
notes[noteHash]       → keccak256(abi.encode(noteHash, 4))
encryptedNotes[hash]  → keccak256(abi.encode(hash, 3))    [length slot]
                      → keccak256(length_slot) + i         [data slots]
orders.length         → slot 14
orders[i].field_j     → keccak256(abi.encode(14)) + i*7 + j
```

### Solidity bytes Storage Encoding

- **Short** (≤31 bytes): `slot = data_left_aligned | (length * 2)`
- **Long** (≥32 bytes): `slot = length * 2 + 1`, data at `keccak256(slot) + i`

## Note State Machine

```
Invalid(0) ←── convertNote (smartNote)
    │
    ↓
Valid(1)   ←── mint, spend (newNote/changeNote), convertNote (newNote), settleOrder (new)
    │
    ├── spend → Spent(3)
    ├── liquidate → Spent(3)
    ├── makeOrder → Trading(2)
    │                  │
    │                  ├── takeOrder → Trading(2)  [parentNote, stakeNote]
    │                  │
    │                  └── settleOrder → Spent(3)  [makerNote, parentNote, takerNote]
    │
    └── settleOrder → Spent(3)

Trading(2) ←── makeOrder (makerNote), takeOrder (parentNote, stakeNote)
    │
    └── settleOrder → Spent(3)
```

**EMPTY_NOTE_HASH** = `0x0a47ead74da5372e7d2598e4f93c389bf03e8330219f8bf1e49b362f73491a26`
(Poseidon(0,0,0,0,0,0,0)) — used as sentinel in `spend` to skip empty positions.

## Order State Machine

```
Created(0) ← makeOrder
    │
    └── takeOrder → Taken(1)
                      │
                      └── settleOrder → Settled(2)
```

### Order Struct (7 fields, 7 slots per element)

| Offset | Field            | Type    |
|--------|------------------|---------|
| 0      | makerNote        | bytes32 |
| 1      | sourceToken      | uint256 |
| 2      | targetToken      | uint256 |
| 3      | price            | uint256 |
| 4      | takerNoteToMaker | bytes32 |
| 5      | parentNote       | bytes32 |
| 6      | state            | uint8   |

## Events

All events must match the Solidity contract exactly for receipt root consistency.

### NoteStateChange(bytes32 indexed note, State state)

Emitted by every note-mutating operation. `State` is `uint8` (0-3).

| Operation    | Events                                                    |
|-------------|-----------------------------------------------------------|
| mint        | NoteStateChange(noteHash, Valid)                          |
| spend       | NoteStateChange(old0, Spent) + ... (up to 4, conditional) |
| liquidate   | NoteStateChange(noteHash, Spent)                          |
| convertNote | NoteStateChange(smartNote, Invalid) + NoteStateChange(newNote, Valid) |
| makeOrder   | NoteStateChange(makerNote, Trading)                       |
| takeOrder   | NoteStateChange(parent, Trading) + NoteStateChange(stake, Trading) + OrderTaken |
| settleOrder | 3× NoteStateChange(new, Valid) + 3× NoteStateChange(old, Spent) + OrderSettled |

### OrderTaken(uint256 indexed orderId, bytes32 takerNoteToMaker, bytes32 parentNote)
### OrderSettled(uint256 indexed orderId, bytes32 rewardNote, bytes32 paymentNote, bytes32 changeNote)

## Calldata Parsing

All operations include a 256-byte Groth16 proof `(a[2] + b[2][2] + c[2])` which
is parsed but **ignored** — the L2 EVM already verified it.

Public inputs are extracted by position from the fixed-size `input[N]` array.
Dynamic `bytes` parameters are extracted using the ABI offset/length/data encoding.

### settleOrder Special Case

`settleOrder` reads `makerNote`, `parentNote`, `takerNoteToMaker` from order
storage (not from calldata inputs). These are passed to `generate_logs` via
`OperationResult.data` (96 bytes = 3 × H256).

The `encDatas` parameter is RLP-encoded as `[encReward, encPayment, encChange]`
and decoded using a lightweight inline RLP decoder.

## Witness Analyzer

`analyze_zk_dex_transactions()` in `mod.rs` determines which accounts and
storage slots need Merkle proofs. For each operation:

| Operation    | Accounts              | Storage Slots                              |
|-------------|----------------------|-------------------------------------------|
| transfer    | sender, recipient    | balances[token][sender], balances[token][to] |
| mint        | sender, DEX contract | notes[hash], encryptedNotes[hash] slots    |
| spend       | sender, DEX contract | up to 4 note slots + encrypted note slots  |
| liquidate   | sender, DEX, recipient | notes[hash]                              |
| convertNote | sender, DEX contract | 2 note slots + encrypted note slots        |
| makeOrder   | sender, DEX contract | note slot, orders.length                   |
| takeOrder   | sender, DEX contract | 2 note slots, order fields, enc slots      |
| settleOrder | sender, DEX contract | 3 note+enc slots, order fields             |

## ETH Balance Handling

- **mint**: `tx.value` transfers ETH from sender to contract (handled by `app_execution.rs`)
- **liquidate**: Contract → recipient ETH transfer (handled in `execute_liquidate`)
- **DAI**: Not yet supported (requires DAI contract storage proofs)

## Test Coverage (39 tests)

- `storage.rs`: 7 tests (slot computation, bytes encoding)
- `events.rs`: 3 tests (log format verification)
- `notes.rs`: 4 tests (mint, spend, liquidate, convertNote)
- `orders.rs`: 3 tests (makeOrder, takeOrder, RLP decoding)
- `circuit.rs`: 16 tests (classify_tx, execute, gas, logs)
- `mod.rs`: 5 tests (program metadata, ELF, serialization)

## TODO

- [ ] Verify storage slot numbers with `forge inspect ZkDex storage-layout`
- [ ] Measure actual EVM gas costs and update constants
- [ ] Add DAI token support for liquidate (requires DAI contract proofs)
- [ ] Add settleOrder old-note slots to witness analyzer (currently relies on ExecutionWitness)
- [ ] Docker E2E test: deploy ZkDex → call each function → SP1 prove → L1 verify
