#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
use ethereum_types::Address;
use ethrex_l2::utils::config::read_env_file_by_config;
use ethrex_rpc::clients::eth::EthClient;
use std::{str::FromStr, time::Duration};
use tokio::time::{interval, timeout};

const DEFAULT_ETH_URL: &str = "http://localhost:8545";

#[tokio::test]
async fn prover_client() -> Result<(), Box<dyn std::error::Error>> {
    let eth_client = eth_client();

    read_env_file_by_config(ethrex_l2::utils::config::ConfigMode::Sequencer)?;

    let on_chain_proposer_address = &std::env::var("COMMITTER_ON_CHAIN_PROPOSER_ADDRESS")?;

    let timeout_duration = Duration::from_secs(1000);
    let check_interval = Duration::from_secs(5);

    timeout(timeout_duration, async move {
        let mut interval = interval(check_interval);
        loop {
            interval.tick().await;
            let last_verified_block = EthClient::get_last_verified_block(
                &eth_client,
                Address::from_str(on_chain_proposer_address).unwrap(),
            )
            .await
            .unwrap();

            println!("Last Verified Block: {last_verified_block}");

            if last_verified_block != 0 {
                println!("Last Verified Block changed to non-zero: {last_verified_block}");
                break;
            }
        }
    })
    .await?;

    Ok(())
}

fn eth_client() -> EthClient {
    EthClient::new(DEFAULT_ETH_URL)
}
