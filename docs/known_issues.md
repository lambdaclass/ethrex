### Stateless blockchain EF-tests (zkevm bundle) skipped under Amsterdam v6.1.1

`make -C tooling/ef_tests/blockchain test-stateless` runs against the
`tests-zkevm@v0.4.1` fixture bundle — currently the only published zkevm test
release. Those fixtures were filled against an older glamsterdam devnet and
re-execute every case under the `for_amsterdam` fork, so they lag the
glamsterdam-devnet **v6.1.1** gas accounting this client now implements
(EIP-8037 / EIP-8038 / EIP-2780 / EIP-7976 / EIP-7981 …). Re-executing them
yields ~`2790/2864` stale-gas failures ("Transaction execution unexpectedly
failed"), spread pervasively across every fork and through the EIP-8025 proof
suite, so there is no clean passing subset to keep.

Until a v6.1.1-aligned zkevm bundle is published, the entire bundle is skipped
for the stateless run via the `fork_Amsterdam` entry in the stateless-only
`EXTRA_SKIPS` (`tooling/ef_tests/blockchain/tests/all.rs`) — every test in this
Amsterdam-only bundle carries the `[fork_Amsterdam-…]` parametrization in its
test key. The skip is `#[cfg(feature = "stateless")]`, so it does **not** touch
the non-stateless `test-levm` run. Coverage of these EIPs is retained by
`test-levm`, the engine EF-tests, and the state EF-tests, all of which execute
against the live v6.1.1 fixtures and pass.

Re-enable by removing the `"fork_Amsterdam"` skip once `.fixtures_url_zkevm`
points at a zkevm bundle filled for glamsterdam-devnet v6.1.1.
