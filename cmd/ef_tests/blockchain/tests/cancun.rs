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
    "vectors/cancun/",
    r".*/.*\.json",
    // LEVM execution
    parse_and_execute_with_levm,
    "vectors/cancun/",
    r"eip1153_tstore/.*/.*\.json",
    // parse_and_execute_with_levm,
    // "vectors/cancun/",
    // r"eip4788_beacon_root/.*/.*\.json",
    // parse_and_execute_with_levm,
    // "vectors/cancun/",
    // r"eip4844_blobs/.*/.*\.json",
    parse_and_execute_with_levm,
    "vectors/cancun/",
    r"eip5656_mcopy/.*/.*\.json",
    parse_and_execute_with_levm,
    "vectors/cancun/",
    r"eip6780_selfdestruct/.*/.*\.json",
    parse_and_execute_with_levm,
    "vectors/cancun/",
    r"eip7516_blobgasfee/.*/.*\.json",
);
