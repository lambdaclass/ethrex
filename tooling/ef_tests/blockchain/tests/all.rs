use ef_tests_blockchain::test_runner::parse_and_execute;
use ethrex_prover::backend::BackendType;
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
#[cfg(feature = "stateless")]
const EXTRA_SKIPS: &[&str] = &[
    // zkevm@v0.3.3 tolerance tests: the fixture's `statelessOutputBytes` declares `valid = 1`
    // because the executed path does not actually consume the malformed/extra/missing witness
    // entry, but our RpcExecutionWitness conversion eagerly validates the full witness and
    // rejects it. Re-enable once the witness conversion is lazy per EIP-8025 §Tolerance.
    "validation_headers_malformed_rlp_header",
    "validation_headers_missing_oldest_blockhash_ancestor",
    "validation_headers_missing_parent_header",
    "validation_state_extra_unused_trie_node",
    // zkevm@v0.3.3 rejection tests: `statelessOutputBytes` declares `valid = 0` so the guest
    // program must reject the deliberately-incomplete witness, but our stateless path runs
    // to completion instead of detecting the missing entry. Re-enable once the witness
    // completeness checks land (missing delegation/external-code bytecodes, non-contiguous
    // header chain detection).
    "validation_codes_missing_delegated_code_on_insufficient_balance_call",
    "validation_codes_missing_external_code_read_target",
    "validation_codes_missing_redelegation_old_marker",
    "validation_codes_missing_sender_delegation_marker",
    "validation_headers_non_contiguous_chain",
];
#[cfg(not(any(feature = "sp1", feature = "stateless")))]
const EXTRA_SKIPS: &[&str] = &[];

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
        .collect();

    parse_and_execute(path, Some(&skips), BACKEND)
}

datatest_stable::harness!(blockchain_runner, TEST_FOLDER, r".*");
