use ef_tests_blockchain::test_runner::parse_and_execute;
use std::path::Path;

const TEST_FOLDER: &str = "vectors/";

#[cfg(not(feature = "revm"))]
const SKIPPED_TESTS: &[&str] = &[
    "system_contract_deployment",
    "test_tx_gas_larger_than_block_gas_limit[fork_Osaka-blockchain_test-exceed_block_gas_limit_True]",
];
// We are skipping test_tx_gas_larger_than_block_gas_limit[fork_Osaka-blockchain_test-exceed_block_gas_limit_True] because of an
// inconsistency on the expected exception. Exception returned is InvalidBlock(GasUsedMismatch(0x06000000,0x05000000)) while
// exception expected GAS_ALLOWANCE_EXCEEDED. The thing is gas allowance exception is supposed to be thrown on any transaction
// execution in case the transaction's gas limit is larger than the block's, which is not the case of this test.
// This test has a block with "gasLimit": "0x055d4a80", "gasUsed": "0x05000000" and six transactions with "gasLimit": "0x01000000",
// Apparently each transaction consumes up to its gas limit, which together is larger than the block's. Then when executing validate_gas_used
// after the block's execution, it throws InvalidBlock(GasUsedMismatch(0x06000000,0x05000000)) on comparing the receipt's cumulative gas used agains the block's gas limit.
#[cfg(feature = "revm")]
const SKIPPED_TESTS: &[&str] = &[
    "system_contract_deployment",
    "fork_Osaka",
    "fork_PragueToOsaka",
    "fork_BPO0",
    "fork_BPO1",
    "fork_BPO2",
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
