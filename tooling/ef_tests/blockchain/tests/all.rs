use ef_tests_blockchain::test_runner::parse_and_execute;
use std::path::Path;

const TEST_FOLDER: &str = "vectors/";

#[cfg(not(any(feature = "sp1", feature = "stateless")))]
const SKIPPED_TESTS: &[&str] = &[
    "system_contract_deployment",
    "stTransactionTest/HighGasPriceParis", // Skipped because it sets a gas price higher than u64::MAX, which most clients don't implement and is a virtually impossible scenario
    "dynamicAccountOverwriteEmpty_Paris", // Skipped because the scenario described is virtually impossible
    "create2collisionStorageParis", // Skipped because it's not worth implementing since the scenario of the test is virtually impossible. See https://github.com/lambdaclass/ethrex/issues/1555
    "RevertInCreateInInitCreate2Paris", // Skipped because it's not worth implementing since the scenario of the test is virtually impossible. See https://github.com/lambdaclass/ethrex/issues/1555
    "test_tx_gas_larger_than_block_gas_limit",
];
// We are skipping test_tx_gas_larger_than_block_gas_limit[fork_Osaka-blockchain_test-exceed_block_gas_limit_True] because of an
// inconsistency on the expected exception. Exception returned is InvalidBlock(GasUsedMismatch(0x06000000,0x05000000)) while
// exception expected GAS_ALLOWANCE_EXCEEDED. The thing is gas allowance exception is supposed to be thrown on any transaction
// execution in case the transaction's gas limit is larger than the block's, which is not the case of this test.
// This test has a block with "gasLimit": "0x055d4a80", "gasUsed": "0x05000000" and six transactions with "gasLimit": "0x01000000",
// Apparently each transaction consumes up to its gas limit, which together is larger than the block's. Then when executing validate_gas_used
// after the block's execution, it throws InvalidBlock(GasUsedMismatch(0x06000000,0x05000000)) on comparing the receipt's cumulative gas used agains the block's gas limit.
#[cfg(any(feature = "sp1", feature = "stateless"))]
const SKIPPED_TESTS: &[&str] = &[
    // We skip most of these for the same reason we skip them in LEVM; since we need to do a LEVM run before doing one with the stateless backend
    "system_contract_deployment",
    "test_tx_gas_larger_than_block_gas_limit",
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

#[cfg(any(all(feature = "sp1", feature = "stateless"),))]
compile_error!("Only one of `sp1`, `stateless` can be enabled at a time.");
