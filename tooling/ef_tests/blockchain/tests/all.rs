use ef_tests_blockchain::test_runner::parse_and_execute;
use ethrex_prover_lib::backend::BackendType;
use std::path::Path;

// Enable only one of `sp1` or `stateless` at a time.
#[cfg(all(feature = "sp1", feature = "stateless"))]
compile_error!("Only one of `sp1` and `stateless` can be enabled at a time.");

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
#[cfg(feature = "sp1")]
const EXTRA_SKIPS: &[&str] = &[
    // I believe these tests fail because of how much stress they put into the zkVM, they probably cause an OOM though this should be checked
    "static_Call50000",
    "Return50000",
    "static_Call1MB1024Calldepth",
];
#[cfg(not(feature = "sp1"))]
const EXTRA_SKIPS: &[&str] = &[];

// Amsterdam fork tests - skipped until Amsterdam EIPs are fully implemented
// See docs/eip.md for Amsterdam EIP implementation status
//
// Since Amsterdam introduces several new EIPs that change gas costs and behavior,
// all tests running on the Amsterdam fork are skipped until the fork is fully supported.
//
// Amsterdam EIPs and their status:
// - EIP-7928: Block-Level Access Lists (SFI) - Not implemented
// - EIP-7708: ETH Transfers Emit a Log (CFI) - Not implemented
// - EIP-7778: Block Gas Accounting without Refunds (CFI) - Not implemented
// - EIP-7843: SLOTNUM Opcode (CFI) - Partially implemented
// - EIP-8024: DUPN/SWAPN/EXCHANGE (CFI) - Implemented
//
// To re-enable Amsterdam tests:
// 1. Implement remaining Amsterdam EIPs (track progress in docs/eip.md)
// 2. Remove "fork_Amsterdam" from this list
// 3. Run: cd tooling/ef_tests/blockchain && make test-levm
// 4. Fix any remaining test failures
// 5. Update docs/eip.md to mark Amsterdam as supported
const SKIPPED_AMSTERDAM: &[&str] = &[
    // Skip all tests running on Amsterdam fork - fork not fully implemented
    // This includes tests from all EIP directories that run on Amsterdam
    "fork_Amsterdam",
    // Skip fork transition tests to Amsterdam
    "ToAmsterdam",
];

// Select backend
#[cfg(feature = "stateless")]
const BACKEND: Option<BackendType> = Some(BackendType::Exec);
#[cfg(feature = "sp1")]
const BACKEND: Option<BackendType> = Some(BackendType::SP1);
#[cfg(not(any(feature = "sp1", feature = "stateless")))]
const BACKEND: Option<BackendType> = None;

fn blockchain_runner(path: &Path) -> datatest_stable::Result<()> {
    // Compose the final skip list
    let skips: Vec<&'static str> = SKIPPED_BASE
        .iter()
        .copied()
        .chain(EXTRA_SKIPS.iter().copied())
        .chain(SKIPPED_AMSTERDAM.iter().copied())
        .collect();

    parse_and_execute(path, Some(&skips), BACKEND)
}

datatest_stable::harness!(blockchain_runner, TEST_FOLDER, r".*");
