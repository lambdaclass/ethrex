# EIP Implementation Guide for ethrex2

## Overview

This document provides a comprehensive guide for implementing four repricing EIPs in the ethrex2 Ethereum client. These EIPs are part of the upcoming Glamsterdam hardfork focused on improving network efficiency and scalability.

## EIPs Covered

- **EIP-7976**: Increase Calldata Floor Cost
- **EIP-7981**: Increase Access List Cost
- **EIP-8037**: State Creation Gas Cost Increase
- **EIP-8038**: State-access Gas Cost Update

## Implementation Complexity Ranking

### 1. EIP-7976 (Calldata Floor Cost) - FASTEST ⚡

**Complexity:** 1/5  
**Estimated Changes:** 2 constants

#### What It Does
Increases calldata floor cost from 10/40 to 15/60 gas per zero/non-zero byte. Reduces worst-case block size by ~33% with minimal impact on regular transactions. Only affects ~1.5% of transactions.

#### Implementation Details

**Files to Modify:**
- `crates/vm/levm/src/gas_cost.rs`

**Constants to Change (lines 163-166):**
```rust
// BEFORE
pub const CALLDATA_COST_ZERO_BYTE: u64 = 4;
pub const CALLDATA_COST_NON_ZERO_BYTE: u64 = 16;

// AFTER (EIP-7976)
pub const CALLDATA_COST_ZERO_BYTE: u64 = 15;
pub const CALLDATA_COST_NON_ZERO_BYTE: u64 = 16;  // OR 60, depending on interpretation
```

**Note:** The EIP mentions "15/60" as the floor cost. The current implementation uses these constants in the `tx_calldata()` function. You may need to verify if the floor cost should apply conditionally (similar to EIP-7623 floor gas logic).

**Where It's Used:**

1. **Main calculation:** `crates/vm/levm/src/gas_cost.rs:560-576`
```rust
pub fn tx_calldata(calldata: &Bytes) -> Result<u64, VMError> {
    let mut calldata_cost: u64 = 0;
    for byte in calldata {
        calldata_cost = if *byte != 0 {
            calldata_cost.checked_add(CALLDATA_COST_NON_ZERO_BYTE).ok_or(OutOfGas)?
        } else {
            calldata_cost.checked_add(CALLDATA_COST_ZERO_BYTE).ok_or(OutOfGas)?
        }
    }
    Ok(calldata_cost)
}
```

2. **Intrinsic gas calculation:** `crates/vm/levm/src/utils.rs:453-524`
   - Called within `VM::get_intrinsic_gas()`

3. **Transaction validation:** `crates/vm/levm/src/hooks/default_hook.rs:273-300`
   - Line 283: `let calldata_cost: u64 = gas_cost::tx_calldata(&calldata)?;`
   - Includes EIP-7623 floor gas cost calculation

**Fork Activation:**
You'll need to add a fork identifier for Glamsterdam and conditionally apply these constants:
```rust
pub fn tx_calldata(calldata: &Bytes, fork: Fork) -> Result<u64, VMError> {
    let (zero_byte_cost, non_zero_byte_cost) = if fork >= Fork::Glamsterdam {
        (15, 60)  // EIP-7976
    } else {
        (4, 16)   // Current
    };
    
    let mut calldata_cost: u64 = 0;
    for byte in calldata {
        calldata_cost = if *byte != 0 {
            calldata_cost.checked_add(non_zero_byte_cost).ok_or(OutOfGas)?
        } else {
            calldata_cost.checked_add(zero_byte_cost).ok_or(OutOfGas)?
        }
    }
    Ok(calldata_cost)
}
```

**Testing Considerations:**
- Test with data-heavy transactions
- Verify impact on contract deployments (large bytecode)
- Check compatibility with EIP-7623 floor gas logic

---

### 2. EIP-7981 (Access List Cost) - ALSO VERY FAST ⚡

**Complexity:** 1/5  
**Estimated Changes:** 2 constants + logic for data footprint

#### What It Does
Charges access lists for their data footprint, adding 40 gas per non-zero byte and 10 gas per zero byte. Closes loophole that circumvents EIP-7623 floor pricing. Reduces worst-case block size by ~21%.

#### Implementation Details

**Files to Modify:**
- `crates/vm/levm/src/gas_cost.rs`
- `crates/vm/levm/src/utils.rs`

**Current Constants (lines 171-173):**
```rust
pub const ACCESS_LIST_STORAGE_KEY_COST: u64 = 1900;
pub const ACCESS_LIST_ADDRESS_COST: u64 = 2400;
```

**New Implementation:**

The EIP requires charging BOTH the existing EIP-2930 costs AND the data footprint. The data footprint calculation needs to be added:

```rust
// Add new constants for data footprint
pub const ACCESS_LIST_DATA_COST_ZERO_BYTE: u64 = 10;
pub const ACCESS_LIST_DATA_COST_NON_ZERO_BYTE: u64 = 40;

// Keep existing constants
pub const ACCESS_LIST_STORAGE_KEY_COST: u64 = 1900;
pub const ACCESS_LIST_ADDRESS_COST: u64 = 2400;
```

**Where It's Used:**

`crates/vm/levm/src/utils.rs:488-503` in `VM::get_intrinsic_gas()`:

```rust
// CURRENT IMPLEMENTATION
let mut access_lists_cost: u64 = 0;
for (_, keys) in self.tx.access_list() {
    access_lists_cost = access_lists_cost
        .checked_add(ACCESS_LIST_ADDRESS_COST)  // 2400 per address
        .ok_or(OutOfGas)?;
    for _ in keys {
        access_lists_cost = access_lists_cost
            .checked_add(ACCESS_LIST_STORAGE_KEY_COST)  // 1900 per storage key
            .ok_or(OutOfGas)?;
    }
}
```

**NEW IMPLEMENTATION (EIP-7981):**

```rust
pub fn access_list_cost(
    access_list: &[(Address, Vec<H256>)],
    fork: Fork,
) -> Result<u64, VMError> {
    let mut cost: u64 = 0;
    
    for (address, keys) in access_list {
        // EIP-2930 functionality cost
        cost = cost.checked_add(ACCESS_LIST_ADDRESS_COST).ok_or(OutOfGas)?;
        
        for _ in keys {
            cost = cost.checked_add(ACCESS_LIST_STORAGE_KEY_COST).ok_or(OutOfGas)?;
        }
        
        // EIP-7981: Add data footprint cost
        if fork >= Fork::Glamsterdam {
            // Address bytes (20 bytes)
            for byte in address.as_bytes() {
                let byte_cost = if *byte != 0 {
                    ACCESS_LIST_DATA_COST_NON_ZERO_BYTE
                } else {
                    ACCESS_LIST_DATA_COST_ZERO_BYTE
                };
                cost = cost.checked_add(byte_cost).ok_or(OutOfGas)?;
            }
            
            // Storage key bytes (32 bytes each)
            for key in keys {
                for byte in key.as_bytes() {
                    let byte_cost = if *byte != 0 {
                        ACCESS_LIST_DATA_COST_NON_ZERO_BYTE
                    } else {
                        ACCESS_LIST_DATA_COST_ZERO_BYTE
                    };
                    cost = cost.checked_add(byte_cost).ok_or(OutOfGas)?;
                }
            }
        }
    }
    
    Ok(cost)
}
```

**Testing Considerations:**
- Test transactions with access lists
- Verify backwards compatibility (pre-Glamsterdam)
- Check that EIP-2930 functionality costs are preserved
- Test edge cases: empty access lists, large access lists

---

### 3. EIP-8038 (State-access Gas Cost Update) - MODERATE 🔸

**Complexity:** 3/5  
**Estimated Changes:** 4-6 constants

#### What It Does
Updates gas costs for state-access operations to reflect Ethereum's larger state since EIP-2929. Raises base costs for storage operations and cold/warm account access. Coordinates with EIP-8032 for contract size assumptions.

#### Implementation Details

**Files to Modify:**
- `crates/vm/levm/src/gas_cost.rs`

**Constants to Update (lines 102-156):**

```rust
// STORAGE ACCESS - BEFORE
pub const SLOAD_COLD_DYNAMIC: u64 = 2100;
pub const SLOAD_WARM_DYNAMIC: u64 = 100;

// STORAGE ACCESS - AFTER (EIP-8038)
// Check EIP spec for exact new values
pub const SLOAD_COLD_DYNAMIC: u64 = 2600;  // Example value, verify from spec
pub const SLOAD_WARM_DYNAMIC: u64 = 150;   // Example value, verify from spec

// STORAGE MODIFICATION - BEFORE
pub const SSTORE_COLD_DYNAMIC: u64 = 2100;
pub const SSTORE_DEFAULT_DYNAMIC: u64 = 100;
pub const SSTORE_STORAGE_CREATION: u64 = 20000;
pub const SSTORE_STORAGE_MODIFICATION: u64 = 2900;

// STORAGE MODIFICATION - AFTER (EIP-8038)
pub const SSTORE_COLD_DYNAMIC: u64 = 2600;      // Example
pub const SSTORE_DEFAULT_DYNAMIC: u64 = 150;    // Example
pub const SSTORE_STORAGE_CREATION: u64 = 22000; // Example
pub const SSTORE_STORAGE_MODIFICATION: u64 = 3200; // Example

// ACCOUNT ACCESS - BEFORE
pub const WARM_ADDRESS_ACCESS_COST: u64 = 100;
pub const COLD_ADDRESS_ACCESS_COST: u64 = 2600;

// ACCOUNT ACCESS - AFTER (EIP-8038)
pub const WARM_ADDRESS_ACCESS_COST: u64 = 150;  // Example
pub const COLD_ADDRESS_ACCESS_COST: u64 = 3100; // Example

// EXTCODESIZE/EXTCODECOPY - Update if needed
pub const EXTCODESIZE_STATIC_COST: u64 = 0;
pub const EXTCODESIZE_DYNAMIC_COST: u64 = 100;
pub const EXTCODESIZE_COLD_DYNAMIC_COST: u64 = 2600;
```

**Note:** You MUST verify the exact new gas values from the official EIP-8038 specification.

**Where These Are Used:**

1. **SLOAD:** `crates/vm/levm/src/opcode_handlers/stack_memory_storage_flow.rs:129-148`
   - Gas cost function: `crates/vm/levm/src/gas_cost.rs:396-404`

2. **SSTORE:** `crates/vm/levm/src/opcode_handlers/stack_memory_storage_flow.rs:150-247`
   - Gas cost function: `crates/vm/levm/src/gas_cost.rs:406-462`

3. **Account access (BALANCE, EXTCODESIZE, EXTCODECOPY, EXTCODEHASH):**
   - `crates/vm/levm/src/opcode_handlers/environmental.rs`
   - Uses `WARM_ADDRESS_ACCESS_COST` and `COLD_ADDRESS_ACCESS_COST`

4. **CALL variants (CALL, CALLCODE, DELEGATECALL, STATICCALL):**
   - `crates/vm/levm/src/opcode_handlers/system.rs`
   - Account access costs integrated into call gas calculation

5. **SELFDESTRUCT:**
   - `crates/vm/levm/src/opcode_handlers/system.rs`

**Cold/Warm Tracking System:**

The system uses `Substate` to track accessed addresses and storage slots:

`crates/vm/levm/src/vm.rs:66-74`:
```rust
pub struct Substate {
    accessed_addresses: FxHashSet<Address>,
    accessed_storage_slots: BTreeMap<Address, BTreeSet<H256>>,
    // ...
}
```

This tracking is already implemented and doesn't need changes. The EIP only updates the constants.

**Fork-Aware Implementation:**

Each gas cost function should check the fork:

```rust
pub fn sload(storage_slot_was_cold: bool, fork: Fork) -> Result<u64, VMError> {
    let (cold_cost, warm_cost) = if fork >= Fork::Glamsterdam {
        (2600, 150)  // EIP-8038 values (example)
    } else {
        (2100, 100)  // Current values
    };
    
    let dynamic_cost = if storage_slot_was_cold {
        cold_cost
    } else {
        warm_cost
    };
    
    SLOAD_STATIC.checked_add(dynamic_cost).ok_or(OutOfGas.into())
}
```

**Testing Considerations:**
- Test all opcodes that access state: SLOAD, SSTORE, BALANCE, EXTCODESIZE, EXTCODECOPY, EXTCODEHASH
- Test CALL variants with cold and warm access
- Test SELFDESTRUCT
- Verify cold/warm tracking still works correctly
- Test fork transition behavior

---

### 4. EIP-8037 (State Creation Gas Cost Increase) - SLOWEST 🔻

**Complexity:** 5/5  
**Estimated Changes:** Algorithmic modification + state tracking

#### What It Does
Introduces dynamic `cost_per_state_byte` that adjusts based on block gas limit to mitigate state growth. Uses multidimensional gas metering so state creation costs don't affect other operations.

#### Implementation Details

This is the most complex EIP because it requires:
1. Dynamic gas calculation based on block gas limit
2. Separate gas dimension for state creation
3. New parameter threading through execution
4. State tracking mechanism

**Files to Modify:**
- `crates/vm/levm/src/gas_cost.rs`
- `crates/vm/levm/src/opcode_handlers/system.rs`
- `crates/vm/levm/src/execution_handlers.rs`
- `crates/vm/levm/src/vm.rs` (add state creation tracking)
- `crates/vm/levm/src/call_frame.rs` (add separate gas dimension)

**Current Implementation:**

1. **CREATE gas cost:** `crates/vm/levm/src/gas_cost.rs:464-535`
```rust
fn compute_gas_create(
    new_memory_size: usize,
    current_memory_size: usize,
    code_size_in_memory: usize,
    is_create_2: bool,
    fork: Fork,
) -> Result<u64, VMError> {
    // Uses static CREATE_BASE_COST: 32000
    let gas_create_cost = memory_expansion_cost
        .checked_add(init_code_cost)?
        .checked_add(CREATE_BASE_COST)?  // Static 32000
        .checked_add(hash_cost)?;
    
    Ok(gas_create_cost)
}
```

2. **Code deposit cost:** `crates/vm/levm/src/execution_handlers.rs:130-158`
```rust
fn validate_contract_creation(&mut self) -> Result<(), VMError> {
    let code_length: u64 = code.len().try_into()?;
    let code_deposit_cost: u64 = code_length.checked_mul(CODE_DEPOSIT_COST)?;  // Static 200
    callframe.increase_consumed_gas(code_deposit_cost)?;
    Ok(())
}
```

**New Implementation (EIP-8037):**

```rust
// Add new configuration
pub struct StateCreationConfig {
    pub target_state_growth_per_year: u64,
    pub cost_per_state_byte: u64,
}

impl StateCreationConfig {
    pub fn calculate_cost_per_byte(block_gas_limit: u64) -> u64 {
        // Formula from EIP-8037
        // cost_per_state_byte = f(block_gas_limit, target_state_growth)
        // Implement the dynamic calculation based on the EIP spec
        todo!()
    }
}

// Modify CREATE gas calculation
fn compute_gas_create(
    new_memory_size: usize,
    current_memory_size: usize,
    code_size_in_memory: usize,
    is_create_2: bool,
    fork: Fork,
    state_creation_config: &StateCreationConfig,  // New parameter
) -> Result<u64, VMError> {
    let base_cost = if fork >= Fork::Glamsterdam {
        // Dynamic cost based on state size
        let estimated_state_size = code_size_in_memory;  // Simplified
        estimated_state_size.checked_mul(state_creation_config.cost_per_state_byte)?
    } else {
        CREATE_BASE_COST  // Static 32000
    };
    
    let gas_create_cost = memory_expansion_cost
        .checked_add(init_code_cost)?
        .checked_add(base_cost)?
        .checked_add(hash_cost)?;
    
    Ok(gas_create_cost)
}

// Add multidimensional gas tracking
pub struct CallFrame {
    pub gas_limit: u64,
    pub gas_used: u64,
    pub state_creation_gas_used: u64,  // New: separate dimension
    // ...
}
```

**Multidimensional Gas Metering:**

The EIP specifies that state creation should be metered separately. This means:
1. Regular operations consume from `gas_used`
2. State creation operations consume from `state_creation_gas_used`
3. Both have separate limits derived from the transaction gas limit

**Configuration Threading:**

You'll need to pass the state creation config through:
1. VM initialization
2. Opcode handlers (CREATE, CREATE2)
3. Contract validation

**Testing Considerations:**
- Test CREATE and CREATE2 with various contract sizes
- Test gas limit variations
- Verify separate gas dimension tracking
- Test interaction with other gas calculations
- Verify backwards compatibility

---

## Architecture Overview

### Gas Cost System Structure

```
crates/vm/levm/src/
├── gas_cost.rs                 # Central gas constants and calculation functions
├── constants.rs                # General VM constants
├── utils.rs                    # VM utility functions (intrinsic gas)
├── hooks/
│   └── default_hook.rs         # Transaction validation with calldata costs
├── opcode_handlers/
│   ├── stack_memory_storage_flow.rs  # SLOAD, SSTORE
│   ├── environmental.rs        # BALANCE, EXTCODESIZE, EXTCODECOPY
│   └── system.rs               # CREATE, CREATE2, CALL variants, SELFDESTRUCT
└── execution_handlers.rs       # Contract creation validation
```

### Key Data Structures

**Substate (Cold/Warm Tracking):**
```rust
pub struct Substate {
    accessed_addresses: FxHashSet<Address>,
    accessed_storage_slots: BTreeMap<Address, BTreeSet<H256>>,
    // ...
}
```

**VM Environment:**
```rust
pub struct Environment {
    pub config: Config,      // Contains fork information
    pub block: Block,        // Block context
    // ...
}
```

### Fork Activation Pattern

Most gas cost functions should follow this pattern:

```rust
pub fn some_gas_cost(params: ..., fork: Fork) -> Result<u64, VMError> {
    let cost = if fork >= Fork::Glamsterdam {
        // New EIP values
        new_calculation(params)
    } else {
        // Pre-Glamsterdam values
        old_calculation(params)
    };
    Ok(cost)
}
```

---

## Implementation Roadmap

### Phase 1: Quick Wins (EIP-7976, EIP-7981)
**Estimated Time:** 1-2 days including testing

1. Add `Fork::Glamsterdam` variant
2. Implement EIP-7976 (calldata floor cost)
   - Update constants with fork check
   - Add tests
3. Implement EIP-7981 (access list cost)
   - Add data footprint calculation
   - Update intrinsic gas function
   - Add tests

### Phase 2: Moderate Complexity (EIP-8038)
**Estimated Time:** 2-3 days including testing

1. Research exact new gas values from EIP-8038 spec
2. Update all state-access constants with fork checks
3. Test all affected opcodes
4. Verify cold/warm tracking

### Phase 3: Complex (EIP-8037)
**Estimated Time:** 5-7 days including testing

1. Design multidimensional gas architecture
2. Implement dynamic `cost_per_state_byte` calculation
3. Add separate state creation gas tracking
4. Thread configuration through VM
5. Update CREATE/CREATE2 opcodes
6. Update contract validation
7. Comprehensive testing

---

## Testing Strategy

### Unit Tests

For each EIP, add tests in the relevant test modules:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_eip7976_calldata_cost() {
        // Pre-Glamsterdam
        let cost_old = tx_calldata(&calldata, Fork::Cancun).unwrap();
        
        // Post-Glamsterdam
        let cost_new = tx_calldata(&calldata, Fork::Glamsterdam).unwrap();
        
        assert!(cost_new > cost_old);
    }
    
    #[test]
    fn test_eip7981_access_list_cost() {
        // Test with various access list sizes
    }
    
    #[test]
    fn test_eip8038_sload_cost() {
        // Test cold and warm SLOAD costs
    }
    
    #[test]
    fn test_eip8037_create_dynamic_cost() {
        // Test CREATE with different gas limits
    }
}
```

### Integration Tests

Create comprehensive integration tests that:
1. Execute transactions with the new gas costs
2. Verify correct gas consumption
3. Test fork transition behavior
4. Compare results with reference implementations

### Hive Tests

Check if Hive test suite includes tests for these EIPs:
- Look for Glamsterdam-related tests
- Add custom tests if needed

---

## Reference Links

### Official EIP Specifications
- [EIP-7976: Increase Calldata Floor Cost](https://eips.ethereum.org/EIPS/eip-7976)
- [EIP-7981: Increase Access List Cost](https://eips.ethereum.org/EIPS/eip-7981)
- [EIP-8037: State Creation Gas Cost Increase](https://eips.ethereum.org/EIPS/eip-8037)
- [EIP-8038: State-access Gas Cost Update](https://eips.ethereum.org/EIPS/eip-8038)

### Related EIPs
- [EIP-7623: Calldata Floor Pricing](https://eips.ethereum.org/EIPS/eip-7623) - Already implemented
- [EIP-2929: Gas Cost Increases for State Access](https://eips.ethereum.org/EIPS/eip-2929) - Berlin fork
- [EIP-2930: Optional Access Lists](https://eips.ethereum.org/EIPS/eip-2930) - Berlin fork
- [EIP-7773: Hardfork Meta - Glamsterdam](https://eips.ethereum.org/EIPS/eip-7773) - Glamsterdam overview

### Community Discussions
- [EIP-7976 Discussion](https://ethereum-magicians.org/t/eip-7976-further-increase-calldata-cost/24597)
- [EIP-7981 Discussion](https://ethereum-magicians.org/t/eip-7981-increase-access-list-cost/24680)
- [EIP-8037 Discussion](https://ethereum-magicians.org/t/eip-8037-state-creation-gas-cost-increase/25694)
- [Glamsterdam Overview](https://etherworld.co/glamsterdam-prep-begins-10-repricing-eips-take-spotlight/)

---

## Notes and Considerations

### General Implementation Notes

1. **Fork Activation**: All EIPs require a fork identifier. You'll need to add `Fork::Glamsterdam` to the fork enum.

2. **Backwards Compatibility**: All changes must be backwards compatible with pre-Glamsterdam blocks. Use fork checks consistently.

3. **Gas Overflow Protection**: The codebase uses `checked_add` and `checked_mul` extensively. Maintain this pattern in all new code.

4. **Error Handling**: Use `OutOfGas` error when gas calculations overflow or exceed limits.

5. **Testing**: The project has Hive test integration. Check `.github/workflows/daily_hive_report.yaml` for the testing setup.

### Performance Considerations

- EIP-7976 and EIP-7981 have minimal performance impact (constant changes only)
- EIP-8038 may have slight performance impact due to higher costs
- EIP-8037 has moderate performance impact due to dynamic calculations

### State Growth Mitigation

These EIPs are specifically designed to address state growth concerns:
- EIP-7976: Reduces data-heavy transaction throughput
- EIP-7981: Makes access lists less attractive for gas optimization
- EIP-8037: Directly limits state creation rate
- EIP-8038: Reflects true cost of state access

### Coordination Between EIPs

- EIP-7981 builds on EIP-7623 (already implemented)
- EIP-8038 coordinates with EIP-8032 (check if implemented)
- All EIPs should be activated together in Glamsterdam fork

---

## Quick Start: Implementing EIP-7976

Since EIP-7976 is the fastest to implement, here's a step-by-step guide:

1. **Add Fork Variant** (if not already present):
```rust
// In crates/vm/levm/src/fork.rs or wherever forks are defined
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Fork {
    // ...
    Prague,
    Glamsterdam,  // Add this
}
```

2. **Update Calldata Function**:
```rust
// In crates/vm/levm/src/gas_cost.rs
pub fn tx_calldata(calldata: &Bytes, fork: Fork) -> Result<u64, VMError> {
    let (zero_byte_cost, non_zero_byte_cost) = if fork >= Fork::Glamsterdam {
        (15, 60)  // EIP-7976
    } else {
        (4, 16)   // Current
    };
    
    let mut calldata_cost: u64 = 0;
    for byte in calldata {
        calldata_cost = if *byte != 0 {
            calldata_cost.checked_add(non_zero_byte_cost).ok_or(OutOfGas)?
        } else {
            calldata_cost.checked_add(zero_byte_cost).ok_or(OutOfGas)?
        }
    }
    Ok(calldata_cost)
}
```

3. **Update Call Sites**:
```rust
// In crates/vm/levm/src/utils.rs (get_intrinsic_gas)
let calldata_cost = gas_cost::tx_calldata(&self.tx.data(), self.env.config.fork)?;

// In crates/vm/levm/src/hooks/default_hook.rs
let calldata_cost = gas_cost::tx_calldata(&calldata, env.config.fork)?;
```

4. **Add Tests**:
```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_eip7976_calldata_zero_bytes() {
        let data = Bytes::from(vec![0u8; 100]);
        
        let cost_pre = tx_calldata(&data, Fork::Prague).unwrap();
        assert_eq!(cost_pre, 400);  // 100 * 4
        
        let cost_post = tx_calldata(&data, Fork::Glamsterdam).unwrap();
        assert_eq!(cost_post, 1500);  // 100 * 15
    }
    
    #[test]
    fn test_eip7976_calldata_non_zero_bytes() {
        let data = Bytes::from(vec![1u8; 100]);
        
        let cost_pre = tx_calldata(&data, Fork::Prague).unwrap();
        assert_eq!(cost_pre, 1600);  // 100 * 16
        
        let cost_post = tx_calldata(&data, Fork::Glamsterdam).unwrap();
        assert_eq!(cost_post, 6000);  // 100 * 60
    }
    
    #[test]
    fn test_eip7976_calldata_mixed() {
        let data = Bytes::from(vec![0, 1, 0, 1, 0, 1]);
        
        let cost_pre = tx_calldata(&data, Fork::Prague).unwrap();
        assert_eq!(cost_pre, 60);  // 3*4 + 3*16
        
        let cost_post = tx_calldata(&data, Fork::Glamsterdam).unwrap();
        assert_eq!(cost_post, 225);  // 3*15 + 3*60
    }
}
```

5. **Run Tests**:
```bash
cargo test tx_calldata
cargo test --package levm
```

6. **Verify with Hive** (if available):
```bash
# Check if there are Glamsterdam-specific tests
# Update accordingly
```

---

## Conclusion

**Recommended Implementation Order:**
1. **EIP-7976** (1-2 days) - Simplest, good starting point
2. **EIP-7981** (1-2 days) - Similar complexity, builds momentum
3. **EIP-8038** (2-3 days) - Moderate complexity, good learning opportunity
4. **EIP-8037** (5-7 days) - Most complex, tackle last

**Total Estimated Time:** 9-14 days for all four EIPs

This staggered approach allows you to:
- Build confidence with simpler EIPs first
- Establish testing patterns early
- Tackle the complex EIP-8037 with experience from the others

Good luck with the implementation! 🚀
