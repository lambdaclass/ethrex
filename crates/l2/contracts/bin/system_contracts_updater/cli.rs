use std::path::PathBuf;

use clap::Parser;

#[derive(Parser, Default)]
pub struct SystemContractsUpdaterOptions {
    #[arg(
        long,
        value_name = "PATH",
        env = "ETHREX_DEPLOYER_GENESIS_L1_PATH",
        help_heading = "Deployer options",
        help = "Path to the genesis file. The default is ../../fixtures/genesis/l1-dev.json"
    )]
    pub l2_genesis_path: PathBuf,
}
