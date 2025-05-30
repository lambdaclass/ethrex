use crate::{
    commands::{autocomplete, config, info, stack, utils, wallet},
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
    #[command(subcommand, about = "Stack related commands.")]
    Stack(stack::Command),
    #[command(
        subcommand,
        about = "Wallet interaction commands. The configured wallet could operate both with the L1 and L2 networks.",
        visible_alias = "w"
    )]
    Wallet(wallet::Command),
    #[command(subcommand, about = "CLI config commands.")]
    Config(config::Command),
    #[command(subcommand, about = "Generate shell completion scripts.")]
    Autocomplete(autocomplete::Command),
    #[command(subcommand, about = "Gets L2's information.")]
    Info(info::Command),
    #[command(subcommand, about = "Utils commands.")]
    Utils(utils::Command),
}

pub async fn start() -> eyre::Result<()> {
    let EthrexL2CLI { command } = EthrexL2CLI::parse();
    if let EthrexL2Command::Config(cmd) = command {
        return cmd.run().await;
    }

    let cfg = load_selected_config().await?;
    match command {
        EthrexL2Command::Stack(cmd) => cmd.run(cfg).await?,
        EthrexL2Command::Wallet(cmd) => cmd.run(cfg).await?,
        EthrexL2Command::Autocomplete(cmd) => cmd.run()?,
        EthrexL2Command::Config(_) => unreachable!(),
        EthrexL2Command::Info(cmd) => cmd.run(cfg).await?,
        EthrexL2Command::Utils(cmd) => cmd.run()?,
    };
    Ok(())
}
