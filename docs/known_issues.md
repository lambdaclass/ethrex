# Known Issues

Tests intentionally excluded from CI. Source of truth for the **Known
Issues** section the L1 workflow appends to each ef-tests job summary
and posts as a sticky PR comment.

## Hive — bal-devnet-6 Amsterdam consume-engine tests — 32 functions / 54 cases

Same root cause as the blockchain-runner skip list (see *EF Tests —
Blockchain* below): snobal-devnet-6 fixtures expect bal-devnet-6 spec
semantics, but our impl runs ahead due to the bal-devnet-7-prep
`set_delegation` SELFDESTRUCT-style refund subtraction. These fixtures
are routed through hive's `eels/consume-engine` simulator and produce
the same failures. Excluded via `KNOWN_EXCLUDED_TESTS` (substring
match on `test_<name>[fork_Amsterdam`, anchoring to the Amsterdam fork
so legacy Prague/Osaka variants still run).

<details>
<summary>Affected EELS test functions (32)</summary>

- `test_auth_refund_block_gas_accounting`
- `test_auth_refund_bypasses_one_fifth_cap`
- `test_auth_state_gas_scales_with_cpsb`
- `test_auth_with_calldata_and_access_list`
- `test_auth_with_multiple_sstores`
- `test_authorization_exact_state_gas_boundary`
- `test_authorization_to_precompile_address`
- `test_authorization_with_sstore`
- `test_bal_7702_delegation_clear`
- `test_bal_7702_delegation_create`
- `test_bal_7702_delegation_update`
- `test_bal_7702_double_auth_reset`
- `test_bal_7702_double_auth_swap`
- `test_bal_7702_null_address_delegation_no_code_change`
- `test_bal_all_transaction_types`
- `test_bal_selfdestruct_to_7702_delegation`
- `test_bal_withdrawal_to_7702_delegation`
- `test_duplicate_signer_authorizations`
- `test_existing_account_auth_header_gas_used_uses_worst_case`
- `test_existing_account_refund`
- `test_existing_account_refund_enables_sstore`
- `test_existing_auth_with_reverted_execution_preserves_intrinsic`
- `test_many_authorizations_state_gas`
- `test_mixed_auths_header_gas_used_uses_worst_case`
- `test_mixed_new_and_existing_auths`
- `test_mixed_valid_and_invalid_auths`
- `test_multi_tx_block_auth_refund_and_sstore`
- `test_multiple_refund_types_in_one_tx`
- `test_simple_gas_accounting`
- `test_sstore_state_gas_all_tx_types`
- `test_transfer_with_all_tx_types`
- `test_varying_calldata_costs`

</details>

## EF Tests — Stateless coverage narrowed to EIP-8025 optional-proofs

`make -C tooling/ef_tests/blockchain test` calls `test-stateless-zkevm`
instead of `test-stateless`. The zkevm@v0.3.3 fixtures are filled against
bal@v5.6.1, out of sync with current bal spec; the broad target trips ~549
fixtures. Re-broaden once the zkevm bundle is regenerated.

<details>
<summary>Why and resolution path</summary>

[PR #6527](https://github.com/lambdaclass/ethrex/pull/6527) broadened
`test-stateless` to extract the entire `for_amsterdam/` tree from the
zkevm bundle and run all of it under `--features stateless`; combined with
this branch's bal-devnet-6+ semantics (and bal-devnet-7-prep
`set_delegation` re-application) that scope produces ~549
`GasUsedMismatch` / `ReceiptsRootMismatch` /
`BlockAccessListHashMismatch` failures.

`test-stateless-zkevm` filters cargo to the `eip8025_optional_proofs`
suite, which still validates the stateless harness without the bal-version
mismatch.

Re-broaden by switching `test:` back to `test-stateless` in
`tooling/ef_tests/blockchain/Makefile` once the zkevm bundle is regenerated
against the current bal spec.

</details>

## EF Tests — Blockchain bal-devnet-6 (Amsterdam fork) — 74 tests

snobal-devnet-6 fixtures expect bal-devnet-6 spec semantics, but our impl
runs ahead due to the bal-devnet-7-prep `set_delegation` SELFDESTRUCT-style
refund subtraction. Skipped in
`tooling/ef_tests/blockchain/tests/all.rs::SKIPPED_BASE`, anchored on
`[fork_Amsterdam` so legacy Prague / Osaka variants still run.

<details>
<summary>Bucket breakdown (74 total) and resolution path</summary>

| EIP      | Bucket                                                | Count |
| -------- | ----------------------------------------------------- | ----- |
| EIP-7702 | `set_code_txs`                                        | 24    |
| EIP-7702 | `set_code_txs_2`                                      | 15    |
| EIP-7702 | `gas`                                                 | 1     |
| EIP-8037 | `state_gas_set_code`                                  | 17    |
| EIP-8037 | `state_gas_pricing`                                   | 1     |
| EIP-8037 | `state_gas_sstore`                                    | 1     |
| EIP-7928 | `block_access_lists_eip7702`                          | 8     |
| EIP-7928 | `block_access_lists`                                  | 1     |
| EIP-7778 | `gas_accounting`                                      | 3     |
| EIP-7708 | `transfer_logs`                                       | 1     |
| EIP-7976 | `refunds`                                             | 1     |
| EIP-1344 | `chainid` (Amsterdam fork-transition fixture)         | 1     |
| **Total**|                                                       | **74**|

Re-enable once we either:
- (a) bump fixtures to a snobal-devnet-7 release that locks in the new
      accounting; or
- (b) revert the bal-devnet-7-prep subtraction for bal-devnet-6
      compatibility.

</details>

<details>
<summary>Full test list (74)</summary>

**EIP-7702 — `for_amsterdam/prague/eip7702_set_code_tx/set_code_txs/`**
- `delegation_clearing`
- `delegation_clearing_and_set`
- `delegation_clearing_failing_tx`
- `delegation_clearing_tx_to`
- `eoa_tx_after_set_code`
- `ext_code_on_chain_delegating_set_code`
- `ext_code_on_self_delegating_set_code`
- `ext_code_on_self_set_code`
- `ext_code_on_set_code`
- `many_delegations`
- `nonce_overflow_after_first_authorization`
- `nonce_validity`
- `reset_code`
- `self_code_on_set_code`
- `self_sponsored_set_code`
- `set_code_multiple_valid_authorization_tuples_same_signer_increasing_nonce`
- `set_code_multiple_valid_authorization_tuples_same_signer_increasing_nonce_self_sponsored`
- `set_code_to_log`
- `set_code_to_non_empty_storage_non_zero_nonce`
- `set_code_to_self_destruct`
- `set_code_to_self_destructing_account_deployed_in_same_tx`
- `set_code_to_sstore`
- `set_code_to_sstore_then_sload`
- `set_code_to_system_contract`

**EIP-7702 — `for_amsterdam/prague/eip7702_set_code_tx/set_code_txs_2/`**
- `call_pointer_to_created_from_create_after_oog_call_again`
- `call_to_precompile_in_pointer_context`
- `contract_storage_to_pointer_with_storage`
- `delegation_replacement_call_previous_contract`
- `double_auth`
- `pointer_measurements`
- `pointer_normal`
- `pointer_reentry`
- `pointer_resets_an_empty_code_account_with_storage`
- `pointer_reverts`
- `pointer_to_pointer`
- `pointer_to_precompile`
- `pointer_to_static`
- `pointer_to_static_reentry`
- `static_to_pointer`

**EIP-7702 — `for_amsterdam/prague/eip7702_set_code_tx/gas/`**
- `account_warming`

**EIP-8037 — `for_amsterdam/amsterdam/eip8037_state_creation_gas_cost_increase/state_gas_set_code/`**
- `auth_refund_block_gas_accounting`
- `auth_refund_bypasses_one_fifth_cap`
- `auth_with_calldata_and_access_list`
- `auth_with_multiple_sstores`
- `authorization_exact_state_gas_boundary`
- `authorization_to_precompile_address`
- `authorization_with_sstore`
- `duplicate_signer_authorizations`
- `existing_account_auth_header_gas_used_uses_worst_case`
- `existing_account_refund`
- `existing_account_refund_enables_sstore`
- `existing_auth_with_reverted_execution_preserves_intrinsic`
- `many_authorizations_state_gas`
- `mixed_auths_header_gas_used_uses_worst_case`
- `mixed_new_and_existing_auths`
- `mixed_valid_and_invalid_auths`
- `multi_tx_block_auth_refund_and_sstore`

**EIP-8037 — `state_gas_pricing/`**
- `auth_state_gas_scales_with_cpsb`

**EIP-8037 — `state_gas_sstore/`**
- `sstore_state_gas_all_tx_types`

**EIP-7928 — `for_amsterdam/amsterdam/eip7928_block_level_access_lists/block_access_lists_eip7702/`**
- `bal_7702_delegation_clear`
- `bal_7702_delegation_create`
- `bal_7702_delegation_update`
- `bal_7702_double_auth_reset`
- `bal_7702_double_auth_swap`
- `bal_7702_null_address_delegation_no_code_change`
- `bal_selfdestruct_to_7702_delegation`
- `bal_withdrawal_to_7702_delegation`

**EIP-7928 — `block_access_lists/`**
- `bal_all_transaction_types`

**EIP-7778 — `for_amsterdam/amsterdam/eip7778_block_gas_accounting_without_refunds/gas_accounting/`**
- `multiple_refund_types_in_one_tx`
- `simple_gas_accounting`
- `varying_calldata_costs`

**EIP-7708 — `for_amsterdam/amsterdam/eip7708_eth_transfer_logs/transfer_logs/`**
- `transfer_with_all_tx_types`

**EIP-7976 — `for_amsterdam/amsterdam/eip7976_increase_calldata_floor_cost/refunds/`**
- `gas_refunds_from_data_floor`

**EIP-1344 — `for_amsterdam/istanbul/eip1344_chainid/chainid/`**
- `chainid` (Amsterdam fork-transition fixture)

</details>
