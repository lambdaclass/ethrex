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

### Stateless EF tests: no skips

**Where:** `tooling/ef_tests/blockchain` — `make test-stateless`, which runs the
downloaded `tests-zkevm@v0.8.0` bundle from `vectors_zkevm/`. `make test-levm` is
unaffected.

`EXTRA_SKIPS` in `tests/all.rs` is empty and the run is green over all 3218
`for_amsterdam` fixture files. Anything failing there is a real bug — do not add
a skip without replacing this section with the reason.

#### Previously: 45 fixtures skipped for a devnet-7/devnet-8 gas split

Until `tests-zkevm@v0.8.0` there was no published bundle carrying both the
EIP-8025 stateless schema and the glamsterdam-devnet-8 gas schedule, so the
vectors were generated locally from execution-specs `3c3b6f4af` — which is
devnet-7. 45 fixtures failed in `add_block_pipeline` with `GasUsedMismatch` or
`ReceiptsRootMismatch` on the gas difference alone (`COLD_STORAGE_ACCESS`
3000 vs 2100, `ACCOUNT_WRITE` 8000 vs 9000, `CALL_VALUE` 10300 vs 11300,
access-list entries at full cold cost vs cold − `WARM_ACCESS`) and were skipped
by name. v0.8.0 is filled against `tests-glamsterdam-devnet@v8.1.0`, the same
base this client targets, so the split is gone and the generator, its `uv` and
Python 3.12 prerequisites, and the whole skip list were removed with it.

#### Why this file previously claimed an Amsterdam-wide skip

`parse_and_execute` used to drop every `network >= Fork::Amsterdam` fixture from
the stateless run. Every test in the `tests-zkevm` bundle it read is
`network: Amsterdam` (23,946 of 23,946), so the stateless suite executed nothing
at all while reporting success. `parse_and_execute` now fails any stateless
fixture file that runs zero tests without a named skip, so a structural skip
cannot silently empty the suite again.

---

### The stateless schema id does not identify the encoding

**Where:** `STATELESS_INPUT_SCHEMA_ID` in `crates/common/types/stateless_ssz.rs`.

Upstream keeps the stateless input schema id at `0x1501`
(`fork_index 0x15 << 8 | revision 0x01`) across incompatible body changes. Three
encodings have now shipped under it: `tests-zkevm@v0.6.2`, then #3248 + #3278,
then #3356, which moved `state`, `codes` and `public_keys` from `SszList` to
`ProgressiveList`. ethrex speaks the last one.

The consequence is that the 2-byte prefix cannot be used to detect a stale or
mismatched bundle. A wrong-dialect input is accepted by the id check and then
fails later — in SSZ decode, or on a root that does not match — rather than being
rejected up front for what it is. `only_amsterdam_schema_id_decodes` therefore
proves less than its name suggests.

Worth raising upstream: a revision field that does not move across a body change
provides no version negotiation at all.

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
