use std::path::Path;

use ef_tests_blockchain::{
    network::Network,
    test_runner::{parse_test_file, run_ef_test},
};

// TODO: enable these tests once the evm is updated.
const SKIPPED_TEST: [&str; 2] = [
    "tests/prague/eip6110_deposits/test_deposits.py::test_deposit[fork_Prague-blockchain_test-multiple_deposit_from_same_eoa_last_reverts]",
    "tests/prague/eip6110_deposits/test_deposits.py::test_deposit[fork_Prague-blockchain_test-multiple_deposit_from_same_eoa_first_reverts]",
];

#[allow(dead_code)]
fn parse_and_execute(path: &Path) -> datatest_stable::Result<()> {
    let tests = parse_test_file(path);

    for (test_key, test) in tests {
        if test.network < Network::Merge || SKIPPED_TEST.contains(&test_key.as_str()) {
            // Discard this test
            continue;
        }

        run_ef_test(&test_key, &test);
    }
    Ok(())
}

datatest_stable::harness!(
    parse_and_execute,
    "vectors/prague/eip2935_historical_block_hashes_from_state",
    r".*/.*\.json",
    parse_and_execute,
    "vectors/prague/eip6110_deposits/deposits",
    r".*/.*\.json",
);
