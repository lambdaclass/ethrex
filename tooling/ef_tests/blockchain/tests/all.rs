use ef_tests_blockchain::test_runner::parse_and_execute;
use ethrex_prover_lib::backend::Backend;
use std::path::Path;

// Enable only one of `sp1` or `stateless` at a time.
#[cfg(all(feature = "sp1", feature = "stateless"))]
compile_error!("Only one of `sp1` and `stateless` can be enabled at a time.");

const TEST_FOLDER: &str = "vectors/";

// Base skips shared by all runs.
const SKIPPED_BASE: &[&str] = &[
    "system_contract_deployment", // We don't want to implement the check being tested, it's unnecessary and impossible in known networks. It checks that withdrawal requests and consolidation requests accounts have code, which is always the case.
    "HighGasPriceParis", // Gas price higher than u64::MAX; impractical scenario. We don't use 256 bits for gas price for performance reasons, however, it's debatable. See https://github.com/lambdaclass/ethrex/issues/3629
    "dynamicAccountOverwriteEmpty_Paris", // Scenario is virtually impossible.
    "create2collisionStorageParis", // Scenario is virtually impossible. See https://github.com/lambdaclass/ethrex/issues/1555
    "RevertInCreateInInitCreate2Paris", // Scenario is virtually impossible. See https://github.com/lambdaclass/ethrex/issues/1555
    "test_tx_gas_larger_than_block_gas_limit", // Expected exception mismatch (GasUsedMismatch vs GAS_ALLOWANCE_EXCEEDED).
    "createBlobhashTx",
    "RevertInCreateInInit_Paris",
    "InitCollisionParis",
    "ValueOverflowParis",
];

// Extra skips added only for prover backends.
#[cfg(any(feature = "sp1", feature = "stateless"))]
const EXTRA_SKIPS: &[&str] = &[
    "test_large_amount",
    "test_multiple_withdrawals_same_address",
];
#[cfg(not(any(feature = "sp1", feature = "stateless")))]
const EXTRA_SKIPS: &[&str] = &[];

// Select backend
#[cfg(feature = "stateless")]
const BACKEND: Option<Backend> = Some(Backend::Exec);
#[cfg(feature = "sp1")]
const BACKEND: Option<Backend> = Some(Backend::SP1);
#[cfg(not(any(feature = "sp1", feature = "stateless")))]
const BACKEND: Option<Backend> = None;

fn blockchain_runner(path: &Path) -> datatest_stable::Result<()> {
    // Compose the final skip list
    let skips: Vec<&'static str> = SKIPPED_BASE
        .iter()
        .copied()
        .chain(EXTRA_SKIPS.iter().copied())
        .collect();

    parse_and_execute(path, Some(&skips), BACKEND)
}

datatest_stable::harness!(blockchain_runner, TEST_FOLDER, r"");
