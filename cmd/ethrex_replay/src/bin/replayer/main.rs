use std::time::{Duration, SystemTime};

use clap::Parser;
use ethrex_replay::{
    cli::SubcommandExecute,
    networks::{Network, PublicNetwork},
};
use ethrex_rpc::{EthClient, clients::EthClientError, types::block_identifier::BlockIdentifier};
use reqwest::Url;
use tokio::task::{JoinError, JoinHandle};

use crate::block_execution_report::BlockExecutionReport;

mod block_execution_report;
mod slack;

#[derive(Parser)]
pub struct Options {
    #[arg(long, env = "SLACK_WEBHOOK_URL")]
    pub slack_webhook_url: Option<Url>,
    #[arg(long, env = "HOODI_RPC_URL")]
    pub hoodi_rpc_url: Url, // TODO: Make optional.
    #[arg(long, env = "SEPOLIA_RPC_URL")]
    pub sepolia_rpc_url: Url, // TODO: Make optional.
    #[arg(long, env = "MAINNET_RPC_URL")]
    pub mainnet_rpc_url: Url, // TODO: Make optional.
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

    let replayers = [
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
    ];

    let mut replayers_handles = Vec::new();

    for (rpc_url, network) in replayers.into_iter() {
        let slack_webhook_url = opts.slack_webhook_url.clone();

        let handle = tokio::spawn(async move { replay(rpc_url, network, slack_webhook_url).await });

        replayers_handles.push(handle);
    }

    // TODO: These tasks are spawned outside the above loop to be able to handled
    // in the tokio::select!. We should find a way to spawn them inside the loop
    // and still be able to handle them in the tokio::select!.
    let hoodi_rpc_url = opts.hoodi_rpc_url.clone();
    let hoodi_rpc_revalidation_handle = tokio::spawn(async { revalidate_rpc(hoodi_rpc_url).await });
    let sepolia_rpc_url = opts.sepolia_rpc_url.clone();
    let sepolia_rpc_revalidation_handle =
        tokio::spawn(async { revalidate_rpc(sepolia_rpc_url).await });
    let mainnet_rpc_url = opts.mainnet_rpc_url.clone();
    let mainnet_rpc_revalidation_handle =
        tokio::spawn(async { revalidate_rpc(mainnet_rpc_url).await });

    tokio::select! {
        _ = tokio::signal::ctrl_c() => {
            tracing::info!("Received Ctrl+C, shutting down...");
            shutdown(replayers_handles);
        }
        res = hoodi_rpc_revalidation_handle => {
            handle_rpc_revalidation_handle_result(res, opts.hoodi_rpc_url.clone(), opts.slack_webhook_url.clone()).await;
            shutdown(replayers_handles);
        }
        res = sepolia_rpc_revalidation_handle => {
            handle_rpc_revalidation_handle_result(res, opts.sepolia_rpc_url.clone(), opts.slack_webhook_url.clone()).await;
            shutdown(replayers_handles);
        }
        res = mainnet_rpc_revalidation_handle => {
            handle_rpc_revalidation_handle_result(res, opts.mainnet_rpc_url.clone(), opts.slack_webhook_url.clone()).await;
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

async fn replay(
    rpc_url: Url,
    network: Network,
    slack_webhook_url: Option<Url>,
) -> Result<(), EthClientError> {
    tracing::info!("Starting replayer for network: {network} with RPC URL: {rpc_url}");

    let eth_client = EthClient::new(rpc_url.as_str()).unwrap();

    let mut latest_block = eth_client.get_block_number().await?.as_usize();

    loop {
        let block = eth_client
            .get_raw_block(BlockIdentifier::Number(latest_block as u64))
            .await?;

        let start = SystemTime::now();

        let execution_result = SubcommandExecute::Block {
            block: Some(latest_block), // This will execute the latest block
            rpc_url: rpc_url.clone(),
            network: network.clone(),
            bench: false,
        }
        .run()
        .await;

        let elapsed = start.elapsed().unwrap_or_else(|e| {
            panic!("SystemTime::elapsed failed: {e}");
        });

        let block_execution_report =
            BlockExecutionReport::new_for(block, network.clone(), execution_result, elapsed);

        if block_execution_report.execution_result.is_err() {
            tracing::error!("{block_execution_report}");
        } else {
            tracing::info!("{block_execution_report}");
        }

        if block_execution_report.execution_result.is_err() {
            try_send_failed_execution_report_to_slack(
                block_execution_report,
                slack_webhook_url.clone(),
            )
            .await
            .unwrap_or_else(|e| {
                tracing::error!("Failed to post to Slack webhook: {e}");
            });
        }

        // Wait at most 12 seconds for executing the next block.
        // This will only wait if the execution took less than 12 seconds.
        tokio::time::sleep(Duration::from_secs(12).saturating_sub(elapsed)).await;

        latest_block = eth_client
            .get_block_number()
            .await
            .unwrap_or_else(|e| {
                panic!("Failed to get latest block number from {rpc_url}: {e}");
            })
            .as_usize();
    }
}

async fn revalidate_rpc(rpc_url: Url) -> Result<(), EthClientError> {
    let eth_client = EthClient::new(rpc_url.as_str()).unwrap();

    loop {
        let mut interval = tokio::time::interval(Duration::from_secs(10));

        eth_client.get_block_number().await.map(|_| ())?;

        interval.tick().await;
    }
}

async fn try_send_failed_execution_report_to_slack(
    report: BlockExecutionReport,
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
