use std::time::Duration;

use crate::utils::config::{proposer::ProposerConfig, read_env_file};
use errors::ProposerError;
use ethereum_types::Address;
use ethrex_dev::utils::engine_client::config::EngineApiConfig;
use ethrex_storage::Store;
use tokio::time::sleep;
use tracing::{error, info};

pub mod l1_committer;
pub mod l1_watcher;
pub mod prover_server;
pub mod state_diff;

pub mod errors;

pub struct Proposer {
    engine_config: EngineApiConfig,
    block_production_interval: u64,
    coinbase_address: Address,
}

pub async fn start_proposer(store: Store) {
    info!("Starting Proposer");

    if let Err(e) = read_env_file() {
        panic!("Failed to read .env file: {e}");
    }

    let l1_watcher = tokio::spawn(l1_watcher::start_l1_watcher(store.clone()));
    let l1_committer = tokio::spawn(l1_committer::start_l1_commiter(store.clone()));
    let prover_server = tokio::spawn(prover_server::start_prover_server(store.clone()));
    let proposer = tokio::spawn(async move {
        let proposer_config = ProposerConfig::from_env().expect("ProposerConfig::from_env");
        let engine_config = EngineApiConfig::from_env().expect("EngineApiConfig::from_env");
        let proposer = Proposer::new_from_config(&proposer_config, engine_config)
            .expect("Proposer::new_from_config");

        proposer.run(store.clone()).await;
    });
    tokio::try_join!(l1_watcher, l1_committer, prover_server, proposer).expect("tokio::try_join");
}

impl Proposer {
    pub fn new_from_config(
        proposer_config: &ProposerConfig,
        engine_config: EngineApiConfig,
    ) -> Result<Self, ProposerError> {
        Ok(Self {
            engine_config,
            block_production_interval: proposer_config.interval_ms,
            coinbase_address: proposer_config.coinbase_address,
        })
    }

    pub async fn run(&self, store: Store) {
        loop {
            if let Err(err) = self.main_logic(store.clone()).await {
                error!("Block Producer Error: {}", err);
            }

            sleep(Duration::from_millis(200)).await;
        }
    }

    pub async fn main_logic(&self, store: Store) -> Result<(), ProposerError> {
        let head_block_hash = {
            let current_block_number = store
                .get_latest_block_number()?
                .ok_or(ProposerError::StorageDataIsNone)?;
            store
                .get_canonical_block_hash(current_block_number)?
                .ok_or(ProposerError::StorageDataIsNone)?
        };

        ethrex_dev::block_producer::start_block_producer(
            self.engine_config.rpc_url.clone(),
            std::fs::read(&self.engine_config.jwt_path).unwrap().into(),
            head_block_hash,
            10,
            self.block_production_interval,
            self.coinbase_address,
        )
        .await?;

        Ok(())
    }
}
