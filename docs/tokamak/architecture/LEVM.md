# LEVM (Lambda EVM) Deep Analysis

*Source: `crates/vm/levm/` | Analyzed: 2026-02-22*

## VM Struct

**Location**: `src/vm.rs:388-415`

```rust
pub struct VM<'a> {
    pub call_frames: Vec<CallFrame>,          // Stack of parent call frames (nested calls)
    pub current_call_frame: CallFrame,         // Currently executing call frame
    pub env: Environment,                      // Block and transaction environment
    pub substate: Substate,                    // Accessed addresses, logs, refunds, etc.
    pub db: &'a mut GeneralizedDatabase,       // Account state read/write
    pub tx: Transaction,                       // Transaction being executed
    pub hooks: Vec<Rc<RefCell<dyn Hook>>>,     // Execution hooks (tracing, debugging)
    pub storage_original_values: FxHashMap<(Address, H256), U256>,  // For SSTORE gas calc
    pub tracer: LevmCallTracer,                // Call tracing
    pub debug_mode: DebugMode,                 // Dev diagnostics
    pub stack_pool: Vec<Stack>,                // Reusable stack allocations
    pub vm_type: VMType,                       // L1 or L2(FeeConfig)
    pub(crate) opcode_table: [OpCodeFn<'a>; 256],  // Fork-gated dispatch table
}
```

## Transaction Execution Flow

```
Evm::execute_block() [crates/vm/src/lib.rs]
  └── LEVM::execute_block()
        ├── prepare_block()                    — System contract calls (EIP-2935, 4788, 7002, 7251)
        └── for each tx:
              └── execute_tx()
                    └── VM::new() → vm.execute()

VM::execute() [vm.rs:493-525]
  ├── prepare_execution()                      — Run all hooks' prepare_execution()
  │     └── hooks[].prepare_execution(vm)      — DefaultHook: validate tx, deduct gas, nonce++
  ├── clear callframe backup                   — Changes from prepare are permanent
  ├── EIP-7928 BAL checkpoint                  — Block Access List recording
  ├── handle_create_transaction() (if CREATE)  — Check address collision
  ├── substate.push_backup()                   — Checkpoint for revert
  ├── run_execution()                          — Main opcode loop
  └── finalize_execution(result)               — Run hooks' finalize_execution(), gas refund

VM::stateless_execute() [vm.rs:688]           — Execute without modifying cache (for eth_call)
  ├── add BackupHook
  ├── execute()
  └── db.undo_last_transaction()
```

## Main Execution Loop

**Location**: `src/vm.rs:528-663`

The loop uses a **dual dispatch** strategy for performance:

### 1. Inline Fast Path (compile-time match)

The most frequently executed opcodes are matched directly in a `match` statement. This allows the compiler to inline them and avoid function pointer overhead:

- `PUSH1-PUSH32` (0x60-0x7f) — const-generic `op_push::<N>()`
- `DUP1-DUP16` (0x80-0x8f) — const-generic `op_dup::<N>()`
- `SWAP1-SWAP16` (0x90-0x9f) — const-generic `op_swap::<N>()`
- `ADD` (0x01), `CODECOPY` (0x39), `MLOAD` (0x51)
- `JUMP` (0x56), `JUMPI` (0x57), `JUMPDEST` (0x5b)
- `TSTORE` (0x5d) — fork-gated: `>= Cancun`

### 2. Table Fallback (runtime dispatch)

All other opcodes fall through to `opcode_table[opcode as usize].call(self)`, a 256-entry function pointer table built dynamically per fork.

### Loop Structure

```rust
loop {
    let opcode = self.current_call_frame.next_opcode();
    self.advance_pc(1)?;

    // [perf_opcode_timings]: start timer

    let op_result = match opcode {
        // Fast path: inline match for hot opcodes
        0x60 => self.op_push::<1>(),
        // ... PUSH/DUP/SWAP/common opcodes ...
        _ => self.opcode_table[opcode as usize].call(self),
    };

    // [perf_opcode_timings]: record elapsed time

    match op_result {
        Ok(OpcodeResult::Continue) => continue,
        Ok(OpcodeResult::Halt) => handle_opcode_result()?,
        Err(error) => handle_opcode_error(error)?,
    };

    if self.is_initial_call_frame() {
        handle_state_backup(&result)?;
        return Ok(result);
    }

    handle_return(&result)?;  // Child → parent callframe interaction
}
```

## Opcode Table

**Location**: `src/opcodes.rs`, function `build_opcode_table()` (approx line 385)

Uses **fork-gated incremental layering** with `const fn` chaining:

```
Pre-Shanghai (base)    → All opcodes up to London/Paris
Shanghai additions     → PUSH0
Cancun additions       → TSTORE, TLOAD, MCOPY, BLOBHASH, BLOBBASEFEE
Osaka additions        → CLZ
Amsterdam additions    → DUPN, SWAPN, EXCHANGE (EIP-8024)
```

**Dispatch**: `build_opcode_table(fork)` uses an if-chain to select the right table:
```rust
if fork >= Fork::Amsterdam { Self::build_opcode_table_amsterdam() }
else if fork >= Fork::Osaka { Self::build_opcode_table_osaka() }
else if fork >= Fork::Cancun { Self::build_opcode_table_pre_osaka() }
// ...
```

**Chaining**: Each builder is a `const fn` that calls the previous fork's builder as its base and adds new entries:
```rust
const fn build_opcode_table_pre_cancun() -> [OpCodeFn<'a>; 256] {
    let mut opcode_table = Self::build_opcode_table_pre_shanghai();
    opcode_table[Opcode::PUSH0 as usize] = OpCodeFn(VM::op_push0);
    opcode_table
}
```

This pattern compiles each fork's table at compile time. Invalid/undefined opcodes map to `on_invalid_opcode` handler that returns an error.

## Hook System

**Location**: `src/hooks/`

### Hook Trait (`src/hooks/hook.rs:9-17`)

```rust
pub trait Hook {
    fn prepare_execution(&mut self, vm: &mut VM<'_>) -> Result<(), VMError>;
    fn finalize_execution(&mut self, vm: &mut VM<'_>, report: &mut ContextResult) -> Result<(), VMError>;
}
```

### Hook Dispatch (`src/hooks/hook.rs:19-24`)

```rust
pub fn get_hooks(vm_type: &VMType) -> Vec<Rc<RefCell<dyn Hook + 'static>>> {
    match vm_type {
        VMType::L1 => l1_hooks(),           // [DefaultHook]
        VMType::L2(fee_config) => l2_hooks(*fee_config),  // [L2Hook, BackupHook]
    }
}
```

### Implementations

| Hook | Purpose | Used By |
|------|---------|---------|
| `DefaultHook` | Tx validation, gas deduction, nonce increment, gas refund | L1 |
| `L2Hook` | L2 fee handling (additional fee config logic) | L2 |
| `BackupHook` | Cache state backup/restore for stateless execution | L2, `stateless_execute()` |

### Extension Point for Tokamak

Adding a new hook requires:
1. Implement `Hook` trait
2. Add to `get_hooks()` match (or add new `VMType` variant)
3. No changes to the main loop needed

## State Management

### GeneralizedDatabase (`src/db/gen_db.rs:28-37`)

```rust
pub struct GeneralizedDatabase {
    pub store: Arc<dyn Database>,              // Backing persistent store
    pub current_accounts_state: CacheDB,       // Current modified state (FxHashMap)
    pub initial_accounts_state: CacheDB,       // State at start of block
    pub codes: FxHashMap<H256, Code>,           // Contract bytecode cache
    pub code_metadata: FxHashMap<H256, CodeMetadata>,  // Code metadata cache
    pub tx_backup: Option<CallFrameBackup>,    // Transaction-level backup
    pub bal_recorder: Option<BlockAccessListRecorder>,  // EIP-7928 BAL
}
```

`CacheDB` is `FxHashMap<Address, LevmAccount>` — a fast hash map using Rust's `rustc-hash`.

### Substate (`src/vm.rs:66-83`)

Tracks all revertible state changes using a **linked-list checkpointing** pattern:

```rust
pub struct Substate {
    parent: Option<Box<Self>>,                     // Checkpoint chain
    selfdestruct_set: FxHashSet<Address>,           // SELFDESTRUCT targets
    accessed_addresses: FxHashSet<Address>,         // EIP-2929 warm addresses
    accessed_storage_slots: FxHashMap<Address, FxHashSet<H256>>,  // EIP-2929 warm slots
    created_accounts: FxHashSet<Address>,           // Newly created accounts
    pub refunded_gas: u64,                          // Gas refund accumulator
    transient_storage: TransientStorage,            // EIP-1153
    logs: Vec<Log>,                                 // Event logs
}
```

Operations:
- `push_backup()` — Create checkpoint (moves current state to parent)
- `commit_backup()` — Merge child into parent (success path)
- `revert_backup()` — Discard child, restore parent (failure path)

## Core Types

### CallFrame (`src/call_frame.rs`)

```rust
pub struct CallFrame {
    pub gas_limit: u64,
    pub gas_remaining: i64,          // Signed (i64) for perf; safe per EIP-7825
    pub pc: usize,                   // Program counter
    pub msg_sender: Address,         // Sender of the message (NOT "caller")
    pub to: Address,                 // Recipient address
    pub code_address: Address,       // Address of executing code
    pub bytecode: Code,              // Bytecode to execute (Code type, NOT Bytes)
    pub msg_value: U256,             // Value sent with the message
    pub stack: Stack,                // Fixed 1024-element stack
    pub memory: Memory,              // Dynamically expanding byte array
    pub calldata: Bytes,
    pub output: Bytes,               // Return data of CURRENT context
    pub sub_return_data: Bytes,      // Return data of SUB-context (child call)
    pub is_static: bool,             // Static call flag (no state changes)
    pub depth: usize,                // Call depth (max 1024)
    pub is_create: bool,             // CREATE/CREATE2 context flag
    pub call_frame_backup: CallFrameBackup,  // Pre-write state for revert
    pub ret_offset: usize,           // Return data offset
    pub ret_size: usize,             // Return data size
}
```

- **Stack**: Fixed `[U256; 1024]` array (STACK_LIMIT constant)
- **Memory**: Dynamically expanding, 32-byte word aligned
- **PC**: Simple `usize` index into bytecode
- **Code vs Bytes**: `bytecode` is `Code` type (includes hash metadata), not raw `Bytes`
- **Output split**: `output` = current frame's return, `sub_return_data` = child call's return (RETURNDATACOPY source)

### Environment (`src/environment.rs:17-44`)

```rust
pub struct Environment {
    pub origin: Address,             // tx.from (external sender)
    pub gas_limit: u64,              // Transaction gas limit
    pub config: EVMConfig,           // Fork + blob schedule
    pub block_number: U256,
    pub coinbase: Address,           // Block beneficiary
    pub timestamp: U256,
    pub prev_randao: Option<H256>,
    // (difficulty, slot_number omitted)
    pub chain_id: U256,
    pub base_fee_per_gas: U256,
    pub gas_price: U256,             // Effective gas price
    // ... difficulty, slot_number, blob fields, tx params, fee token
}
```

### EVMConfig (`src/environment.rs:55-58`)

```rust
pub struct EVMConfig {
    pub fork: Fork,                  // Current hard fork
    pub blob_schedule: ForkBlobSchedule,  // EIP-7840 blob parameters
}
```

## Tracing

**Location**: `src/tracing.rs`

`LevmCallTracer` records call-level traces during execution:
- Call entry/exit events
- Gas usage per call
- Return data and revert reasons
- Used by `debug_traceTransaction` RPC method

## Benchmarking

**Location**: `src/timings.rs`

When `perf_opcode_timings` feature is enabled:
- `OPCODE_TIMINGS`: Global `LazyLock<Mutex<OpcodeTimings>>`
- Each opcode execution records `Instant::now()` → `elapsed()`
- `OpcodeTimings` stores 4 fields:
  - `totals: HashMap<Opcode, Duration>` — accumulated wall time per opcode
  - `counts: HashMap<Opcode, u64>` — invocation count per opcode
  - `blocks: usize` — number of blocks processed
  - `txs: usize` — number of transactions processed
- `info()` computes average duration at display time (total / count), no min/max tracked
- Used in the main loop via `#[cfg(feature = "perf_opcode_timings")]` blocks

## Lint Configuration

**Location**: `Cargo.toml` `[lints]` section

### Strict Denials

| Lint | Level | Purpose |
|------|-------|---------|
| `clippy::arithmetic_side_effects` | **deny** | Prevent unchecked overflow/underflow |
| `clippy::unwrap_used` | **deny** | No `.unwrap()` calls |
| `clippy::expect_used` | **deny** | No `.expect()` calls |
| `clippy::as_conversions` | **deny** | No `as` casts (use `try_into()` etc.) |
| `clippy::panic` | **deny** | No `panic!()` macro |

### Warnings

| Lint | Level |
|------|-------|
| `unsafe_code` | warn |
| `clippy::indexing_slicing` | warn |
| `clippy::redundant_clone` | warn |
| `clippy::panicking_overflow_checks` | warn |
| `clippy::manual_saturating_arithmetic` | warn |

These strict lints ensure safety-critical EVM execution code avoids common Rust pitfalls.
