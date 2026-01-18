# Investigation: build_block_benchmark Failure

## Objective
Run baseline benchmarks for LEVM performance tracking.

## Command Executed
```bash
cargo bench --package ethrex-benches --bench build_block_benchmark
```

## Result: FAILED

### Error
```
thread 'main' panicked at benches/benches/build_block_benchmark.rs:178:45:
called `Result::unwrap()` on an `Err` value: NotEnoughBalance
```

### Root Cause Analysis

The failure occurs in `fill_mempool()` at line 178:
```rust
b.add_transaction_to_pool(tx).await.unwrap();
```

The transaction creation uses:
```rust
let mut tx = Transaction::EIP1559Transaction(EIP1559Transaction {
    nonce: n,
    value: 1_u64.into(),
    gas_limit: 250000_u64,
    max_fee_per_gas: u64::MAX,  // <-- PROBLEM
    max_priority_fee_per_gas: 10_u64,
    ...
});
```

**Issue:** `max_fee_per_gas: u64::MAX` causes the balance check to fail because:
- Required balance = `gas_limit * max_fee_per_gas + value`
- With `gas_limit = 250000` and `max_fee_per_gas = u64::MAX`, this overflows

Even though genesis accounts are created with `balance: u64::MAX.into()`, the multiplication overflows.

### Code Location
`benches/benches/build_block_benchmark.rs:163-175`

## Recommended Fix

Change line 167:
```rust
// Before
max_fee_per_gas: u64::MAX,

// After
max_fee_per_gas: 1_000_000_000_u64,  // 1 gwei, reasonable value
```

## Status
**BLOCKED** - Cannot run performance benchmarks until this is fixed.

## Next Steps
1. Open issue or PR to fix the benchmark
2. Re-run baseline after fix is merged
