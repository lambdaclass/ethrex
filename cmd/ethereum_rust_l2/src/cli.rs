use crate::{
    commands::{autocomplete, config, stack, test, utils, wallet},
    config::load_selected_config,
};
use clap::{Parser, Subcommand};

pub const VERSION_STRING: &str = env!("CARGO_PKG_VERSION");

#[derive(Parser)]
#[command(name="ethereum_rust_l2_cli", author, version=VERSION_STRING, about, long_about = None)]
pub struct EthereumRustL2CLI {
    #[command(subcommand)]
    command: EthereumRustL2Command,
}

#[derive(Subcommand)]
enum EthereumRustL2Command {
    #[clap(subcommand, about = "Stack related commands.")]
    Stack(stack::Command),
    #[clap(
        subcommand,
        about = "Wallet interaction commands. The configured wallet could operate both with the L1 and L2 networks.",
        visible_alias = "w"
    )]
    Wallet(wallet::Command),
    #[clap(
        subcommand,
        about = "Different utilities for developers.",
        visible_alias = "u"
    )]
    Utils(utils::Command),
    #[clap(subcommand, about = "CLI config commands.")]
    Config(config::Command),
    #[clap(subcommand, about = "Run tests.")]
    Test(test::Command),
    #[clap(subcommand, about = "Generate shell completion scripts.")]
    Autocomplete(autocomplete::Command),
}

pub async fn start() -> eyre::Result<()> {
    let EthereumRustL2CLI { command } = EthereumRustL2CLI::parse();
    if let EthereumRustL2Command::Config(cmd) = command {
        return cmd.run().await;
    }
    let cfg = load_selected_config().await?;
    match command {
        EthereumRustL2Command::Stack(cmd) => cmd.run(cfg).await?,
        EthereumRustL2Command::Wallet(cmd) => cmd.run(cfg).await?,
        EthereumRustL2Command::Utils(cmd) => cmd.run().await?,
        EthereumRustL2Command::Autocomplete(cmd) => cmd.run()?,
        EthereumRustL2Command::Config(_) => unreachable!(),
        EthereumRustL2Command::Test(cmd) => cmd.run(cfg).await?,
    };
    Ok(())
}
