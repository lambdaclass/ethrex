use std::time::{Duration, SystemTime};

use ethrex_replay::{
    cli::SubcommandExecute,
    networks::{Network, PublicNetwork},
};
use ethrex_rpc::{EthClient, clients::EthClientError, types::block_identifier::BlockIdentifier};
use reqwest::Url;
use tokio::task::JoinHandle;

use crate::block_execution_report::BlockExecutionReport;

mod block_execution_report;
mod slack;

#[tokio::main]
async fn main() {
    if std::env::var("SLACK_WEBHOOK_URL").is_err() {
        tracing::warn!(
            "SLACK_WEBHOOK_URL environment variable is not set. Slack notifications will not be sent."
        );
    }

    init_tracing();

    // TODO: These RPC URLs should be configurable via environment variables or command line arguments.
    let hoodi_rpc_url = "http://65.108.69.58:8545";
    let sepolia_rpc_url = "";
    let mainnet_rpc_url = "http://157.180.1.98:8545";

    let replayers = [
        (Network::PublicNetwork(PublicNetwork::Hoodi), hoodi_rpc_url),
        (
            Network::PublicNetwork(PublicNetwork::Sepolia),
            sepolia_rpc_url,
        ),
        (
            Network::PublicNetwork(PublicNetwork::Mainnet),
            mainnet_rpc_url,
        ),
    ];

    let mut replayers_handles = Vec::new();

    for (network, rpc_url) in replayers {
        tracing::info!("Starting replayer for network: {network} with RPC URL: {rpc_url}");

        let handle = tokio::spawn(async move { replay(rpc_url, network).await });

        replayers_handles.push(handle);
    }

    // TODO: These tasks are spawned outside the above loop to be able to handled
    // in the tokio::select!. We should find a way to spawn them inside the loop
    // and still be able to handle them in the tokio::select!.
    let hoodi_rpc_revalidation_handle = tokio::spawn(async { revalidate_rpc(hoodi_rpc_url).await });
    let sepolia_rpc_revalidation_handle =
        tokio::spawn(async { revalidate_rpc(sepolia_rpc_url).await });
    let mainnet_rpc_revalidation_handle =
        tokio::spawn(async { revalidate_rpc(mainnet_rpc_url).await });

    tokio::select! {
        _ = tokio::signal::ctrl_c() => {
            tracing::info!("Received Ctrl+C, shutting down...");
            shutdown(replayers_handles);
        }
        res = hoodi_rpc_revalidation_handle => {
            if let Err(e) = res {
                tracing::error!("Hoodi RPC failed: {e}");
                try_notify_no_longer_valid_rpc_to_slack(
                    hoodi_rpc_url,
                    Network::PublicNetwork(PublicNetwork::Hoodi),
                ).await.unwrap_or_else(|e| {
                    tracing::error!("Failed to notify Slack about invalid Hoodi RPC: {e}");
                });
            }
            shutdown(replayers_handles);
        }
        res = sepolia_rpc_revalidation_handle => {
            if let Err(e) = res {
                tracing::error!("Sepolia RPC failed: {e}");
                try_notify_no_longer_valid_rpc_to_slack(
                    sepolia_rpc_url,
                    Network::PublicNetwork(PublicNetwork::Sepolia),
                ).await.unwrap_or_else(|e| {
                    tracing::error!("Failed to notify Slack about invalid Sepolia RPC: {e}");
                });
            }
            shutdown(replayers_handles);
        }
        res = mainnet_rpc_revalidation_handle => {
            if let Err(e) = res {
                tracing::error!("Mainnet RPC failed: {e}");
                try_notify_no_longer_valid_rpc_to_slack(
                    mainnet_rpc_url,
                    Network::PublicNetwork(PublicNetwork::Mainnet),
                ).await.unwrap_or_else(|e| {
                    tracing::error!("Failed to notify Slack about invalid Mainnet RPC: {e}");
                });
            }
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

async fn replay(rpc_url: &str, network: Network) -> Result<(), EthClientError> {
    tracing::info!("Starting execution for network: {network}");

    let eth_client = EthClient::new(rpc_url).unwrap();

    let mut latest_block = eth_client.get_block_number().await?.as_usize();

    loop {
        let block = eth_client
            .get_raw_block(BlockIdentifier::Number(latest_block as u64))
            .await?;

        let start = SystemTime::now();

        let execution_result = SubcommandExecute::Block {
            block: Some(latest_block), // This will execute the latest block
            rpc_url: Url::parse(rpc_url).unwrap(),
            network: network.clone(),
            bench: false,
        }
        .run()
        .await
        .inspect_err(|e| {
            tracing::error!("Error executing block: {e:?}");
        });

        let elapsed = start.elapsed().unwrap_or_else(|e| {
            panic!("SystemTime::elapsed failed: {e}");
        });

        let block_execution_report =
            BlockExecutionReport::new_for(block, network.clone(), execution_result, elapsed);

        tracing::info!("{block_execution_report}");

        if block_execution_report.execution_result.is_err() {
            try_send_failed_execution_report_to_slack(block_execution_report)
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

async fn revalidate_rpc(rpc_url: &str) -> Result<(), EthClientError> {
    let eth_client = EthClient::new(rpc_url).unwrap();

    loop {
        let mut interval = tokio::time::interval(Duration::from_secs(10));

        eth_client.get_block_number().await.map(|_| ())?;

        interval.tick().await;
    }
}

async fn try_send_failed_execution_report_to_slack(
    report: BlockExecutionReport,
) -> Result<(), reqwest::Error> {
    let Ok(webhook_url) = std::env::var("SLACK_WEBHOOK_URL") else {
        return Ok(());
    };

    let client = reqwest::Client::new();

    let payload = report.to_slack_message();

    client.post(webhook_url).json(&payload).send().await?;

    Ok(())
}

async fn try_notify_no_longer_valid_rpc_to_slack(
    rpc_url: &str,
    network: Network,
) -> Result<(), reqwest::Error> {
    let Ok(webhook_url) = std::env::var("SLACK_WEBHOOK_URL") else {
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

fn shutdown(handles: Vec<JoinHandle<Result<(), EthClientError>>>) {
    tracing::info!("Shutting down...");

    for handle in handles {
        if !handle.is_finished() {
            handle.abort();
        }
    }
}
