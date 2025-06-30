use std::error::Error;

use clap::Parser;
use ethrex_common::Address;

mod monitor;
mod runner;

mod ui;

#[derive(Debug, Parser)]
struct MonitorOptions {
    #[arg(
        long = "l1.rpc-url",
        value_name = "ETH_RPC_URL",
        env = "ETHREX_ETH_RPC_URL",
        help_heading = "Ethrex monitor options"
    )]
    pub l1_rpc_url: String,
    #[arg(
        long = "l2.rpc-url",
        value_name = "L2_RPC_URL",
        env = "ETHREX_RPC_URL",
        help_heading = "Ethrex monitor options"
    )]
    pub l2_rpc_url: String,
    #[arg(
        long = "l1.on-chain-proposer-address",
        value_name = "ADDRESS",
        env = "ETHREX_ON_CHAIN_PROPOSER_ADDRESS",
        help_heading = "Ethrex monitor options"
    )]
    on_chain_proposer_address: Address,
    #[arg(
        long = "l1.common-bridge-address",
        value_name = "ADDRESS",
        env = "ETHREX_COMMON_BRIDGE_ADDRESS",
        help_heading = "Ethrex monitor options"
    )]
    common_bridge_address: Address,
    #[arg(
        long = "l1.sequencer-registry-address",
        value_name = "ADDRESS",
        env = "ETHREX_SEQUENCER_REGISTRY_ADDRESS",
        required_if_eq("based", "true"),
        help_heading = "Ethrex monitor options"
    )]
    sequencer_registry_address: Option<Address>,
    #[arg(
        long,
        default_value = "false",
        value_name = "BOOLEAN",
        env = "ETHREX_BASED",
        help_heading = "Ethrex monitor options"
    )]
    based: bool,
    /// time in ms between two ticks.
    #[arg(short, long, default_value_t = 250)]
    tick_rate: u64,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let opts = MonitorOptions::parse();
    crate::runner::run(opts).await?;
    Ok(())
}
