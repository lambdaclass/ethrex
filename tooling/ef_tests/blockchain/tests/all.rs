use ef_tests_blockchain::test_runner::parse_and_execute;
use ethrex_prover_lib::backend::Backend;
use std::path::Path;

// Enable only one of `sp1` or `stateless` at a time.
#[cfg(all(feature = "sp1", feature = "stateless"))]
compile_error!("Only one of `sp1` and `stateless` can be enabled at a time.");

const TEST_FOLDER: &str = "vectors/";

// Base skips shared by all runs.
const SKIPPED_BASE: &[&str] = &[
    // These tests contain accounts without nonce or code but have storage, which is a virtually impossible scenario. That's why we fail, but that's okay.
    // When creating an account we don't check the storage root but just if it has nonce or code, and that's the right check for real case scenarios.
    "InitCollisionParis",
    "RevertInCreateInInitCreate2Paris",
    "create2collisionStorageParis",
    "dynamicAccountOverwriteEmpty_Paris",
    "RevertInCreateInInit_Paris",
    // Skip because they take too long to run, but they pass
    "static_Call50000_sha256",
    "CALLBlake2f_MaxRounds",
    "loopMul",
    // We don't want to implement the check being tested, it's unnecessary and impossible in known networks. It checks that withdrawal requests and consolidation requests accounts have code, which is always the case, except for this test.
    "system_contract_deployment",
    // Gas price higher than u64::MAX; impractical scenario. We don't use 256 bits for gas price for performance reasons, however, it's debatable. See https://github.com/lambdaclass/ethrex/issues/3629
    "HighGasPriceParis",
    // Skip because it tries to deserialize number > U256::MAX
    "ValueOverflowParis",
    // Skip because it's a "Create" Blob Transaction, which doesn't actually exist. It never reaches the EVM because we can't even parse it as an actual Transaction.
    "createBlobhashTx",
];

// Extra skips added only for prover backends.
#[cfg(feature = "sp1")]
const EXTRA_SKIPS: &[&str] = &[
    "static_Call50000",
    "Return50000",
    "static_Call1MB1024Calldepth",
];
#[cfg(not(feature = "sp1"))]
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

datatest_stable::harness!(blockchain_runner, TEST_FOLDER, r".*");
