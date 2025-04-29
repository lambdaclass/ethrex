use std::path::PathBuf;

use clap::Parser;

#[derive(Parser, Default)]
pub struct SystemContractsUpdaterOptions {
    #[arg(
        long,
        default_value = ".",
        value_name = "PATH",
        env = "ETHREX_SYSTEM_CONTRACTS_UPDATER_CONTRACTS_PATH",
        required = false,
        help_heading = "Deployer options",
        help = "Path to the contracts directory. The default is the current directory."
    )]
    pub contracts_path: PathBuf,
    #[arg(
        long,
        default_value = "../../test_data/genesis-l1-dev.json",
        value_name = "PATH",
        env = "ETHREX_DEPLOYER_GENESIS_L1_PATH",
        help_heading = "Deployer options",
        help = "Path to the genesis file. The default is ../../test_data/genesis-l1-dev.json"
    )]
    pub genesis_l1_path: String,
}
