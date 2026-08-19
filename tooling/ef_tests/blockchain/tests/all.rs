use ef_tests_blockchain::test_runner::parse_and_execute;
use std::path::Path;

// test-levm reads the `../.fixtures_url` bundle, the Amsterdam overlay and the
// legacy tests from `vectors/`; test-stateless reads the `../.fixtures_url_zkevm`
// bundle from `vectors_zkevm/`, so the two do not overlay each other. Each pin
// lives in its URL file rather than here. See the Makefile for how they are
// populated.
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

// Do not re-add a skip without recording why in docs/known_issues.md.
const EXTRA_SKIPS: &[&str] = &[];

fn blockchain_runner(path: &Path) -> datatest_stable::Result<()> {
    let skips: Vec<&'static str> = SKIPPED_BASE
        .iter()
        .copied()
        .chain(EXTRA_SKIPS.iter().copied())
        .collect();

    // Whether to run stateless validation after the stateful run. There is no
    // backend choice any more: the in-memory paths call
    // `validate_blocks_statelessly` and the wire path calls the guest entrypoint
    // directly, so nothing dispatches on a prover backend.
    parse_and_execute(path, Some(&skips), cfg!(feature = "stateless"))
}

datatest_stable::harness!(blockchain_runner, TEST_FOLDER, r".*");
