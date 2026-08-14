### rpc-compat log-bearing cases excluded

**Where:** `KNOWN_EXCLUDED_TESTS` in `.github/scripts/check-hive-results.sh` counts out
eight hive `rpc-compat` cases — the four `eth_getLogs` cases, `eth_getBlockReceipts/get-block-receipts-latest`,
and three `eth_getTransactionReceipt` cases. They are exactly the cases whose recorded
response contains at least one log object; every case with an empty log array still runs.
Note this leaves `eth_getLogs` with no rpc-compat coverage at all, since all four of its
cases are in the set.

**Why:** ethrex populates `blockTimestamp` on log objects, as geth, besu, nethermind, reth
and erigon all do. hive's rpc-compat compares responses byte-exactly (`jsondiff.FullMatch`;
the lenient `checkJSONStructure` path applies only to cases upstream marks `speconly`), and
the corpus is pinned to execution-apis `d08382ae` (2025-02-10), whose recordings predate the
field — it entered the schema in execution-apis#639 and the fixtures in #846 (2026-07-22).
So the extra key cannot match, and this is a property of the pin rather than of the response.

**The pin cannot move, and this is not temporary.** The pin sits one commit before
execution-apis#627, which moved the test chain to a pre-merge genesis: the current corpus has
~36 proof-of-work blocks before its terminal total difficulty. ethrex does not support
pre-merge chains and will not, so importing that `chain.rlp` fails at block 1 —
`validate_block_header` has no pre-London base-fee path. Every revision carrying
`blockTimestamp` in its fixtures also carries that chain, so there is no revision that
satisfies both. Nor can the corpus be patched locally: rpc-compat's Dockerfile clones
`ethereum/execution-apis` by hard-coded URL, so the `branch` buildarg cannot point at a fork.

**Coverage:** the field itself is pinned by
`block_timestamp_is_on_the_log_and_not_on_the_receipt` in
`crates/networking/rpc/types/receipt.rs`, which asserts it is present on each log and absent
from the receipt level.

**Removal:** delete the entries if ethrex ever gains pre-merge chain import, or if upstream
marks these cases `speconly` so they are type-checked instead of compared byte-for-byte.

---

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

---

### ZisK guest program hash changes with the `unsync_cell` gate

**Where:** `crates/common/types/block.rs`, `transaction.rs`.

The gate on the single-threaded `unsync_cell::OnceCell` moved from
`all(feature = "eip-8025", target_arch = "riscv64")` to
`all(feature = "zisk", target_arch = "riscv64")` when the `eip-8025` feature was removed.

The guest ELFs were previously built `--features "<zkvm>-build-elf,ci"`, which never enabled
`eip-8025`, so they compiled the atomic `once_cell` variant. `bin/zisk/Cargo.toml` does enable
`ethrex-common/zisk`, so **the ZisK guest now compiles the `unsafe impl Sync` cell instead**.
That changes the ELF bytes and therefore the program hash and verification key.

This is intended (the guest is single-threaded, so the unsync cell is sound and cheaper), but it
is a VK change rather than a no-op refactor, and the diffstat presents it as a file rename
(`eip8025_cell.rs` → `unsync_cell.rs`). Anyone pinning a ZisK VK across this change must
re-register it. The `stateless-validator` crate now forwards `ethrex-common/zisk` from its own
`zisk` feature so the two ZisK guests do not disagree on the cell type.

---

### Release signing key is an unprotected repository secret

**Where:** `.github/workflows/tag_release.yaml`.

`MINISIGN_SECRET_KEY` is a plain repository secret. There is no `environment:` on
`finalize-release` or `dry-run-release-assets`, and `gh api repos/lambdaclass/ethrex/rulesets`
shows only branch-targeted rulesets, so the `github.ref_type == 'tag'` condition is a workflow
check rather than an enforced boundary: anyone who can push a tag can reach the signing key.

This is a repository-settings change, not a code change, so it is recorded here rather than
fixed in the tree. Recommended:

1. Move `MINISIGN_SECRET_KEY` / `MINISIGN_PASSWORD` into a GitHub **Environment** with required
   reviewers, and add `environment:` to the two jobs that sign.
2. Add a ruleset targeting `refs/tags/v*` restricting who may create release tags.

Until then, the compromise of that key is silent and durable: signatures would still verify
against the committed `.github/minisign.pub`.
