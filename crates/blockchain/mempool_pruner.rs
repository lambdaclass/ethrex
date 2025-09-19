use std::{
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use crate::{Blockchain, error::MempoolError};
use ethrex_common::types::MempoolTransaction;
use ethrex_storage::error::StoreError;
use spawned_concurrency::{
    messages::Unused,
    tasks::{CastResponse, GenServer, GenServerHandle, send_interval},
};
use tracing::{debug, info, warn};

// Amount of seconds after which we prune old entries from mempool (We should fine tune this)
const PRUNE_WAIT_TIME_SECS: u128 = 300; // 5 minutes

// Amount of seconds between each prune
const PRUNE_INTERVAL_SECS: u64 = 300; // 5 minutes

// Number of microseconds in a second
const MICROS_PER_SECOND: u128 = 1_000_000;

#[derive(Debug, Clone)]
pub struct MempoolPruner {
    blockchain: Arc<Blockchain>,
}

#[derive(Debug, Clone)]
pub enum InMessage {
    Prune,
}

#[derive(Debug, Clone)]
pub enum OutMessage {
    Done,
}

impl MempoolPruner {
    pub async fn spawn(
        blockchain: Arc<Blockchain>,
    ) -> Result<GenServerHandle<MempoolPruner>, MempoolPrunerError> {
        info!("Starting Mempool Pruner");

        let state = MempoolPruner { blockchain };

        let server = state.clone().start();

        send_interval(
            Duration::from_secs(PRUNE_INTERVAL_SECS),
            server.clone(),
            InMessage::Prune,
        );

        Ok(server)
    }

    pub fn prune(&self) -> Result<(), MempoolPrunerError> {
        let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_micros();
        let filter_fn = |tx: &MempoolTransaction| {
            now.saturating_sub(tx.time()) > PRUNE_WAIT_TIME_SECS * MICROS_PER_SECOND
        };
        let mempool_hashes = self
            .blockchain
            .mempool
            .get_hashes_with_filter_fn(&filter_fn)?;
        for hash in &mempool_hashes {
            self.blockchain.remove_transaction_from_pool(hash)?;
        }
        Ok(())
    }
}

impl GenServer for MempoolPruner {
    type CallMsg = Unused;
    type CastMsg = InMessage;
    type OutMsg = OutMessage;
    type Error = MempoolPrunerError;

    async fn handle_cast(
        &mut self,
        message: Self::CastMsg,
        _handle: &spawned_concurrency::tasks::GenServerHandle<Self>,
    ) -> CastResponse {
        match message {
            Self::CastMsg::Prune => {
                debug!(received = "Prune");
                let _ = self.prune().inspect_err(|e| {
                    warn!(error = %e, "Error pruning mempool");
                });
                CastResponse::NoReply
            }
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum MempoolPrunerError {
    #[error(transparent)]
    StoreError(#[from] StoreError),
    #[error(transparent)]
    MempoolError(#[from] MempoolError),
    #[error(transparent)]
    SystemTimeError(#[from] std::time::SystemTimeError),
}
