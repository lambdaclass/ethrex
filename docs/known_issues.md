### Stateless (zkEVM) Amsterdam EF tests — entire `for_amsterdam` subtree skipped

**Where:** `tooling/ef_tests/blockchain/tests/all.rs` — the `"for_amsterdam"` entry in
`EXTRA_SKIPS` under `#[cfg(feature = "stateless")]`. Affects `make test-stateless`
(the `vectors_zkevm/` run); `make test-levm` is unaffected.

**Why:** The stateless run uses the `tests-zkevm@v0.5.0` bundle, filled against
`glamsterdam-devnet` v6.1.0, which predeploys the EIP-8282 builder deposit/exit
contracts at the OLD addresses (`0x0000884d…d9008282` / `0x000014574a…0f008282`).
This client now uses the devnet-7 addresses (`0x0000bff4…300d8282` /
`0x000064d6…800e8282`, matching the live `tests-glamsterdam-devnet@v7.2.0` bundle
used by `make test-levm`). Every Amsterdam block runs the end-of-block EIP-8282
builder system call; with the new addresses absent from the v0.5.0 bundle, each
stateless Amsterdam block fails with
`SystemContractCallFailed("System contract: 0x0000…8282 has no code after deployment")`.
The whole zkEVM bundle is `for_amsterdam`, so it is skipped wholesale rather than
per-fixture.

**Removal:** Delete the `"for_amsterdam"` entry from the stateless `EXTRA_SKIPS`
once a `tests-zkevm` bundle filled with the devnet-7 builder predeploy addresses is
released and `.fixtures_url_zkevm` is bumped to it.
