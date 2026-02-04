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

// Amsterdam EIP tests - skipped until each EIP is implemented
// See docs/eip.md for Amsterdam EIP implementation status
//
// STRUCTURE:
// - Section 1: Amsterdam-specific EIP directory skips (can be individually enabled)
// - Section 2: Legacy tests running on Amsterdam fork (skip until ALL Amsterdam EIPs done)
//
// HOW TO TEST AN INDIVIDUAL EIP (e.g., EIP-7708):
// 1. Comment out BOTH:
//    - The EIP's skip pattern (e.g., "eip7708_eth_transfer_logs")
//    - The "fork_Amsterdam" pattern in Section 2
// 2. Run with cargo test filter to only run that EIP's tests:
//      cargo test eip7708 --profile release-with-debug
// 3. Fix any test failures in your EIP implementation
// 4. IMPORTANT: Restore both skip patterns before committing
// 5. Update docs/eip.md to track progress
//
// WHY TWO SKIPS ARE NEEDED:
// - EIP patterns skip by test directory (e.g., "eip7708_eth_transfer_logs")
// - "fork_Amsterdam" skips by fork parameter in test name
// - Tests have BOTH in their full name, so both must be commented to run
//
// HOW TO FULLY ENABLE AMSTERDAM:
// 1. Implement ALL Amsterdam EIPs
// 2. Remove/comment ALL entries in this list (both sections)
// 3. Run: make test-levm
// 4. Fix any remaining failures
const SKIPPED_AMSTERDAM: &[&str] = &[
    // =========================================================================
    // SECTION 1: Amsterdam-specific EIP tests
    // Comment out individual EIPs to test them (use cargo test filter)
    // =========================================================================
    //
    // EIP-7928: Block-Level Access Lists (SFI) - NOT IMPLEMENTED
    // Requires block-level state access tracking that changes gas costs
    // ~250 tests | To test: cargo test eip7928 --profile release-with-debug
    "eip7928_block_level_access_lists",
    //
    // EIP-7708: ETH Transfers Emit a Log (CFI) - NOT IMPLEMENTED
    // Requires LOG emission on ETH value transfers
    // ~66 tests | To test: cargo test eip7708 --profile release-with-debug
    "eip7708_eth_transfer_logs",
    //
    // EIP-7778: Block Gas Accounting without Refunds (CFI) - NOT IMPLEMENTED
    // Requires changes to gas refund calculations at block level
    // ~24 tests | To test: cargo test eip7778 --profile release-with-debug
    "eip7778_block_gas_accounting_without_refunds",
    //
    // EIP-7843: SLOTNUM Opcode (CFI) - PARTIALLY IMPLEMENTED
    // New opcode returning current slot number
    // ~7 tests | To test: cargo test eip7843 --profile release-with-debug
    "eip7843_slotnum",
    //
    // EIP-8024: DUPN/SWAPN/EXCHANGE (CFI) - IMPLEMENTED
    // New stack manipulation opcodes
    // Tests fail due to gas cost changes from other Amsterdam EIPs (EIP-7928, EIP-7778)
    // ~400 tests | To test: cargo test eip8024 --profile release-with-debug
    "eip8024_dupn_swapn_exchange",
    //
    // =========================================================================
    // SECTION 2: Legacy tests running on Amsterdam fork
    // These tests from older EIP directories run on Amsterdam and fail because
    // Amsterdam changes gas costs (EIP-7928, EIP-7778). Keep skipped until ALL
    // Amsterdam EIPs are implemented.
    // ~31,000 tests across berlin, byzantium, cancun, prague, etc.
    // =========================================================================
    "fork_Amsterdam",
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
    // Set UNFILTERED_AMSTERDAM=1 to run Amsterdam tests without skips (for CI reporting)
    let skip_amsterdam = std::env::var("UNFILTERED_AMSTERDAM").is_err();

    let skips: Vec<&'static str> = SKIPPED_BASE
        .iter()
        .copied()
        .chain(EXTRA_SKIPS.iter().copied())
        .chain(if skip_amsterdam {
            SKIPPED_AMSTERDAM.to_vec()
        } else {
            vec![]
        })
        .collect();

    parse_and_execute(path, Some(&skips), BACKEND)
}

datatest_stable::harness!(blockchain_runner, TEST_FOLDER, r".*");
