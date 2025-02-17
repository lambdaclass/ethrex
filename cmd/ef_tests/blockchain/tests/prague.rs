use std::path::Path;

use ef_tests_blockchain::{
    network::Network,
    test_runner::{parse_test_file, run_ef_test},
};

// TODO: enable these tests once the evm is updated.
const SKIPPED_TEST: [&str; 19] = [
    "tests/prague/eip6110_deposits/test_deposits.py::test_deposit[fork_Prague-blockchain_test-multiple_deposit_from_same_eoa_last_reverts]",
    "tests/prague/eip6110_deposits/test_deposits.py::test_deposit[fork_Prague-blockchain_test-multiple_deposit_from_same_eoa_first_reverts]",
    "tests/prague/eip7702_set_code_tx/test_set_code_txs.py::test_ext_code_on_chain_delegating_set_code[fork_Prague-blockchain_test]",
    "tests/prague/eip7702_set_code_tx/test_set_code_txs.py::test_ext_code_on_self_delegating_set_code[fork_Prague-blockchain_test-balance_1]",
    "tests/prague/eip7702_set_code_tx/test_set_code_txs.py::test_ext_code_on_self_set_code[fork_Prague-blockchain_test-balance_0]",
    "tests/prague/eip7702_set_code_tx/test_set_code_txs.py::test_ext_code_on_set_code[fork_Prague-blockchain_test-EMPTY_ACCOUNT-balance_0]",
    "tests/prague/eip7702_set_code_tx/test_set_code_txs.py::test_set_code_to_non_empty_storage[fork_Prague-blockchain_test-zero_nonce]",
    "tests/prague/eip7702_set_code_tx/test_set_code_txs_2.py::test_call_to_precompile_in_pointer_context[fork_Prague-precompile_0x000000000000000000000000000000000000000b-blockchain_test]",
    "tests/prague/eip7702_set_code_tx/test_set_code_txs_2.py::test_pointer_measurements[fork_Prague-blockchain_test]",
    "tests/prague/eip7702_set_code_tx/test_set_code_txs.py::test_ext_code_on_self_delegating_set_code[fork_Prague-blockchain_test-balance_0]",
    "tests/prague/eip7702_set_code_tx/test_set_code_txs.py::test_ext_code_on_self_set_code[fork_Prague-blockchain_test-balance_1]",
    "tests/prague/eip7702_set_code_tx/test_set_code_txs.py::test_ext_code_on_set_code[fork_Prague-blockchain_test-EOA_WITH_SET_CODE-balance_0]",
    "tests/prague/eip7702_set_code_tx/test_set_code_txs.py::test_valid_tx_invalid_chain_id[fork_Prague-blockchain_test-auth_chain_id=2**256-1]",
    "tests/prague/eip7702_set_code_tx/test_set_code_txs.py::test_ext_code_on_set_code[fork_Prague-blockchain_test-CONTRACT-balance_0]",
    "tests/prague/eip7702_set_code_tx/test_set_code_txs.py::test_ext_code_on_set_code[fork_Prague-blockchain_test-EOA_WITH_SET_CODE-balance_1]",
    "tests/prague/eip7702_set_code_tx/test_set_code_txs.py::test_ext_code_on_set_code[fork_Prague-blockchain_test-EMPTY_ACCOUNT-balance_1]",
    "tests/prague/eip7702_set_code_tx/test_set_code_txs.py::test_ext_code_on_set_code[fork_Prague-blockchain_test-EOA-balance_1]",
    "tests/prague/eip7702_set_code_tx/test_set_code_txs.py::test_ext_code_on_set_code[fork_Prague-blockchain_test-CONTRACT-balance_1]",
    "tests/prague/eip7702_set_code_tx/test_set_code_txs.py::test_ext_code_on_set_code[fork_Prague-blockchain_test-EOA-balance_0]"];

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
    "vectors/prague/eip7702_set_code_tx",
    r".*/.*\.json",
    parse_and_execute,
    "vectors/prague/eip6110_deposits/deposits",
    r".*/.*\.json",
);
