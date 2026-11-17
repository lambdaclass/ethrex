use ef_tests_blockchain::test_runner::{SKIPPED_TESTS, parse_and_execute};
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

// The same bundle's `blockchain_test_engine` fixtures, kept in their own root
// because the blockchain runner above would otherwise try to parse them. Only
// their stateless bytes are checked here; see `check_engine_stateless_bytes`.
#[cfg(feature = "stateless")]
const ENGINE_TEST_FOLDER: &str = "vectors_zkevm_engine/";

// Do not re-add a skip without recording why in docs/known_issues.md.
const EXTRA_SKIPS: &[&str] = &[];

fn blockchain_runner(path: &Path) -> datatest_stable::Result<()> {
    let skips: Vec<&'static str> = SKIPPED_TESTS
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

#[cfg(not(feature = "stateless"))]
datatest_stable::harness!(blockchain_runner, TEST_FOLDER, r".*");
#[cfg(feature = "stateless")]
datatest_stable::harness!(
    blockchain_runner,
    TEST_FOLDER,
    r".*",
    ef_tests_blockchain::test_runner::check_engine_stateless_bytes,
    ENGINE_TEST_FOLDER,
    r".*",
);
