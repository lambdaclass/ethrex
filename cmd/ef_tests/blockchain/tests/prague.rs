use std::path::Path;

use ef_tests_blockchain::{
    network::Network,
    test_runner::{parse_test_file, run_ef_test},
};

// TODO: enable these tests once the evm is updated.
const SKIPPED_TEST: [&str; 1] = [
    "tests/prague/eip7702_set_code_tx/test_set_code_txs.py::test_set_code_to_non_empty_storage[fork_Prague-blockchain_test-zero_nonce]",
];

fn parse_and_execute(path: &Path) -> datatest_stable::Result<()> {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let tests = parse_test_file(path);

    for (test_key, test) in tests {
        if test.network < Network::Merge || SKIPPED_TEST.contains(&test_key.as_str()) {
            // Discard this test
            continue;
        }

        rt.block_on(run_ef_test(&test_key, &test));
    }
    Ok(())
}

datatest_stable::harness!(parse_and_execute, "vectors/prague/", r".*/.*\.json",);
