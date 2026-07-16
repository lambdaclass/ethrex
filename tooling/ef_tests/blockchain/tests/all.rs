use ef_tests_blockchain::test_runner::parse_and_execute;
use ethrex_prover::backend::BackendType;
use std::path::Path;

// Enable only one of `sp1` or `stateless` at a time.
#[cfg(all(feature = "sp1", feature = "stateless"))]
compile_error!("Only one of `sp1` and `stateless` can be enabled at a time.");

// test-levm / test-sp1 read snobal-devnet-6 + legacy from `vectors/`.
// test-stateless reads zkevm@v0.6.1 (EIP-8025 canonical bundle) from a separate
// `vectors_zkevm/` so the bundles don't overlay each other.
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
#[cfg(all(feature = "sp1", not(feature = "stateless")))]
const EXTRA_SKIPS: &[&str] = &[
    // I believe these tests fail because of how much stress they put into the zkVM, they probably cause an OOM though this should be checked
    "static_Call50000",
    "Return50000",
    "static_Call1MB1024Calldepth",
];
// The stateless run executes the zkevm@v0.6.1 bundle (`vectors_zkevm/`), filled against
// `tests-glamsterdam-devnet@v7.2.0` — the same base as the live `vectors/` fixtures on this
// branch — and carrying the devnet-7 EIP-8282 builder predeploy addresses plus the updated
// canonical stateless-input format (schema id, `SszForkConfig`). The whole bundle re-executes
// cleanly, so no blanket skip and no per-fork skip are needed. Per-fixture leniency cases
// (`*_extra_unused_*` padding, deliberately-invalid witnesses) are handled in `test_runner.rs`.
// Amsterdam+ fixtures are skipped in the stateless run by fork (see
// `parse_and_execute` in `test_runner.rs` and docs/known_issues.md): the
// tests-zkevm@v0.5.0 bundle predeploys the EIP-8282 builder contracts at the OLD
// addresses, incompatible with this client's devnet-7 addresses. That skip is
// fork-based (not name-based), so no per-test entries are needed here.
#[cfg(feature = "stateless")]
const EXTRA_SKIPS: &[&str] = &[];
#[cfg(not(any(feature = "sp1", feature = "stateless")))]
const EXTRA_SKIPS: &[&str] = &[];

// Select backend
#[cfg(feature = "stateless")]
const BACKEND: Option<BackendType> = Some(BackendType::Exec);
#[cfg(all(feature = "sp1", not(feature = "stateless")))]
const BACKEND: Option<BackendType> = Some(BackendType::SP1);
#[cfg(not(any(feature = "sp1", feature = "stateless")))]
const BACKEND: Option<BackendType> = None;

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
