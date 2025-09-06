use std::time::{Duration, SystemTime};

use clap::Parser;
use ethrex_config::networks::{Network, PublicNetwork};
use ethrex_replay::{
    block_run_report::{BlockRunReport, ReplayerMode},
    cli::{BlockOptions, EthrexReplayCommand, EthrexReplayOptions, replayer_mode},
    slack::{SlackWebHookBlock, SlackWebHookRequest},
};
use ethrex_rpc::{EthClient, clients::EthClientError, types::block_identifier::BlockIdentifier};
use futures::future::select_all;
use reqwest::Url;
use tokio::task::{JoinError, JoinHandle};

#[derive(Parser)]
#[clap(group = clap::ArgGroup::new("rpc_urls").multiple(true).required(true))]
#[clap(group = clap::ArgGroup::new("modes").required(true))]
pub struct Options {
    #[arg(
        long,
        value_name = "URL",
        env = "SLACK_WEBHOOK_URL",
        help_heading = "Replayer options"
    )]
    pub slack_webhook_url: Option<Url>,
    #[arg(
        long,
        value_name = "URL",
        env = "HOODI_RPC_URL",
        help_heading = "Replayer options",
        group = "rpc_urls"
    )]
    pub hoodi_rpc_url: Option<Url>,
    #[arg(
        long,
        value_name = "URL",
        env = "SEPOLIA_RPC_URL",
        help_heading = "Replayer options",
        group = "rpc_urls"
    )]
    pub sepolia_rpc_url: Option<Url>,
    #[arg(
        long,
        value_name = "URL",
        env = "MAINNET_RPC_URL",
        help_heading = "Replayer options",
        group = "rpc_urls"
    )]
    pub mainnet_rpc_url: Option<Url>,
    #[arg(
        long,
        default_value_t = false,
        value_name = "BOOLEAN",
        group = "modes",
        help = "Replayer will execute blocks",
        help_heading = "Replayer options"
    )]
    pub execute: bool,
    #[arg(
        long,
        default_value_t = false,
        value_name = "BOOLEAN",
        group = "modes",
        help = "Replayer will prove blocks",
        help_heading = "Replayer options"
    )]
    pub prove: bool,
    #[arg(
        long,
        short = 'l',
        value_name = "LEVEL",
        default_value = "all",
        help = "Block cache level: off, failed, all (default: all)",
        help_heading = "Replayer options"
    )]
    pub cache_level: CacheLevel,
}

#[derive(clap::ValueEnum, Clone, Debug, PartialEq, Eq, Copy)]
pub enum CacheLevel {
    Off,
    Failed,
    All,
}

#[tokio::main]
async fn main() {
    init_tracing();

    let opts = Options::parse();

    if opts.slack_webhook_url.is_none() {
        tracing::warn!(
            "SLACK_WEBHOOK_URL environment variable is not set and --slack-webhook-url was not passed. Slack notifications will not be sent."
        );
    }

    let mut replayers_handles = Vec::new();

    if opts.execute {
        for (rpc_url, network) in [
            (
                opts.hoodi_rpc_url.clone(),
                Network::PublicNetwork(PublicNetwork::Hoodi),
            ),
            (
                opts.sepolia_rpc_url.clone(),
                Network::PublicNetwork(PublicNetwork::Sepolia),
            ),
            (
                opts.mainnet_rpc_url.clone(),
                Network::PublicNetwork(PublicNetwork::Mainnet),
            ),
        ] {
            let slack_webhook_url = opts.slack_webhook_url.clone();

            if let Some(rpc_url) = rpc_url {
                let handle = tokio::spawn(async move {
                    replay_execution(
                        replayer_mode(opts.execute).unwrap(),
                        network,
                        rpc_url,
                        slack_webhook_url,
                        opts.cache_level,
                    )
                    .await
                });

                replayers_handles.push(handle);
            }
        }
    } else {
        let slack_webhook_url = opts.slack_webhook_url.clone();
        let hoodi_rpc_url = opts.hoodi_rpc_url.clone();
        let sepolia_rpc_url = opts.sepolia_rpc_url.clone();
        let mainnet_rpc_url = opts.mainnet_rpc_url.clone();

        let handle = tokio::spawn(async move {
            replay_proving(
                replayer_mode(opts.execute).unwrap(),
                [
                    (hoodi_rpc_url, Network::PublicNetwork(PublicNetwork::Hoodi)),
                    (
                        sepolia_rpc_url,
                        Network::PublicNetwork(PublicNetwork::Sepolia),
                    ),
                    (
                        mainnet_rpc_url,
                        Network::PublicNetwork(PublicNetwork::Mainnet),
                    ),
                ],
                slack_webhook_url,
                opts.cache_level,
            )
            .await
        });

        replayers_handles.push(handle);
    };

    // TODO: These tasks are spawned outside the above loop to be able to handled
    // in the tokio::select!. We should find a way to spawn them inside the loop
    // and still be able to handle them in the tokio::select!.
    let mut revalidation_handles = Vec::new();
    if let Some(hoodi_rpc_url) = opts.hoodi_rpc_url.clone() {
        let handle = tokio::spawn(async move { revalidate_rpc(hoodi_rpc_url).await });
        revalidation_handles.push(handle);
    }
    if let Some(sepolia_rpc_url) = opts.sepolia_rpc_url.clone() {
        let handle = tokio::spawn(async move { revalidate_rpc(sepolia_rpc_url).await });
        revalidation_handles.push(handle);
    }
    if let Some(mainnet_rpc_url) = opts.mainnet_rpc_url.clone() {
        let handle = tokio::spawn(async move { revalidate_rpc(mainnet_rpc_url).await });
        revalidation_handles.push(handle);
    }

    tokio::select! {
        _ = tokio::signal::ctrl_c() => {
            tracing::info!("Received Ctrl+C, shutting down...");
            shutdown(replayers_handles);
        }
        res = async {
            let (result, _index, _remaining_handles) = select_all(revalidation_handles).await;
            result
        } => {
            handle_rpc_revalidation_handle_result(res, opts.hoodi_rpc_url.unwrap(), opts.slack_webhook_url.clone()).await; // TODO: change hoodi rpc to generic
            shutdown(replayers_handles);
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
    replayer_mode: ReplayerMode,
    network: Network,
    rpc_url: Url,
    slack_webhook_url: Option<Url>,
    cache_level: CacheLevel,
) -> Result<(), EthClientError> {
    tracing::info!("Starting execution replayer for network: {network} with RPC URL: {rpc_url}");

    let eth_client = EthClient::new(rpc_url.as_str()).unwrap();

    loop {
        let elapsed = replay_latest_block(
            replayer_mode.clone(),
            network.clone(),
            rpc_url.clone(),
            &eth_client,
            slack_webhook_url.clone(),
            cache_level,
        )
        .await?;

        // Wait at most 12 seconds for executing the next block.
        // This will only wait if the run took less than 12 seconds.
        tokio::time::sleep(Duration::from_secs(12).saturating_sub(elapsed)).await;
    }
}

async fn replay_proving(
    replayer_mode: ReplayerMode,
    rpc_urls: [(Option<Url>, Network); 3],
    slack_webhook_url: Option<Url>,
    cache_level: CacheLevel,
) -> Result<(), EthClientError> {
    loop {
        let start = SystemTime::now();
        for (rpc_url, network) in &rpc_urls {
            let rpc_url = if let Some(url) = rpc_url {
                url.clone()
            } else {
                continue;
            };
            let eth_client = EthClient::new(rpc_url.as_str()).unwrap();

            replay_latest_block(
                replayer_mode.clone(),
                network.clone(),
                rpc_url.clone(),
                &eth_client,
                slack_webhook_url.clone(),
                cache_level,
            )
            .await?;
        }
        let elapsed = start.elapsed().unwrap_or_else(|e| {
            panic!("SystemTime::elapsed failed: {e}");
        });

        // Wait at most 12 seconds for executing the next block.
        // This will only wait if the run took less than 12 seconds.
        tokio::time::sleep(Duration::from_secs(12).saturating_sub(elapsed)).await;
    }
}

async fn replay_latest_block(
    replayer_mode: ReplayerMode,
    network: Network,
    rpc_url: Url,
    eth_client: &EthClient,
    slack_webhook_url: Option<Url>,
    cache_level: CacheLevel,
) -> Result<Duration, EthClientError> {
    let mut latest_block = eth_client
        .get_block_number()
        .await
        .unwrap_or_else(|e| {
            panic!("Failed to get latest block number from {rpc_url}: {e}");
        })
        .as_u64();

    // We only want to replay prover killers (see https://github.com/yourbuddyconner/zkarnage?tab=readme-ov-file#background)
    while latest_block % 100 != 0 {
        latest_block = eth_client
            .get_block_number()
            .await
            .unwrap_or_else(|e| {
                panic!("Failed to get latest block number from {rpc_url}: {e}");
            })
            .as_u64();
    }

    if let Network::PublicNetwork(PublicNetwork::Mainnet) = network {
        tracing::info!("Replaying block https://etherscan.io/block/{latest_block}");
    } else {
        tracing::info!("Replaying block https://{network}.etherscan.io/block/{latest_block}",);
    }

    let block = eth_client
        .get_raw_block(BlockIdentifier::Number(latest_block))
        .await?;

    let start = SystemTime::now();

    let run_result = match replayer_mode {
        ReplayerMode::Execute | ReplayerMode::ExecuteSP1 | ReplayerMode::ExecuteRISC0 => {
            EthrexReplayCommand::Block(BlockOptions {
                block: Some(latest_block),
                opts: EthrexReplayOptions {
                    execute: true,
                    prove: false,
                    rpc_url,
                    cached: false,
                    bench: false,
                    to_csv: false,
                },
            })
            .run()
            .await
        }
        ReplayerMode::ProveSP1 | ReplayerMode::ProveRISC0 => {
            EthrexReplayCommand::Block(BlockOptions {
                block: Some(latest_block),
                opts: EthrexReplayOptions {
                    execute: false,
                    prove: true,
                    rpc_url,
                    cached: false,
                    bench: false,
                    to_csv: false,
                },
            })
            .run()
            .await
        }
    };

    let elapsed = start.elapsed().unwrap_or_else(|e| {
        panic!("SystemTime::elapsed failed: {e}");
    });

    let block_run_report = BlockRunReport::new_for(
        block,
        network.clone(),
        run_result,
        replayer_mode.clone(),
        elapsed,
    );

    if block_run_report.run_result.is_err() {
        tracing::error!("{block_run_report}");
    } else {
        tracing::info!("{block_run_report}");
    }

    // Caching logic: In replay every block is cached. So here we decide whether to keep the cache or not
    match cache_level {
        CacheLevel::Off => {
            // We don't want any cache
            tracing::info!("Deleting cache: Caching is disabled");
            delete_cache(network, latest_block);
        }
        CacheLevel::Failed => {
            // We only want caches that failed
            if block_run_report.run_result.is_ok() {
                tracing::info!(
                    "Deleting cache: Execution was successful and Cache Level is 'failed'"
                );
                delete_cache(network, latest_block);
            } else {
                // I prefer to be explicit about keeping the cache file
                tracing::info!(
                    "Keeping cache file for block {} on network {} because execution failed.",
                    latest_block,
                    network
                );
            }
        }
        CacheLevel::All => {}
    }

    if replayer_mode.is_proving_mode()
        || (replayer_mode.is_execution_mode() && block_run_report.run_result.is_err())
    {
        try_send_failed_run_report_to_slack(block_run_report, slack_webhook_url.clone())
            .await
            .unwrap_or_else(|e| {
                tracing::error!("Failed to post to Slack webhook: {e}");
            })
    }

    Ok(elapsed)
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

    let payload = SlackWebHookRequest {
        blocks: vec![
            SlackWebHookBlock::Header {
                text: Box::new(SlackWebHookBlock::PlainText {
                    text: "⚠️ RPC URL is no longer valid".to_string(),
                    emoji: true,
                }),
            },
            SlackWebHookBlock::Section {
                text: Box::new(SlackWebHookBlock::Markdown {
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

fn delete_cache(network: Network, block_number: u64) {
    // This file_name is the same used in ethrex_replay, this is a quick and simple solution but not ideal. Be aware that if we decide to change the name we have to do it in both places.
    let file_name = format!("cache_{network}_{block_number}.bin");
    if let Err(e) = std::fs::remove_file(&file_name) {
        if e.kind() != std::io::ErrorKind::NotFound {
            tracing::error!("Failed to delete cache file {}: {}", file_name, e);
        }
    }
}
