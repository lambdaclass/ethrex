use crate::{
    cli::{self as ethrex_cli, Options},
    initializers::{
        get_local_p2p_node, get_network, get_signer, init_blockchain, init_metrics, init_network,
        init_rpc_api, init_store,
    },
    utils::{self, set_datadir, store_known_peers},
    DEFAULT_L2_DATADIR,
};
use clap::{Parser, Subcommand};
use ethrex_p2p::network::peer_table;
use secp256k1::SecretKey;
use std::{future::IntoFuture, path::PathBuf, time::Duration};
use tokio_util::task::TaskTracker;
use tracing::info;

#[allow(clippy::large_enum_variant)]
#[derive(Subcommand)]
pub enum Command {
    #[clap(about = "Initialize an ethrex L2 node", visible_alias = "i")]
    Init {
        #[clap(flatten)]
        opts: L2Options,
    },
    #[clap(name = "removedb", about = "Remove the database", visible_aliases = ["rm", "clean"])]
    RemoveDB {
        #[clap(long = "datadir", value_name = "DATABASE_DIRECTORY", default_value = DEFAULT_L2_DATADIR, required = false)]
        datadir: String,
        #[clap(long = "force", required = false, action = clap::ArgAction::SetTrue)]
        force: bool,
    },
}

#[derive(Parser, Default)]
pub struct L2Options {
    #[clap(flatten)]
    pub node_opts: Options,
    #[arg(
        long = "sponsorable-addresses",
        value_name = "SPONSORABLE_ADDRESSES_PATH",
        help = "Path to a file containing addresses of contracts to which ethrex_SendTransaction should sponsor txs",
        help_heading = "L2 options"
    )]
    pub sponsorable_addresses_file_path: Option<String>,
    #[arg(long, value_parser = utils::parse_private_key, env = "SPONSOR_PRIVATE_KEY", help = "The private key of ethrex L2 transactions sponsor.", help_heading = "L2 options")]
    pub sponsor_private_key: Option<SecretKey>,
    #[cfg(feature = "based")]
    #[clap(flatten)]
    pub based_opts: BasedOptions,
}

#[cfg(feature = "based")]
#[derive(Parser, Default)]
pub struct BasedOptions {
    #[arg(
        long = "gateway.addr",
        default_value = "0.0.0.0",
        value_name = "GATEWAY_ADDRESS",
        env = "GATEWAY_ADDRESS",
        help_heading = "Based options"
    )]
    pub gateway_addr: String,
    #[arg(
        long = "gateway.eth_port",
        default_value = "8546",
        value_name = "GATEWAY_ETH_PORT",
        env = "GATEWAY_ETH_PORT",
        help_heading = "Based options"
    )]
    pub gateway_eth_port: String,
    #[arg(
        long = "gateway.auth_port",
        default_value = "8553",
        value_name = "GATEWAY_AUTH_PORT",
        env = "GATEWAY_AUTH_PORT",
        help_heading = "Based options"
    )]
    pub gateway_auth_port: String,
    #[arg(
        long = "gateway.jwtsecret",
        default_value = "jwt.hex",
        value_name = "GATEWAY_JWTSECRET_PATH",
        env = "GATEWAY_JWTSECRET_PATH",
        help_heading = "Based options"
    )]
    pub gateway_jwtsecret: String,
    #[arg(
        long = "gateway.pubkey",
        value_name = "GATEWAY_PUBKEY",
        env = "GATEWAY_PUBKEY",
        help_heading = "Based options"
    )]
    pub gateway_pubkey: String,
}

impl Command {
    pub async fn run(self) -> eyre::Result<()> {
        match self {
            Command::Init { opts } => {
                let data_dir = set_datadir(&opts.node_opts.datadir);

                let network = get_network(&opts.node_opts);

                let store = init_store(&data_dir, &network);

                let blockchain = init_blockchain(opts.node_opts.evm, store.clone());

                let signer = get_signer(&data_dir);

                let local_p2p_node = get_local_p2p_node(&opts.node_opts, &signer);

                let peer_table = peer_table(signer.clone());

                // TODO: Check every module starts properly.
                let tracker = TaskTracker::new();

                let cancel_token = tokio_util::sync::CancellationToken::new();

                init_rpc_api(
                    &opts.node_opts,
                    &opts,
                    &signer,
                    peer_table.clone(),
                    local_p2p_node,
                    store.clone(),
                    blockchain.clone(),
                    cancel_token.clone(),
                    tracker.clone(),
                );

                // TODO: Add a --metrics flag to enable metrics.
                init_metrics(&opts.node_opts, tracker.clone());

                if opts.node_opts.p2p_enabled {
                    init_network(
                        &opts.node_opts,
                        &network,
                        &data_dir,
                        local_p2p_node,
                        signer,
                        peer_table.clone(),
                        store.clone(),
                        tracker.clone(),
                        blockchain.clone(),
                    )
                    .await;
                } else {
                    info!("P2P is disabled");
                }

                let l2_sequencer = ethrex_l2::start_l2(store, blockchain).into_future();

                tracker.spawn(l2_sequencer);

                tokio::select! {
                    _ = tokio::signal::ctrl_c() => {
                        info!("Server shut down started...");
                        let peers_file = PathBuf::from(data_dir + "/peers.json");
                        info!("Storing known peers at {:?}...", peers_file);
                        cancel_token.cancel();
                        store_known_peers(peer_table, peers_file).await;
                        tokio::time::sleep(Duration::from_secs(1)).await;
                        info!("Server shutting down!");
                    }
                }
            }
            Self::RemoveDB { datadir, force } => {
                Box::pin(async {
                    ethrex_cli::Subcommand::RemoveDB { datadir, force }
                        .run(&Options::default()) // This is not used by the RemoveDB command.
                        .await
                })
                .await?
            }
        }
        Ok(())
    }
}
