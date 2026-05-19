# Known Issues

Tests intentionally excluded from CI. Source of truth for the **Known
Issues** section the L1 workflow appends to each ef-tests job summary
and posts as a sticky PR comment.

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
this branch's bal-devnet-7 semantics that scope produces ~549
`GasUsedMismatch` / `ReceiptsRootMismatch` /
`BlockAccessListHashMismatch` failures.

`test-stateless-zkevm` filters cargo to the `eip8025_optional_proofs`
suite, which still validates the stateless harness without the bal-version
mismatch.

Re-broaden by switching `test:` back to `test-stateless` in
`tooling/ef_tests/blockchain/Makefile` once the zkevm bundle is regenerated
against the current bal spec.

</details>
