use std::path::Path;

use ef_tests_blockchain::{
    network::Network,
    test_runner::{parse_test_file, run_ef_test},
};

// TODO: enable these tests once the evm is updated.
const SKIPPED_TEST: [&str; 2] = [
    "tests/prague/eip7702_set_code_tx/test_set_code_txs.py::test_contract_create[fork_Prague-blockchain_test]",
    "tests/prague/eip7702_set_code_tx/test_set_code_txs.py::test_set_code_to_non_empty_storage[fork_Prague-blockchain_test-zero_nonce]",
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
    "vectors/prague/eip2537_bls_12_381_precompiles",
    r".*/.*\.json",
    parse_and_execute,
    "vectors/prague/eip2935_historical_block_hashes_from_state",
    r".*/.*\.json",
    parse_and_execute,
    "vectors/prague/eip6110_deposits/deposits",
    r".*/.*\.json",
    parse_and_execute,
    "vectors/prague/eip7002_el_triggerable_withdrawals",
    r".*/.*\.json",
    parse_and_execute,
    "vectors/prague/eip7251_consolidations",
    r".*/.*\.json",
    parse_and_execute,
    "vectors/prague/eip7623_increase_calldata_cost",
    r".*/.*\.json",
    parse_and_execute,
    "vectors/prague/eip7685_general_purpose_el_requests",
    r".*/.*\.json",
    parse_and_execute,
    "vectors/prague/eip7702_set_code_tx",
    r".*/.*\.json",
);
