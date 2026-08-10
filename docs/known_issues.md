### Stateless EF tests: 5 fixtures skipped

**Where:** `tooling/ef_tests/blockchain` — `make test-stateless`, which runs the
generated `vectors_stateless_3278/` conformance set (execution-specs
`3c3b6f4af`, i.e. #3248 progressive SSZ + #3278 `ChainConfig` removal).
`make test-levm` is unaffected.

#### Skipped: 5 fixtures that fail in ordinary execution

Listed in `EXTRA_SKIPS` in `tests/all.rs`:

- `test_witness_codes_auth_nonce_mismatch`
- `test_witness_codes_redelegation_old_marker_included_new_marker_excluded`
- `test_witness_codes_reset_delegation`
- `test_witness_codes_failed_create_after_initcode_read`
- `test_validation_codes_missing_redelegation_old_marker`

These fail in `add_block_pipeline` with `GasUsedMismatch` or
`ReceiptsRootMismatch`, before any stateless validation runs. Running the same
vector set without the `stateless` feature reproduces all five identically, so
they are ordinary block-execution divergences, not stateless conformance gaps.

They are spec-base skew, not client bugs. Measured:

- ethrex passes all five against the `tests-zkevm@v0.6.2` versions of the same
  tests.
- The eth-act witness dashboard agrees: ethrex is 22829/22829 on
  `eels/consume-engine-witness` and 19413/19413 on `stateless-validator sp1`
  for that bundle.
- Diffing one test (`witness_codes_reset_delegation`) across the two fixture
  sets: `pre` and `genesisBlockHeader` are byte-identical and it is the same
  transaction hash, yet `postState`, `receipts`, `stateRoot`, `receiptTrie` and
  `blockAccessListHash` all differ — the receipt's `cumulativeGasUsed` goes
  30816 → 24653. Same input, different output.

So execution semantics changed between v0.6.2's fill base and execution-specs
master at `3c3b6f4af`, and ethrex implements the former, which is the
`glamsterdam-devnet@v7.2.0` base it targets. The regular `vectors/` bundle has no
`eip8025_optional_proofs` tests, so `make test-levm` never sees them.

**Removal:** re-check once ethrex moves to a spec base at or past `3c3b6f4af`.
If they still fail then, they are real execution bugs and the skips must go.

#### Why this file previously claimed an Amsterdam-wide skip

`parse_and_execute` used to drop every `network >= Fork::Amsterdam` fixture from
the stateless run. Every test in the `tests-zkevm` bundle it read is
`network: Amsterdam` (23,946 of 23,946), so the stateless suite executed nothing
at all while reporting success. `parse_and_execute` now fails any stateless
fixture file that runs zero tests without a named skip, so a structural skip
cannot silently empty the suite again.
