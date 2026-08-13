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
