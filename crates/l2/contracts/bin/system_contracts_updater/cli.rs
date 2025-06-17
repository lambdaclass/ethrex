use std::path::PathBuf;

use clap::Parser;

#[derive(Parser, Default)]
pub struct SystemContractsUpdaterOptions {
    #[arg(
        long,
        value_name = "PATH",
        env = "ETHREX_SYSTEM_CONTRACTS_UPDATER_CONTRACTS_PATH",
        help_heading = "Deployer options",
        help = "Path to the contracts directory. The default is the current directory."
    )]
    pub contracts_path: PathBuf,
    #[arg(
        long,
        value_name = "PATH",
        env = "ETHREX_DEPLOYER_GENESIS_L1_PATH",
        help_heading = "Deployer options",
        help = "Path to the genesis file. The default is ../../../../../test_data/fixtures/genesis/l1-dev.json"
    )]
    pub l2_genesis_path: PathBuf,
}
