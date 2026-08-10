use ef_tests_blockchain::test_runner::parse_and_execute;
use std::path::Path;

// test-levm reads snobal-devnet-6 + legacy from `vectors/`; test-stateless reads
// the generated #3248+#3278 conformance vectors from a separate root so the two
// do not overlay each other. See the Makefile for how each is populated.
//
// Only the `blockchain_tests/` subtree: the generator also emits a sibling
// `blockchain_tests_engine/` in the Engine-API fixture format and a `.meta/`
// directory, neither of which this harness can parse.
//
// It must be the generated set, not the downloaded `vectors_zkevm/` one. That
// bundle is tests-zkevm@v0.6.2, which predates execution-specs #3278 while
// reusing schema id 0x1501, so its `statelessOutputBytes` are 69 bytes against
// the 43 this client now produces — every wire-path comparison would fail.
#[cfg(feature = "stateless")]
const TEST_FOLDER: &str = "vectors_stateless_3278/blockchain_tests/";
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

// Fixtures that fail in ORDINARY block execution, before any stateless
// validation runs — `add_block_pipeline` rejects them with `GasUsedMismatch` or
// `ReceiptsRootMismatch`. Verified by running this same vector set without the
// `stateless` feature: all five fail identically, so they are not stateless
// conformance gaps. They are only reachable here because the vectors are filled
// at execution-specs master `3c3b6f4af`, ahead of the glamsterdam-devnet v7.2.0
// base this client targets. See docs/known_issues.md.
#[cfg(feature = "stateless")]
const EXTRA_SKIPS: &[&str] = &[
    "test_witness_codes_auth_nonce_mismatch",
    "test_witness_codes_redelegation_old_marker_included_new_marker_excluded",
    "test_witness_codes_reset_delegation",
    "test_witness_codes_failed_create_after_initcode_read",
    "test_validation_codes_missing_redelegation_old_marker",
];
#[cfg(not(feature = "stateless"))]
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
