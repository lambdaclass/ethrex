use std::path::Path;

use ef_tests_blockchain::{
    network::Network,
    test_runner::{parse_test_file, run_ef_test},
};

#[allow(dead_code)]
fn parse_and_execute(path: &Path) -> datatest_stable::Result<()> {
    let tests = parse_test_file(path);

    for (test_key, test) in tests {
        if test.network < Network::Merge {
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
    // parse_and_execute,
    // "vectors/prague/eip6110_deposits/deposits",
    // r"test.json"
);
