### Stateless (zkEVM) Amsterdam+ EF tests skipped

**Where:** `tooling/ef_tests/blockchain/test_runner.rs` — `parse_and_execute` skips
fixtures with `network >= Fork::Amsterdam` when running with a stateless backend.
Affects `make test-stateless` (the `vectors_zkevm/` run); `make test-levm` is
unaffected.

**Why:** the stateless run uses the `tests-zkevm@v0.6.2` bundle, the newest zkEVM
release, filled against `tests-glamsterdam-devnet@v7.2.0`. This client targets
`glamsterdam-devnet-8`, whose gas schedule diverges from devnet-7: EIP-2780 folds the
EIP-7708 transfer log cost into `TX_VALUE_COST`, and EIP-8038 reprices access-list
entries to the cold cost minus `WARM_ACCESS` (3000 → 2900 per address and per storage
key). Every Amsterdam+ fixture in the bundle therefore carries devnet-7 gas
expectations that no longer match execution. The skip is by fork rather than by test
name, since cross-fork directories such as `for_amsterdam/prague/...` still execute at
the Amsterdam fork.

**Removal:** delete the `skip_stateless_amsterdam` branch in `parse_and_execute` once a
`tests-zkevm` bundle filled against `glamsterdam-devnet-8` is released and
`.fixtures_url_zkevm` is bumped to it.
