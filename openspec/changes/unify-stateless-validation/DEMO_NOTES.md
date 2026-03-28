# Demo Notes

## Build Commands

### EIP-8025 / Stateless EF Tests
```bash
cd tooling/ef_tests/blockchain
cargo test --profile release-with-debug --features stateless -- --test-threads=4
```
Result: 6053/6158 pass (96.3%). Failures are in Amsterdam EIPs (pre-existing).

### Native Rollups L2 Binary
```bash
COMPILE_CONTRACTS=true cargo build -p ethrex --release --features l2,l2-sql,stateless-validation
```
**NOTE:** Must use `-p ethrex` — the features `l2,l2-sql` only exist on the `ethrex` binary crate.

### Starting L1 for Native Rollups
```bash
NATIVE_ROLLUPS=1 make -C crates/l2 init-l1
```
This runs `cargo run --release --features stateless-validation`.

### Deploy + Start L2
```bash
NATIVE_ROLLUPS=1 make -C crates/l2 native-rollup
```

## Known Issues Found During Implementation

### Feature Propagation
- The `stateless-validation` feature must propagate through the entire crate graph
- Missing propagation to `ethrex-l2-rpc` caused `proof_engine` field errors
- Each Cargo.toml that depends on a crate with `stateless-validation` must forward it

### VM::new Parameter
- `VM::new` takes `stateless_validator: Option<&dyn StatelessValidator>` as 7th param
- ALL call sites (~30) must pass this — use `None` when not needed
- Easy to miss: `tracing.rs`, test files, bench, runner all call `VM::new`

### Main Branch API Changes (Merge Artifacts)
- `compute_block_hash()` now requires `&dyn Crypto` parameter
- `compute_transactions_root()` and `compute_withdrawals_root()` require `&dyn Crypto`
- `validate_block_body()` requires `&dyn Crypto`
- `Evm::new_for_l1` requires `Arc<dyn Crypto>`
- `execute_blocks()` takes `crypto: Arc<dyn Crypto>` parameter
- `BlockchainOptions` no longer has `precompute_witnesses` field
- `Bloom.0` is `[u8; 256]`, not `[[u8; ...]; ...]` — use `.0.to_vec()` not `.0.iter().flat_map(|b| b.to_vec())`

### CLI Flag Names
- Cargo feature: `stateless-validation`
- CLI flags: `--stateless-validation`, `--stateless-validation.contract-address`, etc.
- Rust struct field names: still `native_rollups`, `native_rollup_opts` (domain concept, not renamed)
- Makefile: `NATIVE_ROLLUPS=1` env var triggers the feature

### Viewing Logs
- **DO NOT** use `make -C crates/l2 init-l2 2>&1 | tail -20` — the `| tail` pipe waits for the long-running node process to exit, so you never see output
- Instead, run the binary directly with `RUST_LOG=debug` to see real-time output
- Or redirect to a log file: `... > /tmp/l2.log 2>&1 &` then `tail -f /tmp/l2.log`
- The Makefile targets (`init-l1`, `init-l2`) use `cargo run` which recompiles, adding delay before any output appears

### VM::new and StatelessValidator Threading
- The validator must flow through ALL code paths that eventually call `VM::new`
- Critical paths that were missed:
  - `Evm::simulate_tx_from_generic` → `LEVM::simulate_tx_from_generic` → `vm_from_generic` → `VM::new`
  - This path is used by `eth_estimateGas` and `eth_call` RPCs
  - The `advance()` transaction hits this path during gas estimation on L1
- Always search for ALL callers when adding a parameter to a function

### SSZ Encoding
- `SszList::new()` + `list.push(item)` pattern (no `from_iter`)
- `ssz::encode()` is private — use `value.ssz_append(&mut buf)` for serialization
- `SszDecode::from_ssz_bytes(&buf)` for deserialization
- `Bloom` (logs_bloom) needs conversion to `SszVector<u8, 256>` via `try_into()`
