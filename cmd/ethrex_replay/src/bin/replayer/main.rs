use std::{
    fmt::Display,
    time::{Duration, SystemTime},
};

use clap::Parser;
use ethrex_common::types::Block;
use ethrex_config::networks::{Network, PublicNetwork};
use ethrex_replay::cli::{SubcommandExecute, SubcommandProve};
use ethrex_rpc::{EthClient, clients::EthClientError, types::block_identifier::BlockIdentifier};
use reqwest::Url;
use tokio::{
    join,
    task::{JoinError, JoinHandle},
};
use tracing::warn;

use crate::block_execution_report::BlockRunReport;

mod block_execution_report;
mod slack;

#[derive(Parser)]
pub struct Options {
    #[arg(
        long,
        value_name = "URL",
        env = "SLACK_WEBHOOK_URL",
        help_heading = "Replayer options"
    )]
    pub slack_webhook_url: Option<Url>,
    // #[arg(
    //     long,
    //     value_name = "URL",
    //     env = "HOODI_RPC_URL",
    //     help_heading = "Replayer options"
    // )]
    // pub hoodi_rpc_url: Url,
    // #[arg(
    //     long,
    //     value_name = "URL",
    //     env = "SEPOLIA_RPC_URL",
    //     help_heading = "Replayer options"
    // )]
    // pub sepolia_rpc_url: Url,
    // #[arg(
    //     long,
    //     value_name = "URL",
    //     env = "MAINNET_RPC_URL",
    //     help_heading = "Replayer options"
    // )]
    // pub mainnet_rpc_url: Url,
    #[arg(
        long,
        default_value_t = false,
        value_name = "BOOLEAN",
        conflicts_with = "prove",
        help = "Replayer will execute blocks",
        help_heading = "Replayer options"
    )]
    pub execute: bool,
    #[arg(
        long,
        default_value_t = false,
        value_name = "BOOLEAN",
        conflicts_with = "execute",
        help = "Replayer will prove blocks",
        help_heading = "Replayer options"
    )]
    pub prove: bool,
    #[arg(
        long,
        value_name = "URL",
        env = "GETH_RPC_URL",
        help_heading = "Replayer options"
    )]
    pub geth_rpc_url: Url,
    #[arg(
        long,
        value_name = "URL",
        env = "RETH_RPC_URL",
        help_heading = "Replayer options"
    )]
    pub reth_rpc_url: Url,
    #[arg(
        long,
        value_name = "URL",
        env = "NETHERMIND_RPC_URL",
        help_heading = "Replayer options"
    )]
    pub nethermind_rpc_url: Url,
    #[arg(long, required = false)]
    cache: bool,
}

#[derive(Debug, Clone)]
pub enum ReplayerMode {
    Execute,
    Prove,
}

impl Display for ReplayerMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ReplayerMode::Execute => write!(f, "Execute"),
            ReplayerMode::Prove => write!(f, "Prove"),
        }
    }
}

#[tokio::main]
async fn main() {
    init_tracing();

    let opts = Options::parse();

    if !opts.execute && !opts.prove {
        tracing::error!("You must specify either --execute or --prove.");
        std::process::exit(1);
    }

    if opts.slack_webhook_url.is_none() {
        tracing::warn!(
            "SLACK_WEBHOOK_URL environment variable is not set and --slack-webhook-url was not passed. Slack notifications will not be sent."
        );
    }

    let main_handle = if opts.execute {
        let slack_webhook_url = opts.slack_webhook_url.clone();

        let geth_rpc_url = opts.geth_rpc_url.clone();

        let reth_rpc_url = opts.reth_rpc_url.clone();

        let nethermind_rpc_url = opts.nethermind_rpc_url.clone();

        tokio::spawn(async {
            replay_client_diversity(
                geth_rpc_url,
                reth_rpc_url,
                nethermind_rpc_url,
                slack_webhook_url,
                ReplayerMode::Execute,
            )
            .await
        })
    } else {
        todo!("Proving mode is not implemented yet for client diversity replayer");
    };

    // TODO: These tasks are spawned outside the above loop to be able to handled
    // in the tokio::select!. We should find a way to spawn them inside the loop
    // and still be able to handle them in the tokio::select!.
    let geth_rpc_url = opts.geth_rpc_url.clone();
    let geth_rpc_revalidation_handle = tokio::spawn(async { revalidate_rpc(geth_rpc_url).await });
    let reth_rpc_url = opts.reth_rpc_url.clone();
    let reth_rpc_revalidation_handle = tokio::spawn(async { revalidate_rpc(reth_rpc_url).await });
    let nethermind_rpc_url = opts.nethermind_rpc_url.clone();
    let nethermind_rpc_revalidation_handle =
        tokio::spawn(async { revalidate_rpc(nethermind_rpc_url).await });

    tokio::select! {
        _ = tokio::signal::ctrl_c() => {
            tracing::info!("Received Ctrl+C, shutting down...");
            shutdown(vec![main_handle]);
        }
        res = geth_rpc_revalidation_handle => {
            handle_rpc_revalidation_handle_result(res, opts.geth_rpc_url.clone(), opts.slack_webhook_url.clone()).await;
            shutdown(vec![main_handle]);
        }
        res = reth_rpc_revalidation_handle => {
            handle_rpc_revalidation_handle_result(res, opts.reth_rpc_url.clone(), opts.slack_webhook_url.clone()).await;
            shutdown(vec![main_handle]);
        }
        res = nethermind_rpc_revalidation_handle => {
            handle_rpc_revalidation_handle_result(res, opts.nethermind_rpc_url.clone(), opts.slack_webhook_url.clone()).await;
            shutdown(vec![main_handle]);
        }
    }
}

fn init_tracing() {
    let log_filter = tracing_subscriber::EnvFilter::builder()
        .with_default_directive(
            // Filters all sp1-executor logs (clock and program counter information)
            <tracing_subscriber::filter::Directive as std::str::FromStr>::from_str(
                "sp1_core_executor::executor=off",
            )
            .expect("this can't fail"),
        )
        .from_env_lossy()
        .add_directive(tracing_subscriber::filter::Directive::from(
            tracing::Level::INFO,
        ));
    tracing::subscriber::set_global_default(
        tracing_subscriber::FmtSubscriber::builder()
            .with_env_filter(log_filter)
            .finish(),
    )
    .expect("setting default subscriber failed");
}

async fn replay_execution(
    network: Network,
    client: String,
    rpc_url: Url,
    slack_webhook_url: Option<Url>,
) -> Result<(), EthClientError> {
    tracing::info!("Starting execution replayer for network: {network} with RPC URL: {rpc_url}");

    let eth_client = EthClient::new(rpc_url.as_str()).unwrap();

    loop {
        let block_run_report = replay_latest_block(
            ReplayerMode::Execute,
            network.clone(),
            client.clone(),
            rpc_url.clone(),
            &eth_client,
        )
        .await;

        let elapsed = block_run_report.time_taken;

        if block_run_report.run_result.is_err() {
            tracing::error!("{block_run_report}");
        } else {
            tracing::info!("{block_run_report}");
        }

        if block_run_report.run_result.is_err() {
            try_send_failed_run_report_to_slack(block_run_report, slack_webhook_url.clone())
                .await
                .unwrap_or_else(|e| {
                    tracing::error!("Failed to post to Slack webhook: {e}");
                });
        }

        // Wait at most 12 seconds for executing the next block.
        // This will only wait if the run took less than 12 seconds.
        tokio::time::sleep(Duration::from_secs(12).saturating_sub(elapsed)).await;
    }
}

async fn replay_client_diversity(
    geth_rpc_url: Url,
    reth_rpc_url: Url,
    nethermind_rpc_url: Url,
    slack_webhook_url: Option<Url>,
    replayer_mode: ReplayerMode,
) -> Result<(), EthClientError> {
    tracing::info!(
        "Starting client diversity replayer with mode: {replayer_mode} with RPCs: \nGeth: {geth_rpc_url}\nReth: {reth_rpc_url}\nNethermind: {nethermind_rpc_url}"
    );

    loop {
        let geth_client = EthClient::new(geth_rpc_url.as_str()).unwrap();
        let reth_client = EthClient::new(reth_rpc_url.as_str()).unwrap();
        let nethermind_client = EthClient::new(nethermind_rpc_url.as_str()).unwrap();

        let latest_block = geth_client
            .get_block_number()
            .await
            .unwrap_or_else(|e| {
                panic!("Failed to get latest block number from Geth RPC: {e}");
            })
            .as_usize();

        let start = SystemTime::now();

        let mut handles = Vec::new();

        for (eth_client, rpc_url, client_name) in [
            (geth_client, &geth_rpc_url, "geth"),
            (reth_client, &reth_rpc_url, "reth"),
            (nethermind_client, &nethermind_rpc_url, "nethermind"),
        ] {
            let chain_id = eth_client
                .get_chain_id()
                .await
                .unwrap_or_else(|e| {
                    panic!("Failed to get chain ID from {rpc_url}: {e}");
                })
                .as_u64();

            let rpc_url_clone = rpc_url.clone();

            let replayer_mode = replayer_mode.clone();

            let handle = tokio::spawn(async move {
                replay_block(
                    latest_block,
                    replayer_mode,
                    Network::try_from_chain_id(chain_id),
                    client_name.to_string(),
                    rpc_url_clone,
                    &eth_client,
                )
                .await
            });

            handles.push(handle);
        }

        let (geth_result, reth_result, nethermind_result) =
            join!(handles.remove(0), handles.remove(0), handles.remove(0));

        let elapsed = start.elapsed().unwrap_or_else(|e| {
            panic!("SystemTime::elapsed failed: {e}");
        });

        {
            let geth_report = geth_result.unwrap_or_else(|e| {
                panic!("Failed to replay block on Geth RPC: {e}");
            });

            let reth_report = reth_result.unwrap_or_else(|e| {
                panic!("Failed to replay block on Reth RPC: {e}");
            });

            let nethermind_report = nethermind_result.unwrap_or_else(|e| {
                panic!("Failed to replay block on Nethermind RPC: {e}");
            });

            for (client, report) in [
                ("geth", geth_report),
                ("reth", reth_report),
                ("nethermind", nethermind_report),
            ] {
                if report.run_result.is_err() {
                    tracing::error!("[{client}] {report}");
                } else {
                    tracing::info!("[{client}] {report}");
                }

                if report.run_result.is_err() {
                    try_send_failed_run_report_to_slack(report, slack_webhook_url.clone())
                        .await
                        .unwrap_or_else(|e| {
                            tracing::error!("Failed to post to Slack webhook: {e}");
                        });
                }
            }
        }

        // Wait at most 12 seconds for executing the next block.
        // This will only wait if the run took less than 13 seconds.
        // Block time is 12s, but wait an extra second to ensure the next block
        // is different available.
        tokio::time::sleep(Duration::from_secs(13).saturating_sub(elapsed)).await;
    }
}

async fn replay_proving(
    client: String,
    hoodi_rpc_url: Url,
    sepolia_rpc_url: Url,
    mainnet_rpc_url: Url,
) -> Result<(), EthClientError> {
    let hoodi_eth_client = EthClient::new(hoodi_rpc_url.as_str()).unwrap();
    let sepolia_eth_client = EthClient::new(sepolia_rpc_url.as_str()).unwrap();
    let mainnet_eth_client = EthClient::new(mainnet_rpc_url.as_str()).unwrap();

    loop {
        let start = SystemTime::now();

        let hoodi_block_run_report = replay_latest_block(
            ReplayerMode::Prove,
            Network::PublicNetwork(PublicNetwork::Hoodi),
            client.clone(),
            hoodi_rpc_url.clone(),
            &hoodi_eth_client,
        )
        .await;

        let sepolia_block_run_report = replay_latest_block(
            ReplayerMode::Prove,
            Network::PublicNetwork(PublicNetwork::Sepolia),
            client.clone(),
            sepolia_rpc_url.clone(),
            &sepolia_eth_client,
        )
        .await;

        let mainnet_run_report = replay_latest_block(
            ReplayerMode::Prove,
            Network::PublicNetwork(PublicNetwork::Mainnet),
            client.clone(),
            mainnet_rpc_url.clone(),
            &mainnet_eth_client,
        )
        .await;

        let elapsed = start.elapsed().unwrap_or_else(|e| {
            panic!("SystemTime::elapsed failed: {e}");
        });

        for report in [
            hoodi_block_run_report,
            sepolia_block_run_report,
            mainnet_run_report,
        ] {
            if report.run_result.is_err() {
                tracing::error!("{report}");
            } else {
                tracing::info!("{report}");
            }

            if report.run_result.is_err() {
                try_send_failed_run_report_to_slack(report, None)
                    .await
                    .unwrap_or_else(|e| {
                        warn!("Failed to post to Slack webhook: {e}");
                    });
            }
        }

        // Wait at most 12 seconds for executing the next block.
        // This will only wait if the run took less than 12 seconds.
        tokio::time::sleep(Duration::from_secs(12).saturating_sub(elapsed)).await;
    }
}

async fn replay_latest_block(
    replayer_mode: ReplayerMode,
    network: Network,
    client: String,
    rpc_url: Url,
    eth_client: &EthClient,
) -> BlockRunReport {
    let latest_block = eth_client
        .get_block_number()
        .await
        .unwrap_or_else(|e| {
            panic!("Failed to get latest block number from {rpc_url}: {e}");
        })
        .as_usize();

    replay_block(
        latest_block,
        replayer_mode,
        network,
        client,
        rpc_url,
        eth_client,
    )
    .await
}

async fn replay_block(
    block_to_replay: usize,
    replayer_mode: ReplayerMode,
    network: Network,
    client: String,
    rpc_url: Url,
    eth_client: &EthClient,
) -> BlockRunReport {
    let block = match eth_client
        .get_raw_block(BlockIdentifier::Number(block_to_replay as u64))
        .await
    {
        Ok(block) => block,
        Err(e) => {
            return BlockRunReport::new_for(
                client,
                Block::default(),
                network.clone(),
                Err(eyre::eyre!(
                    "Failed to fetch raw block {block_to_replay} from {rpc_url}: {e}"
                )),
                replayer_mode,
                Duration::ZERO,
            );
        }
    };

    let start = SystemTime::now();

    let run_result = match replayer_mode {
        ReplayerMode::Execute => {
            SubcommandExecute::Block {
                block: Some(block_to_replay),
                rpc_url: rpc_url.clone(),
                network: network.clone(),
                bench: false,
                cache: false, // TODO: Parametrize
            }
            .run()
            .await
        }
        ReplayerMode::Prove => {
            SubcommandProve::Block {
                block: Some(block_to_replay),
                rpc_url: rpc_url.clone(),
                network: network.clone(),
                bench: false,
                cache: false, // TODO: Parametrize
            }
            .run()
            .await
        }
    };

    let elapsed = start.elapsed().unwrap_or_else(|e| {
        panic!("SystemTime::elapsed failed: {e}");
    });

    BlockRunReport::new_for(
        client,
        block,
        network.clone(),
        run_result,
        replayer_mode,
        elapsed,
    )
}

async fn revalidate_rpc(rpc_url: Url) -> Result<(), EthClientError> {
    let eth_client = EthClient::new(rpc_url.as_str()).unwrap();

    loop {
        let mut interval = tokio::time::interval(Duration::from_secs(10));

        eth_client.get_block_number().await.map(|_| ())?;

        interval.tick().await;
    }
}

async fn try_send_failed_run_report_to_slack(
    report: BlockRunReport,
    slack_webhook_url: Option<Url>,
) -> Result<(), reqwest::Error> {
    let Some(webhook_url) = slack_webhook_url else {
        return Ok(());
    };

    let client = reqwest::Client::new();

    let payload = report.to_slack_message();

    client.post(webhook_url).json(&payload).send().await?;

    Ok(())
}

async fn try_notify_no_longer_valid_rpc_to_slack(
    rpc_url: Url,
    network: Network,
    slack_webhook_url: Option<Url>,
) -> Result<(), reqwest::Error> {
    let Some(webhook_url) = slack_webhook_url else {
        return Ok(());
    };

    let client = reqwest::Client::new();

    let payload = slack::SlackWebHookRequest {
        blocks: vec![
            slack::SlackWebHookBlock::Header {
                text: Box::new(slack::SlackWebHookBlock::PlainText {
                    text: "⚠️ RPC URL is no longer valid".to_string(),
                    emoji: true,
                }),
            },
            slack::SlackWebHookBlock::Section {
                text: Box::new(slack::SlackWebHookBlock::Markdown {
                    text: format!("`{network}`'s RPC URL `{rpc_url}` is no longer valid."),
                }),
            },
        ],
    };

    client.post(webhook_url).json(&payload).send().await?;

    Ok(())
}

async fn handle_rpc_revalidation_handle_result(
    res: Result<Result<(), EthClientError>, JoinError>,
    rpc_url: Url,
    slack_webhook_url: Option<Url>,
) {
    if let Err(e) = res {
        tracing::error!("Sepolia RPC failed: {e}");
        try_notify_no_longer_valid_rpc_to_slack(
            rpc_url,
            Network::PublicNetwork(PublicNetwork::Sepolia),
            slack_webhook_url,
        )
        .await
        .unwrap_or_else(|e| {
            tracing::error!("Failed to notify Slack about invalid Sepolia RPC: {e}");
        });
    }
}

fn shutdown(handles: Vec<JoinHandle<Result<(), EthClientError>>>) {
    tracing::info!("Shutting down...");

    for handle in handles {
        if !handle.is_finished() {
            handle.abort();
        }
    }
}
