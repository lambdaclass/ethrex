use std::time::Duration;

use crate::utils::config::{errors::ConfigError, proposer::ProposerConfig, read_env_file};
use errors::ProposerError;
use ethereum_types::Address;
use ethrex_dev::utils::engine_client::config::EngineApiConfig;
use ethrex_storage::Store;
use tokio::task::JoinSet;
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
    jwt_secret: Vec<u8>,
}

pub async fn start_proposer(store: Store) {
    info!("Starting Proposer");

    if let Err(e) = read_env_file() {
        error!("Failed to read .env file: {e}");
        return;
    }

    let mut task_set = JoinSet::new();
    task_set.spawn(l1_watcher::start_l1_watcher(store.clone()));
    task_set.spawn(l1_committer::start_l1_commiter(store.clone()));
    task_set.spawn(prover_server::start_prover_server(store.clone()));
    task_set.spawn(start_proposer_server(store.clone()));

    while let Some(res) = task_set.join_next().await {
        match res {
            Ok(Ok(_)) => {}
            Ok(Err(err)) => {
                error!("Error starting Proposer: {err}");
                task_set.abort_all();
                break;
            }
            Err(err) => {
                error!("JoinSet error: {err}");
                task_set.abort_all();
                break;
            }
        };
    }
}

async fn start_proposer_server(store: Store) -> Result<(), ConfigError> {
    let proposer_config = ProposerConfig::from_env()?;
    let engine_config = EngineApiConfig::from_env().map_err(ConfigError::from)?;
    let proposer =
        Proposer::new_from_config(&proposer_config, engine_config).map_err(ConfigError::from)?;

    proposer.run(store.clone()).await;
    Ok(())
}

impl Proposer {
    pub fn new_from_config(
        proposer_config: &ProposerConfig,
        engine_config: EngineApiConfig,
    ) -> Result<Self, ProposerError> {
        let jwt_secret = std::fs::read(&engine_config.jwt_path)?;
        Ok(Self {
            engine_config,
            block_production_interval: proposer_config.interval_ms,
            coinbase_address: proposer_config.coinbase_address,
            jwt_secret,
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
            let current_block_number = store.get_latest_block_number()?;
            store
                .get_canonical_block_hash(current_block_number)?
                .ok_or(ProposerError::StorageDataIsNone)?
        };

        ethrex_dev::block_producer::start_block_producer(
            self.engine_config.rpc_url.clone(),
            self.jwt_secret.clone().into(),
            head_block_hash,
            10,
            self.block_production_interval,
            self.coinbase_address,
        )
        .await?;

        Ok(())
    }
}
