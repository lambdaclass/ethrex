use crate::{
    commands::{autocomplete, config, prove, stack, test, utils, wallet},
    config::load_selected_config,
};
use clap::{Parser, Subcommand};

pub const VERSION_STRING: &str = env!("CARGO_PKG_VERSION");

#[derive(Parser)]
#[command(name="Ethrex_l2_cli", author, version=VERSION_STRING, about, long_about = None)]
pub struct EthrexL2CLI {
    #[command(subcommand)]
    command: EthrexL2Command,
}

#[derive(Subcommand)]
enum EthrexL2Command {
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
    #[clap(about = "Read a test chain from disk and prove a block.")]
    Prove(prove::Command),
}

pub async fn start() -> eyre::Result<()> {
    let EthrexL2CLI { command } = EthrexL2CLI::parse();
    if let EthrexL2Command::Config(cmd) = command {
        return cmd.run().await;
    }
    if let EthrexL2Command::Prove(cmd) = command {
        return cmd.run();
    }

    let cfg = load_selected_config().await?;
    match command {
        EthrexL2Command::Stack(cmd) => cmd.run(cfg).await?,
        EthrexL2Command::Wallet(cmd) => cmd.run(cfg).await?,
        EthrexL2Command::Utils(cmd) => cmd.run().await?,
        EthrexL2Command::Autocomplete(cmd) => cmd.run()?,
        EthrexL2Command::Config(_) => unreachable!(),
        EthrexL2Command::Test(cmd) => cmd.run(cfg).await?,
        EthrexL2Command::Prove(_) => unreachable!(),
    };
    Ok(())
}
