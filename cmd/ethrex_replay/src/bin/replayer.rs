use std::{
    fmt::Display,
    time::{Duration, SystemTime},
};

use ethrex_common::types::Block;
use ethrex_replay::{
    cli::SubcommandExecute,
    networks::{Network, PublicNetwork},
};
use ethrex_rpc::{EthClient, clients::EthClientError, types::block_identifier::BlockIdentifier};
use reqwest::Url;

pub struct BlockExecutionReport {
    pub network: Network,
    pub number: u64,
    pub gas: u64,
    pub txs: u64,
    pub execution_result: String,
}

impl BlockExecutionReport {
    pub fn new(block: Block, network: Network, execution_result: String) -> Self {
        Self {
            network,
            number: block.header.number,
            gas: block.header.gas_used,
            txs: block.body.transactions.len() as u64,
            execution_result,
        }
    }
}

impl Display for BlockExecutionReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Network::PublicNetwork(_) = self.network {
            write!(
                f,
                "[{network}] Block #{number}, Gas Used: {gas}, Tx Count: {txs}, Execution Result: {execution_result} | https://{network}.etherscan.io/block/{number}",
                network = self.network,
                number = self.number,
                gas = self.gas,
                txs = self.txs,
                execution_result = self.execution_result,
            )
        } else {
            write!(
                f,
                "[{}] Block #{}, Gas Used: {}, Tx Count: {}, Execution Result: {}",
                self.network, self.number, self.gas, self.txs, self.execution_result
            )
        }
    }
}

#[tokio::main]
async fn main() {
    init_tracing();

    let hoodi_rpc_url = "http://65.108.69.58:8545";
    let sepolia_rpc_url = "";
    let mainnet_rpc_url = "http://157.180.1.98:8545";

    let hoodi_task_handle = tokio::spawn(async {
        replay(hoodi_rpc_url, Network::PublicNetwork(PublicNetwork::Hoodi)).await
    });
    let hoodi_rpc_revalidation_handle = tokio::spawn(async { revalidate_rpc(hoodi_rpc_url).await });

    let sepolia_task_handle =
        tokio::spawn(async { replay("", Network::PublicNetwork(PublicNetwork::Sepolia)).await });
    let sepolia_rpc_revalidation_handle =
        tokio::spawn(async { revalidate_rpc(sepolia_rpc_url).await });

    let mainnet_task_handle = tokio::spawn(async {
        replay(
            mainnet_rpc_url,
            Network::PublicNetwork(PublicNetwork::Mainnet),
        )
        .await
    });
    let mainnet_rpc_revalidation_handle =
        tokio::spawn(async { revalidate_rpc(mainnet_rpc_url).await });

    tokio::select! {
        _ = tokio::signal::ctrl_c() => {
            tracing::info!("Received Ctrl+C, shutting down...");
            hoodi_task_handle.abort();
            sepolia_task_handle.abort();
            mainnet_task_handle.abort();
        }
        res = hoodi_rpc_revalidation_handle => {
            if let Err(e) = res {
                tracing::error!("Hoodi RPC failed: {e}");
            }
            sepolia_task_handle.abort();
            mainnet_task_handle.abort();
        }
        res = sepolia_rpc_revalidation_handle => {
            if let Err(e) = res {
                tracing::error!("Sepolia RPC failed: {e}");
            }
            hoodi_task_handle.abort();
            mainnet_task_handle.abort();
        }
        res = mainnet_rpc_revalidation_handle => {
            if let Err(e) = res {
                tracing::error!("Mainnet RPC failed: {e}");
            }
            hoodi_task_handle.abort();
            sepolia_task_handle.abort();
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

        let _execution_result = SubcommandExecute::Block {
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
