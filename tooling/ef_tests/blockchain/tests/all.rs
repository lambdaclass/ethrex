use ef_tests_blockchain::test_runner::parse_and_execute;
use std::path::Path;

const TEST_FOLDER: &str = "vectors/";

#[cfg(not(any(feature = "revm", feature = "sp1", feature = "stateless")))]
const SKIPPED_TESTS: &[&str] = &[
    "system_contract_deployment", // Skipped because we don't have that validation in our code; also skipped because other clients do so too
    "stTransactionTest/HighGasPriceParis", // Skipped because it sets a gas price higher than u64::MAX, which most clients don't implement and is a virtually impossible scenario
    "dynamicAccountOverwriteEmpty_Paris", // Skipped because the scenario described is virtually impossible
    "create2collisionStorageParis", // Skipped because it's not worth implementing since the scenario of the test is virtually impossible. See https://github.com/lambdaclass/ethrex/issues/1555
    "RevertInCreateInInitCreate2Paris", // Skipped because it's not worth implementing since the scenario of the test is virtually impossible. See https://github.com/lambdaclass/ethrex/issues/1555
];
#[cfg(feature = "revm")]
const SKIPPED_TESTS: &[&str] = &[
    "system_contract_deployment",
    // We skip these tests because the version of REVM we're using doesn't support Osaka
    "fork_Osaka",
    "fork_PragueToOsaka",
    "fork_BPO0",
    "fork_BPO1",
    "fork_BPO2",
    "test_reserve_price_at_transition",
    "CreateTransactionHighNonce",
    "lowGasLimit",
    // We skip these because they fail in REVM
    "stTransactionTest/HighGasPriceParis",
    "create2collisionStorageParis",
    "dynamicAccountOverwriteEmpty_Paris",
    "RevertInCreateInInitCreate2Paris",
];
#[cfg(any(feature = "sp1", feature = "stateless"))]
const SKIPPED_TESTS: &[&str] = &[
    // We skip most of these for the same reason we skip them in LEVM; since we need to do a LEVM run before doing one with the stateless backend
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
