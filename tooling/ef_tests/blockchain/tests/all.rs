use ef_tests_blockchain::test_runner::parse_and_execute;
use std::path::Path;

// test-levm reads snobal-devnet-6 + legacy from `vectors/`.
// test-stateless reads the generated #3248+#3278 conformance vectors from a separate
// `vectors_stateless_3278/` so the bundles do not overlay each other.
#[cfg(feature = "stateless")]
const TEST_FOLDER: &str = "vectors_zkevm/";
#[cfg(not(feature = "stateless"))]
const TEST_FOLDER: &str = "vectors/";

// Base skips shared by all runs.
const SKIPPED_BASE: &[&str] = &[
    // Skip because they take too long to run, but they pass
    "static_Call50000_sha256",
    "CALLBlake2f_MaxRounds",
    "loopMul",
    // Skip because it tries to deserialize number > U256::MAX
    "ValueOverflowParis",
    // Skip because it's a "Create" Blob Transaction, which doesn't actually exist. It never reaches the EVM because we can't even parse it as an actual Transaction.
    "createBlobhashTx",
];

// Extra skips added only for prover backends.
// The stateless run executes the zkevm@v0.6.2 bundle (`vectors_zkevm/`), filled against
// `tests-glamsterdam-devnet@v7.2.0` — the same base as the live `vectors/` fixtures on this
// branch. v0.6.2 fixes the EIP-8282 fill (PR ethereum/execution-specs#3157): the canonical
// `SszExecutionRequests` now carries the builder-deposit (0x03) and builder-exit (0x04) request
// lists, mirrored in `stateless_ssz::ExecutionRequests`. The whole bundle re-executes cleanly, so
// no blanket skip and no per-fork skip are needed. Per-fixture leniency cases
// (`*_extra_unused_*` padding, deliberately-invalid witnesses) are handled in `test_runner.rs`.
// Amsterdam+ fixtures are skipped in the stateless run by fork (see
// `parse_and_execute` in `test_runner.rs` and docs/known_issues.md): the
// tests-zkevm@v0.5.0 bundle predeploys the EIP-8282 builder contracts at the OLD
// addresses, incompatible with this client's devnet-7 addresses. That skip is
// fork-based (not name-based), so no per-test entries are needed here.
#[cfg(feature = "stateless")]
const EXTRA_SKIPS: &[&str] = &[];
#[cfg(not(feature = "stateless"))]
const EXTRA_SKIPS: &[&str] = &[];

// Whether to run stateless validation after the stateful run. There is no backend
// choice any more: the in-memory paths call `validate_blocks_statelessly` and the
// wire path calls the guest entrypoint directly, so nothing dispatches on a
// prover backend.
#[cfg(feature = "stateless")]
const RUN_STATELESS: bool = true;
#[cfg(not(feature = "stateless"))]
const RUN_STATELESS: bool = false;

fn blockchain_runner(path: &Path) -> datatest_stable::Result<()> {
    // Compose the final skip list
    let skips: Vec<&'static str> = SKIPPED_BASE
        .iter()
        .copied()
        .chain(EXTRA_SKIPS.iter().copied())
        .collect();

    parse_and_execute(path, Some(&skips), RUN_STATELESS)
}

datatest_stable::harness!(blockchain_runner, TEST_FOLDER, r".*");
