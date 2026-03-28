## Why

The l2beat native rollups spec has undergone a major rewrite (l2beat/native-rollups PR #2, merged March 2026). The spec now defines `verify_stateless_new_payload` — the same function the L1 ZK-EVM effort uses — as the core verification function for the EXECUTE precompile. Our current PoC uses a custom `apply_body` variant with ABI-encoded fields and a duplicated `execute_block` inside LEVM. Meanwhile, PR #6361 implements EIP-8025 with SSZ types and stateless execution that overlap significantly with what the spec now requires. Unifying both implementations under the spec's architecture eliminates duplication, aligns with the upstream spec, and produces a clean demo for EF review.

## What Changes

- **Merge EIP-8025 (PR #6361) into a new combined branch** with the native rollups EXECUTE precompile
- **Rename `execution_program` (EIP-8025 mode) to `verify_stateless_new_payload`** to match the execution-specs naming
- **Unify feature flags** — replace separate `native-rollup` and `eip-8025` flags with a single flag
- **Rewrite the EXECUTE precompile** to match the new spec:
  - SSZ-serialized `StatelessInput` as input (replacing ABI-encoded individual fields)
  - SSZ-serialized `StatelessValidationResult` as output (replacing ABI 160-byte return)
  - L2-specific preprocessing as an explicit layer before calling `verify_stateless_new_payload`
  - Gas charging based on `execution_payload.gas_used` (replacing fixed 100k)
  - L1 anchor via repurposed `parent_beacon_block_root` field
- **Break the LEVM cycle dependency** by introducing a `StatelessValidator` trait in LEVM, implemented in `crates/blockchain/`, and injected at EVM construction time
- **Delete the duplicated `execute_block`** from `execute_precompile.rs` in LEVM
- **Add new SSZ types**: `SszStatelessInput`, `SszStatelessValidationResult`, `SszChainConfig`, `SszExecutionWitness`
- **Move `chain_config` out of `ExecutionWitness`** into a sibling field on `StatelessInput` to match the spec layout
- **Update `NativeRollup.sol`** to match the spec's contract (add `blockHash`/`chainId`, remove `lastBaseFeePerGas`/`lastGasUsed`/`relayer`/`advancer`, use SSZ encoding for EXECUTE calldata)
- **Update documentation** (`native_rollups.md`, `native_rollups_gap_analysis.md`) to reflect spec changes and remaining differences

## Capabilities

### New Capabilities

- `stateless-validation`: Shared `verify_stateless_new_payload` function callable from the EXECUTE precompile, EIP-8025 RPC endpoints, and zkVM guest programs. Includes the `StatelessValidator` trait for cycle-free dependency injection.
- `ssz-stateless-types`: SSZ container types for `StatelessInput`, `StatelessValidationResult`, `ChainConfig`, and `ExecutionWitness`, extending the existing EIP-8025 SSZ types.
- `execute-precompile-v2`: Rewritten EXECUTE precompile aligned with the l2beat spec — SSZ input/output, L2 preprocessing layer, gas charging by `gas_used`, L1 anchor via `parent_beacon_block_root`.

### Modified Capabilities

_(No existing specs to modify)_

## Impact

- **Crates affected**: `ethrex-vm` (LEVM — trait definition, precompile rewrite), `ethrex-blockchain` (trait implementation, `verify_stateless_new_payload`), `ethrex-common` (SSZ types, `ExecutionWitness` refactor), `ethrex-guest-program` (rename, new input/output types)
- **Contracts**: `NativeRollup.sol` (storage layout, `advance()` calldata format), `L2Bridge.sol` and `L1Anchor.sol` (minor adjustments for `parent_beacon_block_root` anchoring)
- **Tests**: Native rollup unit tests and integration tests need updating for SSZ format and new contract ABI
- **Dependencies**: `libssz` (already in PR #6361), no new external dependencies
- **Demos**: Both EIP-8025 proof flow and native rollups advance flow must work end-to-end
