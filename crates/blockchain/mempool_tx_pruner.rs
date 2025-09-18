use std::{
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use crate::{Blockchain, error::MempoolError};
use ethrex_storage::error::StoreError;
use spawned_concurrency::{
    messages::Unused,
    tasks::{CastResponse, GenServer, GenServerHandle, send_interval},
};
use tracing::{debug, info};

// Amount of seconds after which we prune old entries from mempool (We should fine tune this)
const PRUNE_WAIT_TIME_SECS: u128 = 300; // 5 minutes

// Amount of seconds between each prune
const PRUNE_INTERVAL_SECS: u64 = 300; // 5 minutes

#[derive(Debug, Clone)]
pub struct MempoolTxPruner {
    blockchain: Arc<Blockchain>,
}

#[derive(Debug, Clone)]
pub enum InMessage {
    PruneMempool,
}

#[derive(Debug, Clone)]
pub enum OutMessage {
    Done,
}

impl MempoolTxPruner {
    pub async fn spawn(
        blockchain: Arc<Blockchain>,
    ) -> Result<GenServerHandle<MempoolTxPruner>, MempoolTxPrunerError> {
        info!("Starting Transaction Broadcaster");

        let state = MempoolTxPruner { blockchain };

        let server = state.clone().start();

        send_interval(
            Duration::from_secs(PRUNE_INTERVAL_SECS),
            server.clone(),
            InMessage::PruneMempool,
        );

        Ok(server)
    }

    pub fn prune_mempool(&self) -> Result<(), MempoolTxPrunerError> {
        let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_micros();
        let mempool_txs = self.blockchain.mempool.mempool_content()?;
        for tx in &mempool_txs {
            if now.saturating_sub(tx.time()) > PRUNE_WAIT_TIME_SECS * 1_000_000 {
                self.blockchain.remove_transaction_from_pool(&tx.hash())?;
            }
        }
        Ok(())
    }
}

impl GenServer for MempoolTxPruner {
    type CallMsg = Unused;
    type CastMsg = InMessage;
    type OutMsg = OutMessage;
    type Error = MempoolTxPrunerError;

    async fn handle_cast(
        &mut self,
        message: Self::CastMsg,
        _handle: &spawned_concurrency::tasks::GenServerHandle<Self>,
    ) -> CastResponse {
        match message {
            Self::CastMsg::PruneMempool => {
                debug!(received = "PruneMempool");
                let _ = self.prune_mempool().inspect_err(|e| {
                    debug!(error = %e, "Error pruning mempool transactions");
                });
                CastResponse::NoReply
            }
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum MempoolTxPrunerError {
    #[error(transparent)]
    StoreError(#[from] StoreError),
    #[error(transparent)]
    MempoolError(#[from] MempoolError),
    #[error(transparent)]
    SystemTimeError(#[from] std::time::SystemTimeError),
}
