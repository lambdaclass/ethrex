# Known issues

## Stateless (zkevm) suite skips CREATE-in-static gas tests (outdated bundle)

The stateless ef-test suite (`make -C tooling/ef_tests/blockchain test-stateless`) runs the
`tests-zkevm@v0.4.1` fixture bundle, which is the latest zkevm release available upstream but is
filled against an older Amsterdam base (`tests-bal@v7.2.0` era). It predates the
`tests-bal@v7.3.0` refinement of CREATE/CREATE2-in-static-context gas accounting, where EIP-7778's
regular-gas dimension excludes the reserved (unused) `create_message_gas` instead of burning the
whole forwarded frame.

ethrex implements the v7.3.0 behavior (verified by the amsterdam `test-levm` suite, which passes),
so it intentionally mismatches the stale zkevm block headers for these tests. They are skipped in
the stateless run via `EXTRA_SKIPS` in `tooling/ef_tests/blockchain/tests/all.rs`:

- `bal_create_in_static_context` (CREATE/CREATE2, with/without value)
- `create2check_fields_in_initcode` (d3, d7)
- `test_static_call_create.py` (d1)

Action: remove these skips once a `tests-zkevm` bundle filled against `tests-bal@v7.3.0` (or later)
is released and `.fixtures_url_zkevm` is bumped.
