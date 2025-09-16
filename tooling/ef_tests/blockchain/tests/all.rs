use ef_tests_blockchain::test_runner::parse_and_execute;
use std::path::Path;

const TEST_FOLDER: &str = "vectors/";

#[cfg(not(any(feature = "revm", feature = "sp1", feature = "stateless")))]
const SKIPPED_TESTS: &[&str] = &[
    "system_contract_deployment", // Skipped because we don't have that validation in our code; also skipped because other clients do so too
    "test_excess_blob_gas_fork_transition", // Skipped because genesis has Cancun fields when it shouldn't, so genesis loading fails
    "test_invalid_post_fork_block_without_blob_fields", // Skipped because genesis has Cancun fields when it shouldn't, so genesis loading fails
    "test_invalid_pre_fork_block_with_blob_fields", // Skipped because genesis has Cancun fields when it shouldn't, so genesis loading fails
    "stTransactionTest/HighGasPriceParis",          // Skipped because it tries to
    "dynamicAccountOverwriteEmpty_Paris", // Skipped because it fails on REVM as well. See https://github.com/lambdaclass/ethrex/issues/1555 for more details
    "create2collisionStorageParis", // Skipped because it fails on REVM as well. See https://github.com/lambdaclass/ethrex/issues/1555 for more details
    "RevertInCreateInInitCreate2Paris", // Skipped because it fails on REVM as well. See https://github.com/lambdaclass/ethrex/issues/1555 for more details
    "createBlobhashTx", // Skipped because it fails and is only part of development fixtures
];
#[cfg(feature = "revm")]
const SKIPPED_TESTS: &[&str] = &[
    "system_contract_deployment",
    "fork_Osaka",
    "fork_PragueToOsaka",
    "fork_BPO0",
    "fork_BPO1",
    "fork_BPO2",
    "test_excess_blob_gas_fork_transition",
    "test_invalid_post_fork_block_without_blob_fields",
    "test_invalid_pre_fork_block_with_blob_fields",
    "stTransactionTest/HighGasPriceParis",
    "dynamicAccountOverwriteEmpty_Paris",
    "create2collisionStorageParis",
    "RevertInCreateInInitCreate2Paris",
    "createBlobhashTx",
    "test_reserve_price_at_transition",
    "CreateTransactionHighNonce",
    "lowGasLimit",
];
#[cfg(any(feature = "sp1", feature = "stateless"))]
const SKIPPED_TESTS: &[&str] = &[
    "system_contract_deployment",
    "test_excess_blob_gas_fork_transition",
    "test_invalid_post_fork_block_without_blob_fields",
    "test_invalid_pre_fork_block_with_blob_fields",
    "stTransactionTest/HighGasPriceParis",
    "dynamicAccountOverwriteEmpty_Paris",
    "create2collisionStorageParis",
    "RevertInCreateInInitCreate2Paris",
    "createBlobhashTx",
    // We skip these two tests because they fail with stateless backend specifically. See https://github.com/lambdaclass/ethrex/issues/4502
    "test_large_amount",
    "test_multiple_withdrawals_same_address",
];

// If neither `sp1` nor `stateless` is enabled: run with whichever engine
// the features imply (LEVM if `levm` is on; otherwise REVM).
#[cfg(not(any(feature = "sp1", feature = "stateless")))]
fn blockchain_runner(path: &Path) -> datatest_stable::Result<()> {
    parse_and_execute(path, Some(SKIPPED_TESTS), None)
}

// If `sp1` or `stateless` is enabled: always use LEVM with the appropriate backend.
#[cfg(any(feature = "sp1", feature = "stateless"))]
fn blockchain_runner(path: &Path) -> datatest_stable::Result<()> {
    #[cfg(feature = "stateless")]
    let backend = Some(ethrex_prover_lib::backend::Backend::Exec);
    #[cfg(feature = "sp1")]
    let backend = Some(ethrex_prover_lib::backend::Backend::SP1);

    parse_and_execute(path, Some(SKIPPED_TESTS), backend)
}

datatest_stable::harness!(blockchain_runner, TEST_FOLDER, r".*");

#[cfg(any(
    all(feature = "sp1", feature = "stateless"),
    all(feature = "sp1", feature = "revm"),
    all(feature = "stateless", feature = "revm"),
))]
compile_error!("Only one of `sp1`, `stateless`, or `revm` can be enabled at a time.");
