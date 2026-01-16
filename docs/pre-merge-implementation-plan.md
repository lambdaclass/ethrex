# Pre-Merge Implementation Plan for ethrex

## Executive Summary

This document outlines the implementation plan to add pre-merge (Proof of Work) support to ethrex. Currently, ethrex only supports post-merge (Paris and later) Ethereum. Adding pre-merge support enables:

1. Running historical mainnet sync from genesis
2. Full EF test coverage for all Ethereum forks
3. Potential use as a research/testing tool for PoW chains

**Estimated total lines of code**: 1,800 - 2,500 LOC (depending on Ethash implementation depth)

---

## Current State Analysis

### Implementation Progress (January 2025)

| Component | Status | PR/Notes |
|-----------|--------|----------|
| Fork-aware gas schedule system | ✅ Complete | `gas_schedule.rs` created with per-fork costs |
| Fork-aware SLOAD | ✅ Complete | Uses `sload_with_fork()` |
| Fork-aware BALANCE | ✅ Complete | Uses `balance_with_fork()` |
| Fork-aware EXTCODESIZE | ✅ Complete | Uses `extcodesize_with_fork()` |
| Fork-aware EXTCODECOPY | ✅ Complete | Uses `extcodecopy_with_fork()` |
| Fork-aware EXTCODEHASH | ✅ Complete | Uses `extcodehash_with_fork()` |
| Fork-aware CALL/CALLCODE/DELEGATECALL/STATICCALL | ✅ Complete | Uses `*_with_fork()` functions |
| Fork-aware SELFDESTRUCT | ✅ Complete | Uses `selfdestruct_with_fork()` |
| Fork-aware EXP | ✅ Complete | Uses `exp_with_fork()` |
| Fork-aware tx calldata costs | ✅ Complete | Uses `tx_calldata_with_fork()` |
| Fork-aware header validation | ✅ Complete | Uses `validate_block_header_with_fork()` |
| Pre-merge EF tests enabled | ✅ Complete | Removed fork skip condition |

### Current EF Test Status

- **Total Tests**: 5957
- **Passing**: 2998
- **Failing**: 2959
- **Primary Errors**: `GasUsedMismatch`, `StateRootMismatch`

### What ethrex Already Has

| Component | Status | Notes |
|-----------|--------|-------|
| Fork enum with pre-merge variants | ✅ Complete | `Fork::Frontier` through `Fork::GrayGlacier` defined (`genesis.rs:284-299`) |
| ChainConfig with pre-merge fields | ✅ Complete | `homestead_block`, `byzantium_block`, etc. all supported (`genesis.rs:218-237`) |
| `terminal_total_difficulty` field | ✅ Complete | Used to detect merge transition (`genesis.rs:255`) |
| Environment `difficulty` field | ✅ Complete | VM environment already stores difficulty (`environment.rs:28`) |
| Pre-merge transaction types | ✅ Complete | Legacy (type 0), EIP-2930 (type 1) supported |
| Fork-aware branching patterns | ✅ Complete | `fork >= Fork::Shanghai` patterns throughout codebase |

### What's Missing (Blocks Pre-Merge Support)

| Component | Current Behavior | Required Change |
|-----------|------------------|-----------------|
| Block validation | Requires `difficulty == 0`, `nonce == 0` | Fork-aware validation |
| Genesis validation | Rejects pre-merge genesis | Allow pre-merge genesis |
| Opcode 0x44 | Returns `prev_randao` only | Return `difficulty` pre-Paris |
| Ommer/uncle blocks | Rejected (`ommers.is_empty()` check) | Validate and reward uncles |
| PoW verification | None | Ethash validation (optional) |
| Difficulty calculation | None | Fork-specific algorithms |
| EF test runner | Skips `fork < Fork::Merge` | Enable pre-merge forks |

---

## Implementation Components

### 1. Fork-Aware Block Header Validation

**Files to modify:**
- `crates/common/types/block.rs` (lines 617-672)

**Current code** (`block.rs:655-665`):
```rust
if !header.difficulty.is_zero() {
    return Err(InvalidBlockHeaderError::DifficultyNotZero);
}
if header.nonce != 0 {
    return Err(InvalidBlockHeaderError::NonceNotZero);
}
if header.ommers_hash != *DEFAULT_OMMERS_HASH {
    return Err(InvalidBlockHeaderError::OmmersHashNotDefault);
}
```

**Required change**: Wrap in fork check:
```rust
if fork >= Fork::Paris {
    // Post-merge: PoS rules
    if !header.difficulty.is_zero() { ... }
    if header.nonce != 0 { ... }
    if header.ommers_hash != *DEFAULT_OMMERS_HASH { ... }
} else {
    // Pre-merge: PoW rules
    validate_pow_header(header, parent_header, fork)?;
}
```

**Estimated LOC**: ~80

---

### 2. Difficulty Calculation Algorithm

**New file**: `crates/common/difficulty.rs`

Ethereum's difficulty adjustment varies by fork:

| Fork | Algorithm | EIP |
|------|-----------|-----|
| Frontier | Basic adjustment | - |
| Homestead | Exponential increase | EIP-2 |
| Byzantium | Bomb delay 3M blocks | EIP-649 |
| Constantinople | Bomb delay 5M blocks | EIP-1234 |
| Muir Glacier | Bomb delay 9M blocks | EIP-2384 |
| London | Bomb delay ~9.7M blocks | EIP-3554 |
| Arrow Glacier | Bomb delay ~10.7M blocks | EIP-4345 |
| Gray Glacier | Bomb delay ~11.4M blocks | EIP-5133 |

**Core formula** (post-Homestead):
```
difficulty = parent_difficulty
           + parent_difficulty // 2048 * max(1 - (timestamp - parent_timestamp) // 10, -99)
           + 2^(period_count - 2)  // bomb component
```

Where `period_count = (block_number - bomb_delay) // 100000`

**Estimated LOC**: ~250

---

### 3. Opcode 0x44 (DIFFICULTY/PREVRANDAO)

**File to modify**: `crates/vm/levm/src/opcode_handlers/block.rs` (lines 75-87)

**Current code**:
```rust
pub fn op_prevrandao(&mut self) -> Result<OpcodeResult, VMError> {
    let randao = u256_from_big_endian_const(
        self.env.prev_randao.unwrap_or_default().to_fixed_bytes()
    );
    // ...
}
```

**Required change**:
```rust
pub fn op_prevrandao(&mut self) -> Result<OpcodeResult, VMError> {
    let value = if self.env.config.fork >= Fork::Paris {
        // Post-merge: EIP-4399 - return prev_randao
        u256_from_big_endian_const(
            self.env.prev_randao.unwrap_or_default().to_fixed_bytes()
        )
    } else {
        // Pre-merge: return difficulty
        self.env.difficulty
    };
    // ...
}
```

**Estimated LOC**: ~15

---

### 4. Uncle/Ommer Block Support

**Files to modify:**
- `crates/common/types/block.rs` (lines 688-690)
- `crates/blockchain/blockchain.rs`

**Ommer validation rules** (pre-merge):
- Maximum 2 ommers per block
- Ommer must be sibling or cousin within 6 generations
- Ommer cannot be ancestor of including block
- Ommer parent must be ancestor of including block

**Ommer rewards**:
- Ommer miner: `(8 + ommer_number - block_number) / 8 * block_reward`
- Including block miner: `block_reward / 32` per ommer

**New file**: `crates/blockchain/ommer.rs`

**Estimated LOC**: ~200

---

### 5. Ethash PoW Verification (Optional)

For running EF tests, actual PoW verification can be skipped since tests don't require mining. However, for full historical sync, Ethash is needed.

**Options:**

| Option | LOC | Use Case |
|--------|-----|----------|
| Skip verification | ~30 | EF tests only |
| Light client verification | ~500 | Historical sync with cache |
| Full Ethash | ~1,000 | Complete implementation |

**Recommendation**: Start with "skip verification" mode behind a feature flag, add light verification later if needed for mainnet sync.

**New file**: `crates/blockchain/ethash.rs` (if implementing)

**Estimated LOC**: 30-500 (depending on depth)

---

### 6. Genesis Validation Changes

**File to modify**: `crates/common/types/genesis.rs` (lines 74-86)

**Current code**:
```rust
if genesis.config.terminal_total_difficulty != Some(0)
    && genesis.config.terminal_total_difficulty.is_some()
{
    tracing::warn!(
        "Genesis block specifies a terminal total difficulty != 0. \
         Only post-merge networks are supported. ..."
    );
}
```

**Required change**: Remove or modify this warning to allow pre-merge genesis files.

**Estimated LOC**: ~30

---

### 7. EF Test Runner Changes

**Files to modify:**
- `tooling/ef_tests/blockchain/fork.rs` (lines 136-158)
- `tooling/ef_tests/blockchain/test_runner.rs` (line 37)
- `tooling/ef_tests/state_v2/src/modules/runner.rs`

**Current skip condition** (`test_runner.rs:37`):
```rust
let should_skip_test = test.network < Fork::Merge
```

**Required changes**:

1. **Remove skip condition** or make it configurable
2. **Add chain configs for pre-merge forks** (`fork.rs`):

```rust
pub static ref FRONTIER_CONFIG: ChainConfig = ChainConfig {
    chain_id: 1_u64,
    ..Default::default()
};

pub static ref HOMESTEAD_CONFIG: ChainConfig = ChainConfig {
    homestead_block: Some(0),
    ..Default::default()
};

// ... configs for each pre-merge fork
```

3. **Implement `chain_config()` for pre-merge forks**:
```rust
impl Fork {
    pub fn chain_config(&self) -> &ChainConfig {
        match self {
            Fork::Frontier => &FRONTIER_CONFIG,
            Fork::Homestead => &HOMESTEAD_CONFIG,
            // ... all pre-merge forks
            Fork::Merge => &MERGE_CONFIG,
            // ...
        }
    }
}
```

**Estimated LOC**: ~300

---

### 8. Error Types

**File to modify**: `crates/common/types/block.rs` and `crates/blockchain/error.rs`

**New error variants needed**:
```rust
pub enum InvalidBlockHeaderError {
    // Existing variants...

    // New PoW-specific variants
    DifficultyCalculationMismatch,
    InvalidPoWNonce,
    InvalidMixHash,
    TooManyOmmers,
    InvalidOmmerRelationship,
    DuplicateOmmer,
    OmmerIsAncestor,
}
```

**Estimated LOC**: ~60

---

## EF Tests to Run

### Blockchain Tests by Fork

The EF blockchain tests are organized by fork. With pre-merge support, the following test sets become runnable:

| Fork | Approx. Test Count | Key Features Tested |
|------|-------------------|---------------------|
| **Frontier** | ~100-150 | Basic EVM, contract creation, simple transactions |
| **Homestead** | ~150-200 | DELEGATECALL, difficulty adjustment (EIP-2) |
| **EIP150 (Tangerine)** | ~100-150 | Gas cost increases for IO operations |
| **EIP158 (Spurious Dragon)** | ~100-150 | State clearing, replay protection (EIP-155, 158, 160, 161) |
| **Byzantium** | ~200-300 | REVERT, RETURNDATASIZE/COPY, STATICCALL, precompiles (EIP-196/197/198) |
| **Constantinople** | ~150-200 | Bitwise shifts, CREATE2, EXTCODEHASH (EIP-145, 1014, 1052) |
| **Petersburg** | ~150-200 | Same as Constantinople (net gas metering reverted) |
| **Istanbul** | ~150-200 | CHAINID, Blake2 precompile, gas repricing (EIP-1344, 152, 1884, 2200) |
| **Berlin** | ~150-200 | Access lists, typed transactions (EIP-2718, 2929, 2930) |
| **London** | ~200-300 | EIP-1559 fee market, BASEFEE opcode (EIP-1559, 3198, 3529) |
| **Total** | **~1,500-2,000** | |

### State Tests by Fork

State tests include expected post-states for each fork. The same test runs against multiple forks:

| Test Category | Description | Pre-merge Relevance |
|---------------|-------------|---------------------|
| `stArithmetic/` | Basic arithmetic | All forks |
| `stCallCodes/` | CALL variants | All forks |
| `stDelegatecallTest/` | DELEGATECALL | Homestead+ |
| `stEIP150/` | Gas cost changes | EIP150+ |
| `stEIP158/` | State clearing | EIP158+ |
| `stRevertTest/` | REVERT opcode | Byzantium+ |
| `stReturnDataTest/` | RETURNDATA opcodes | Byzantium+ |
| `stStaticCall/` | STATICCALL | Byzantium+ |
| `stCreate2/` | CREATE2 | Constantinople+ |
| `stExtCodeHash/` | EXTCODEHASH | Constantinople+ |
| `stShift/` | Bitwise shifts | Constantinople+ |
| `stChainId/` | CHAINID | Istanbul+ |
| `stSStoreTest/` | SSTORE gas | Istanbul+ (EIP-2200) |
| `stAccessLists/` | Access lists | Berlin+ |
| `stEIP1559/` | Fee market | London+ |

**Estimated state test cases**: ~20,000-30,000 (same tests × multiple forks)

### Test Execution Commands

After implementation, run tests with:

```bash
# Blockchain tests - all forks
cd tooling/ef_tests/blockchain
cargo test --release

# State tests - specific forks
cd tooling/ef_tests/state_v2
cargo run --release -- --forks Frontier,Homestead,EIP150,EIP158,Byzantium,Constantinople,Istanbul,Berlin,London
```

---

## Implementation Summary

### Lines of Code Estimate

| Component | LOC | Priority |
|-----------|-----|----------|
| Fork-aware block validation | 80 | P0 - Required |
| Difficulty calculation | 250 | P0 - Required |
| Opcode 0x44 change | 15 | P0 - Required |
| Uncle/ommer support | 200 | P0 - Required |
| Genesis validation | 30 | P0 - Required |
| EF test runner changes | 300 | P0 - Required |
| Error types | 60 | P0 - Required |
| **Subtotal (Core)** | **935** | |
| Ethash skip mode | 30 | P1 - For tests |
| Light Ethash verification | 500 | P2 - For sync |
| Full Ethash | 1,000 | P3 - Complete |

**Total estimate**:
- **Minimal (EF tests only)**: ~1,000 LOC
- **With light Ethash**: ~1,500 LOC
- **Full implementation**: ~2,000-2,500 LOC

### Implementation Order

1. **Phase 1**: Core validation changes (P0 items) - enables EF tests
2. **Phase 2**: Light Ethash - enables historical sync verification
3. **Phase 3**: Full Ethash - complete PoW support

### Key Files to Modify

| File | Changes |
|------|---------|
| `crates/common/types/block.rs` | Fork-aware validation, error types |
| `crates/common/types/genesis.rs` | Remove pre-merge restriction |
| `crates/vm/levm/src/opcode_handlers/block.rs` | DIFFICULTY/PREVRANDAO |
| `crates/blockchain/blockchain.rs` | Ommer validation integration |
| `tooling/ef_tests/blockchain/fork.rs` | Pre-merge chain configs |
| `tooling/ef_tests/blockchain/test_runner.rs` | Remove fork skip |

### New Files to Create

| File | Purpose |
|------|---------|
| `crates/common/difficulty.rs` | Difficulty adjustment algorithms |
| `crates/blockchain/ommer.rs` | Uncle block validation and rewards |
| `crates/blockchain/ethash.rs` | PoW verification (optional) |

---

## Risk Assessment

| Risk | Impact | Mitigation |
|------|--------|------------|
| Ethash complexity | High | Skip verification initially, add later |
| Difficulty bomb edge cases | Medium | Extensive testing with EF tests |
| Uncle validation complexity | Medium | Reference existing implementations |
| Gas cost differences across forks | Low | Already fork-aware in LEVM |
| Breaking existing post-merge tests | Low | Feature flag for pre-merge mode |

---

## Code Cleanup / Beautification Plan

After the pre-merge implementation is functionally complete, the following improvements should be made:

### 1. Consolidate Gas Cost Functions

**Current State**: Both old (non-fork-aware) and new (fork-aware) functions exist in `gas_cost.rs`.

**Cleanup Tasks**:
- Remove deprecated non-fork-aware functions once all callers are updated
- Rename `*_with_fork()` functions to simpler names (e.g., `sload_with_fork` → `sload`)
- Group related functions together with clear section headers

### 2. Simplify Gas Schedule Access

**Current State**: Each opcode handler fetches fork from `self.env.config.fork`.

**Cleanup Options**:
- Add `fork` field directly to `VM` struct for easier access
- Create helper method `VM::gas_schedule()` returning `&GasSchedule`
- Consider caching the schedule at VM initialization

### 3. Documentation Improvements

**Add documentation**:
- Add module-level docs to `gas_schedule.rs` explaining the fork progression
- Document gas cost changes with EIP references inline
- Add examples to key functions showing usage

### 4. Test Coverage

**Add tests for**:
- Gas schedule values for each fork
- Gas cost calculations at fork boundaries
- Regression tests for specific EF test failures

### 5. Remove Dead Code

**Review and remove**:
- Unused gas cost constants after migration to `GasSchedule`
- Any commented-out code from experimentation
- Duplicate constants between `gas_cost.rs` and `gas_schedule.rs`

### 6. Naming Consistency

**Standardize naming**:
- Use consistent naming for fork-aware vs non-fork-aware functions
- Align parameter names across similar functions
- Use idiomatic Rust naming conventions

### 7. Error Handling

**Improve error handling**:
- Review error types for gas calculations
- Ensure errors provide useful diagnostic information
- Consider using custom error types for gas-specific failures

---

## References

- [EIP-2: Homestead Hard-fork Changes](https://eips.ethereum.org/EIPS/eip-2)
- [EIP-649: Metropolis Difficulty Bomb Delay](https://eips.ethereum.org/EIPS/eip-649)
- [EIP-3675: Upgrade consensus to Proof-of-Stake](https://eips.ethereum.org/EIPS/eip-3675)
- [EIP-4399: PREVRANDAO replaces DIFFICULTY](https://eips.ethereum.org/EIPS/eip-4399)
- [Ethereum Yellow Paper - Block Validation](https://ethereum.github.io/yellowpaper/paper.pdf)
- [EF Execution Spec Tests](https://ethereum.github.io/execution-spec-tests/)
