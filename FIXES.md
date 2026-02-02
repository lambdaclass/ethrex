# EF Test Fixes

## Fix 1: Skip coinbase payment when fee is zero

**File:** `crates/vm/levm/src/hooks/default_hook.rs`

**Problem:** The `pay_coinbase` function was calling `increase_account_balance(coinbase, 0)` even when the coinbase fee was zero (e.g., during system contract calls with zero gas price). This caused the coinbase address to be added to the BAL's `touched_addresses` even for empty blocks where no actual balance change occurred.

**Root Cause:** System contract calls (beacon root, history storage) execute with `gas_price = 0` and `base_fee_per_gas = 0`. When `pay_coinbase` was called via `finalize_execution`, it computed `coinbase_fee = 0` but still called `increase_account_balance(coinbase, 0)`, which triggered `record_balance_change` in the BAL recorder, adding the coinbase to `touched_addresses`.

**Fix:** Added a check to skip the `increase_account_balance` call when `coinbase_fee` is zero:

```rust
pub fn pay_coinbase(vm: &mut VM<'_>, gas_to_pay: u64) -> Result<(), VMError> {
    let priority_fee_per_gas = vm
        .env
        .gas_price
        .checked_sub(vm.env.base_fee_per_gas)
        .ok_or(InternalError::Underflow)?;

    let coinbase_fee = U256::from(gas_to_pay)
        .checked_mul(priority_fee_per_gas)
        .ok_or(InternalError::Overflow)?;

    // Only pay coinbase if there's actually a fee to pay.
    // This avoids marking coinbase as touched when there's no actual balance change
    // (e.g., during system contract calls with zero gas price).
    if !coinbase_fee.is_zero() {
        vm.increase_account_balance(vm.env.coinbase, coinbase_fee)?;
    }

    Ok(())
}
```

**Tests Fixed:** 3 tests
- `test_bal_4788_empty_block[fork_Amsterdam-blockchain_test]`
- And 2 other Amsterdam/BAL tests

**Progress:**
- Before: 67 failing Amsterdam tests
- After: 64 failing Amsterdam tests
