# Known Issues

This document lists tests intentionally excluded from CI on the current branch.
It is the source of truth for the **Known Issues** section that the L1
workflow appends to each ef-tests job summary and posts as a sticky PR
comment.

> **Runtime skip list:** `tooling/ef_tests/blockchain/tests/all.rs::SKIPPED_BASE`
> is what the test harness actually consumes. The buckets and counts below
> mirror that constant.

## EF Tests — Stateless coverage narrowed to EIP-8025 optional-proofs

`make -C tooling/ef_tests/blockchain test` invokes `test-stateless-zkevm`
instead of `test-stateless`, narrowing the stateless cargo invocation to the
EIP-8025 optional-proofs suite (`-- eip8025_optional_proofs`).

**Reason.** The zkevm@v0.3.3 fixtures used by `test-stateless` are filled
against bal@v5.6.1, which is out of sync with the current bal spec
(bal-devnet-6+, plus bal-devnet-7-prep `set_delegation` re-application).
PR [#6527](https://github.com/lambdaclass/ethrex/pull/6527) broadened
`test-stateless` to extract the entire `for_amsterdam/` tree from the zkevm
bundle and run all of it under `--features stateless`; that scope trips
~549 fixtures with `GasUsedMismatch` / `ReceiptsRootMismatch` /
`BlockAccessListHashMismatch`.

Re-broaden (call `test-stateless` again from `make test`) once the zkevm
bundle is regenerated against the current bal spec.

## EF Tests — Blockchain (bal-devnet-6, Amsterdam fork only) — 74 tests

All 74 entries are anchored on `[fork_Amsterdam` in the skip list, so the
Prague / Osaka variants of the same EELS test functions still run.

**Root cause.** snobal-devnet-6 fixtures expect bal-devnet-6 spec semantics,
but our impl currently runs ahead of that on the EIP-7702 `set_delegation`
state-gas accounting (the bal-devnet-7-prep SELFDESTRUCT-style refund
subtraction was re-applied in commit `0976534cf0`).

**Resolution path.** Re-enable once we either:
- (a) bump fixtures to a snobal-devnet-7 release that locks in the new
      accounting; or
- (b) revert the bal-devnet-7-prep subtraction for bal-devnet-6 compatibility.

**Tracking.** PR [#6574](https://github.com/lambdaclass/ethrex/pull/6574).

| EIP      | Bucket                                                | Count |
| -------- | ----------------------------------------------------- | ----- |
| EIP-7702 | `set_code_txs`                                        | 24    |
| EIP-7702 | `set_code_txs_2`                                      | 15    |
| EIP-7702 | `gas`                                                 | 1     |
| EIP-8037 | `state_gas_set_code`                                  | 17    |
| EIP-8037 | `state_gas_pricing`                                   | 1    |
| EIP-8037 | `state_gas_sstore`                                    | 1    |
| EIP-7928 | `block_access_lists_eip7702`                          | 8     |
| EIP-7928 | `block_access_lists`                                  | 1     |
| EIP-7778 | `gas_accounting`                                      | 3     |
| EIP-7708 | `transfer_logs`                                       | 1     |
| EIP-7976 | `refunds`                                             | 1     |
| EIP-1344 | `chainid` (Amsterdam fork-transition fixture)         | 1     |
| **Total**|                                                       | **74**|

<details>
<summary>Full test list</summary>

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
