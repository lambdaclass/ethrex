# L2 Hook Tests

Unit and integration tests for the `L2Hook` implementation in `ethrex-levm`.

The goal of these tests is to lock in the expected behavior of the L2 VM's
fee handling, privileged transaction processing, and gas accounting. If a
future change causes a test to fail, that signals that L2 behavior has
changed and needs review.

## Structure

```
hooks/
├── mod.rs                    # Module declarations
├── test_utils.rs             # Shared helpers: addresses, DB, tx, env, fee config, VM creation
├── l2_hook_tests.rs          # Pure function tests: ABI encoding, L1 fee math, hook factory
├── vm_integration_tests.rs   # End-to-end VM execution tests
├── fee_calculation_tests.rs  # Exact fee math verification (base fee, operator, coinbase, L1)
├── error_path_tests.rs       # Error handling, validation failures, revert behavior
├── edge_case_tests.rs        # Boundary values: L1 gas overflow, priority fee edge cases
├── fee_token_tests.rs        # ERC20 fee token encoding and math (pure tests, no VM)
└── README.md                 # This file
```

## Fee Model Summary

L2 transactions pay fees distributed to multiple recipients:

```
gas_price = min(max_priority_fee + base_fee + operator_fee, max_fee)

sender pays: gas_price * gas_used + l1_fee

Distribution:
  base_fee_vault:     base_fee_per_gas * gas_used
  operator_vault:     operator_fee_per_gas * gas_used
  coinbase:           (gas_price - base_fee - operator_fee) * gas_used
  l1_fee_vault:       l1_fee_per_blob_gas * (GAS_PER_BLOB / SAFE_BYTES_PER_BLOB) * tx_size
```

Validation enforces `max_fee >= base_fee + operator_fee`, so the operator is
always paid even when `max_priority_fee = 0` (coinbase gets nothing in that case).

Special transaction types:
- **Privileged (bridge)**: from `COMMON_BRIDGE_L2_ADDRESS` (0x...ffff), no gas fees, no nonce checks, can mint ETH
- **Fee token**: ERC20 payment instead of ETH, with ratio scaling via registry contracts

## Running Tests

```bash
# All L2 hook tests
cargo test -p ethrex-test tests::l2::hooks

# Specific module
cargo test -p ethrex-test tests::l2::hooks::fee_calculation_tests

# Single test
cargo test -p ethrex-test test_base_fee_vault_receives_exact_amount
```

## Adding Tests

1. Choose the file by category:
   - Pure function tests → `l2_hook_tests.rs`
   - Fee distribution math → `fee_calculation_tests.rs`
   - Error/validation paths → `error_path_tests.rs`
   - Boundary/overflow cases → `edge_case_tests.rs`
   - Fee token (ERC20) → `fee_token_tests.rs`
   - General VM behavior → `vm_integration_tests.rs`

2. Use shared helpers from `test_utils.rs`:
   - `create_test_l2_vm(env, db, tx, fee_config)` — VM with L2 hooks
   - `create_eip1559_tx(to, value, gas_limit, max_fee, max_priority_fee, nonce)` — EIP-1559 tx
   - `create_eip1559_env(origin, gas_limit, max_fee, max_priority_fee, base_fee, is_privileged)` — execution environment
   - `create_test_fee_config(...)` — fee config with optional operator/L1 components
   - `create_test_db_with_accounts(accounts)` — in-memory DB
   - Constants: `TEST_SENDER`, `TEST_RECIPIENT`, `TEST_COINBASE`, `DEFAULT_GAS_LIMIT`, etc.

3. Prefer hardcoded expected values over formula-computed ones. A test that re-implements
   the production formula and asserts equality doesn't catch bugs — it just confirms the
   formula matches itself.

## Known Gaps

### Fee Token VM Integration Tests

The ERC20 fee token path (`prepare_execution_fee_token`, `finalize_non_privileged_execution`
with `use_fee_token = true`) requires mock contract bytecode deployed at system addresses
(`FEE_TOKEN_REGISTRY_ADDRESS`, `FEE_TOKEN_RATIO_ADDRESS`, fee token address). This
infrastructure does not exist yet. Only pure encoding and math tests exist in
`fee_token_tests.rs`.

### Hook Trait Method Tests

The following test scenarios require direct Hook trait method access or more complex
VM setup and have not yet been implemented:

**Privileged transaction preparation:**
- Bridge minting ETH (zero-balance sender)
- Insufficient balance revert for non-bridge privileged
- Intrinsic gas failure revert
- Nonce check skip
- Gas allowance validation
- Value transfer mechanics

**Fee token preparation:**
- Registration validation via actual registry contract
- Ratio lookup via actual ratio contract
- Upfront cost deduction via lockFee call
- Sender balance/nonce/EOA validation

**Finalization (non-privileged):**
- L1 gas exceeding limit causes revert
- L1 fee vault payment
- Base fee vault payment
- Operator fee payment
- Coinbase priority fee payment
- Sender gas refund

**Finalization (fee token):**
- Payment via payFee contract calls
- Refund via payFee contract calls
- Base fee burn when no vault configured
- Ratio applied to all payments
