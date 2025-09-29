use std::time::{Duration, SystemTime};

use clap::Parser;
use ethrex_config::networks::{Network, PublicNetwork};
use ethrex_replay::{
    cli::{
        Action, BlockOptions, CacheLevel, CommonOptions, EthrexReplayCommand, EthrexReplayOptions,
    },
    report::Report,
    slack::{SlackWebHookBlock, SlackWebHookRequest, try_send_report_to_slack},
};
use ethrex_rpc::{EthClient, clients::EthClientError, types::block_identifier::BlockIdentifier};
use futures::future::select_all;
use reqwest::Url;
use tokio::task::{JoinError, JoinHandle};

#[derive(Parser, Clone)]
#[clap(group = clap::ArgGroup::new("rpc_urls").multiple(true).required(true))]
pub struct Options {
    #[command(flatten)]
    pub common: CommonOptions,
    #[arg(long, default_value = "on", help_heading = "Replay Options")]
    pub cache_level: CacheLevel,
    #[arg(long, env = "SLACK_WEBHOOK_URL", help_heading = "Replay Options")]
    pub slack_webhook_url: Option<Url>,
    #[arg(
        long,
        help = "Execute with `Blockchain::add_block`, without using zkvm as backend",
        help_heading = "Replay Options",
        conflicts_with = "zkvm"
    )]
    pub no_zkvm: bool,
    #[arg(
        long,
        short,
        help = "Enable verbose logging",
        help_heading = "Replay Options",
        required = false
    )]
    pub verbose: bool,
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

    match opts.common.action {
        Action::Execute => {
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
                let opts = opts.clone();

                if let Some(rpc_url) = rpc_url {
                    let handle =
                        tokio::spawn(async move { replay_execution(opts, network, rpc_url).await });

                    replayers_handles.push(handle);
                }
            }
        }
        Action::Prove => {
            let hoodi_rpc_url = opts.hoodi_rpc_url.clone();
            let sepolia_rpc_url = opts.sepolia_rpc_url.clone();
            let mainnet_rpc_url = opts.mainnet_rpc_url.clone();
            let opts = opts.clone();

            let handle = tokio::spawn(async move {
                replay_proving(
                    opts,
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
                )
                .await
            });

            replayers_handles.push(handle);
        }
    }

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
    opts: Options,
    network: Network,
    rpc_url: Url,
) -> Result<(), EthClientError> {
    tracing::info!("Starting execution replayer for network: {network} with RPC URL: {rpc_url}");

    let eth_client = EthClient::new(rpc_url.as_str()).unwrap();

    loop {
        let elapsed =
            replay_latest_block(opts.clone(), network.clone(), rpc_url.clone(), &eth_client)
                .await?;

        // Wait at most 12 seconds for executing the next block.
        // This will only wait if the run took less than 12 seconds.
        tokio::time::sleep(Duration::from_secs(12).saturating_sub(elapsed)).await;
    }
}

async fn replay_proving(
    opts: Options,
    rpc_urls: [(Option<Url>, Network); 3],
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

            replay_latest_block(opts.clone(), network.clone(), rpc_url.clone(), &eth_client)
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
    opts: Options,
    network: Network,
    rpc_url: Url,
    eth_client: &EthClient,
) -> Result<Duration, EthClientError> {
    let latest_block = eth_client
        .get_block_number()
        .await
        .unwrap_or_else(|e| {
            panic!("Failed to get latest block number from {rpc_url}: {e}");
        })
        .as_u64();

    if let Network::PublicNetwork(PublicNetwork::Mainnet) = network {
        tracing::info!("Replaying block https://etherscan.io/block/{latest_block}");
    } else {
        tracing::info!("Replaying block https://{network}.etherscan.io/block/{latest_block}",);
    }

    let rpc_block = eth_client
        .get_block_by_number(BlockIdentifier::Number(latest_block), true)
        .await?;

    let block = rpc_block.try_into().expect("RPCBlock should be hydrated");

    let start = SystemTime::now();

    let execution_result = EthrexReplayCommand::Block(BlockOptions {
        block: Some(latest_block),
        opts: EthrexReplayOptions {
            common: CommonOptions {
                zkvm: opts.common.zkvm.clone(),
                resource: opts.common.resource.clone(),
                action: Action::Execute,
            },
            rpc_url: rpc_url.clone(),
            cached: false,
            no_zkvm: opts.no_zkvm,
            cache_level: opts.cache_level.clone(),
            // Setting this will send the message always, we opted to
            // send it under different rules (see below in the code).
            slack_webhook_url: None,
            verbose: opts.verbose,
        },
    })
    .run()
    .await
    .map(|_| start.elapsed().expect("SystemTime::elapsed failed"));

    let start = SystemTime::now();

    let proving_result = match opts.common.action {
        Action::Execute => None,
        Action::Prove => Some(
            EthrexReplayCommand::Block(BlockOptions {
                block: Some(latest_block),
                opts: EthrexReplayOptions {
                    common: CommonOptions {
                        zkvm: opts.common.zkvm.clone(),
                        resource: opts.common.resource.clone(),
                        action: Action::Prove,
                    },
                    rpc_url,
                    cached: false,
                    no_zkvm: false,
                    cache_level: opts.cache_level.clone(),
                    // Setting this will send the message always, we opted to
                    // send it under different rules (see below in the code).
                    slack_webhook_url: None,
                    verbose: opts.verbose,
                },
            })
            .run()
            .await
            .map(|_| start.elapsed().expect("SystemTime::elapsed failed")),
        ),
    };

    let elapsed = start.elapsed().unwrap_or_else(|e| {
        panic!("SystemTime::elapsed failed: {e}");
    });

    let execution_failed = execution_result.is_err();

    let report = Report::new_for(
        opts.common.zkvm,
        opts.common.resource,
        opts.common.action.clone(),
        block,
        network,
        execution_result,
        proving_result,
    );

    if opts.common.action == Action::Prove
        || (opts.common.action == Action::Execute && execution_failed)
    {
        try_send_report_to_slack(&report, opts.slack_webhook_url.clone()).await?;
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
