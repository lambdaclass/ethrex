# Known Issues

Tests intentionally excluded from CI. Source of truth for the **Known
Issues** section the L1 workflow appends to each ef-tests job summary
and posts as a sticky PR comment.

## EF Tests — Stateless `stateless_input_invalid_public_key_is_rejected` skipped

The single fixture
`tests/amsterdam/eip8025_optional_proofs/test_stateless_input_validation.py::test_stateless_input_invalid_public_key_is_rejected`
is filtered out of `test-stateless`. All other ~2864 stateless fixtures
from `tests-zkevm@v0.4.1` (bal@v7.2.0 baseline) pass.

<details>
<summary>Why and resolution path</summary>

`tests-zkevm@v0.4.x` introduces the v0.4 stateless wire format: a new
schema-id prefix and a populated `public_keys` field on
`StatelessInput`. Two gaps prevent the wrong-pubkey rejection from
being observable end-to-end in ethrex's harness:

1. `decode_eip8025` still parses the v0.3 framing (no schema-id, old
   `ChainConfig` shape), so v0.4 canonical bytes don't round-trip.
2. The blockchain ef_test runner consumes JSON `executionWitness`
   instead of the canonical `statelessInputBytes`, bypassing the
   `public_keys` check entirely.

Resolution: update `decode_eip8025` for the v0.4 schema-id + reshaped
`ChainConfig`, then route the stateless test runner through
`execution_program(bytes)` (or replicate the public-keys check against
the canonical input). The skip site in
`tooling/ef_tests/blockchain/tests/all.rs` carries the same note.

</details>
