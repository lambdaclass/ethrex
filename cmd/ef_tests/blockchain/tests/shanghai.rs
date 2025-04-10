use ef_tests_blockchain::test_runner::parse_and_execute;
use ethrex_vm::EvmEngine;
use std::path::Path;

fn parse_and_execute_with_revm(path: &Path) -> datatest_stable::Result<()> {
    parse_and_execute(path, EvmEngine::REVM);
    Ok(())
}

fn parse_and_execute_with_levm(path: &Path) -> datatest_stable::Result<()> {
    parse_and_execute(path, EvmEngine::LEVM);
    Ok(())
}

datatest_stable::harness!(
    // REVM execution
    parse_and_execute_with_revm,
    "vectors/shanghai/",
    r".*/.*/.*\.json",
    // LEVM execution
    parse_and_execute_with_levm,
    "vectors/shanghai/",
    r"eip3651_warm_coinbase/.*/.*\.json",
    parse_and_execute_with_levm,
    "vectors/shanghai/",
    r"eip3855_push0/.*/.*\.json",
    // parse_and_execute_with_levm,
    // "vectors/shanghai/",
    // r"eip3860_initcode/.*/.*\.json",
    // parse_and_execute_with_levm,
    // "vectors/shanghai/",
    // r"eip4895_withdrawals/.*/.*\.json",
);
