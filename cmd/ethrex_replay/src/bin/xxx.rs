use std::time::{Duration, SystemTime};

use ethrex_replay::{
    cli::SubcommandExecute,
    networks::{Network, PublicNetwork},
};
use reqwest::Url;

#[tokio::main]
async fn main() {
    init_tracing();

    let hoodi_task_handle = tokio::spawn(async {
        xxx(
            "http://65.108.69.58:8545",
            Network::PublicNetwork(PublicNetwork::Hoodi),
        )
        .await;
    });

    // let sepolia_task_handle = tokio::spawn(async {
    //     xxx(
    //         "",
    //         Network::PublicNetwork(PublicNetwork::Sepolia),
    //     )
    //     .await;
    // });

    // let mainnet_task_handle = tokio::spawn(async {
    //     xxx(
    //         "http://157.180.1.98:8545",
    //         Network::PublicNetwork(PublicNetwork::Mainnet),
    //     )
    //     .await;
    // });

    tokio::select! {
        _ = tokio::signal::ctrl_c() => {
            tracing::info!("Received Ctrl+C, shutting down...");
            hoodi_task_handle.abort();
            // sepolia_task_handle.abort();
            // mainnet_task_handle.abort();
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

async fn xxx(rpc_url: &str, network: Network) {
    tracing::info!("Starting execution for network: {network:?}");

    loop {
        let start = SystemTime::now();

        let _ = SubcommandExecute::Block {
            block: None, // This will execute the latest block
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

        tokio::time::sleep(Duration::from_secs(12).saturating_sub(elapsed)).await;
    }
}
