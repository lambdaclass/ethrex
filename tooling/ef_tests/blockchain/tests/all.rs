use ef_tests_blockchain::test_runner::parse_and_execute;
use ethrex_prover::backend::BackendType;
use std::path::Path;

// Enable only one of `sp1` or `stateless` at a time.
#[cfg(all(feature = "sp1", feature = "stateless"))]
compile_error!("Only one of `sp1` and `stateless` can be enabled at a time.");

// test-levm / test-sp1 read snobal-devnet-6 + legacy from `vectors/`.
// test-stateless reads zkevm@v0.5.0 (EIP-8025 canonical bundle) from a separate
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
// The stateless run executes the zkevm@v0.5.0 bundle (`vectors_zkevm/`). Unlike the earlier
// v0.4.1 release (filled against an older glamsterdam devnet, which lagged this client's v6.1.0
// gas accounting and failed ~2790/2864 with stale gas), v0.5.0 is filled against
// `tests-glamsterdam-devnet@v6.1.0` — the same base as the live `vectors/` fixtures — so the
// whole bundle re-executes cleanly and no blanket skip is needed. Per-fixture leniency cases
// (`*_extra_unused_*` padding, deliberately-invalid witnesses) are handled in `test_runner.rs`.
// TEMPORARY: the stateless zkevm bundle is still `tests-zkevm@v0.5.0` (filled
// against glamsterdam-devnet v6.1.0), which predeploys the EIP-8282 builder
// deposit/exit contracts at the OLD addresses (`…d9008282` / `…0f008282`). This
// client now uses the devnet-7 addresses (`…300d8282` / `…800e8282`, matching
// the live `vectors/` v7.2.0 bundle), so every Amsterdam block's end-of-block
// builder system call finds no code at the new addresses and fails the block.
// Since the whole zkevm bundle is `for_amsterdam`, skip it wholesale until a
// zkevm bundle filled with the new predeploy addresses is released, then remove
// this skip (mirrors #6740's "unskip stateless validation tests").
#[cfg(feature = "stateless")]
const EXTRA_SKIPS: &[&str] = &["for_amsterdam"];
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
