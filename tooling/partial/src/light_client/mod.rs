//! A sync committee light client over the beacon REST API. It vouches for execution block
//! hashes: the only trusted input the rest of the node takes.
//!
//! Trust: the weak-subjectivity checkpoint root the node starts from, and an honest sync
//! committee. The beacon endpoint can stall the client but cannot make it accept a header
//! the committee did not sign.

pub mod beacon_api;
pub mod bls;
pub mod merkle;
pub mod sync;
pub mod types;

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use ethrex_common::H256;
use tokio::sync::watch;
use tracing::{debug, info, warn};

use self::beacon_api::{BeaconApi, BeaconApiError, MAX_UPDATES_PER_REQUEST};
use self::sync::{LightClientSpec, LightClientStore, SyncError, period_at_slot};
use self::types::LightClientHeader;

/// A beacon block the light client vouches for, with the execution block it points at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LcHead {
    /// Slot of the beacon block.
    pub slot: u64,
    /// Root of the beacon block.
    pub beacon_root: H256,
    /// Root of its parent beacon block.
    pub parent_root: H256,
    /// The latest applied execution block as of this beacon block (Gloas), or the block's
    /// own payload (pre-Gloas).
    pub execution_block_hash: H256,
}

impl LcHead {
    fn from_header(header: &LightClientHeader) -> Result<Self, LightClientError> {
        Ok(Self {
            slot: header.beacon.slot,
            beacon_root: header.beacon.root(),
            parent_root: H256(header.beacon.parent_root),
            execution_block_hash: header.execution_block_hash().ok_or(
                LightClientError::NoExecutionHash {
                    slot: header.beacon.slot,
                },
            )?,
        })
    }
}

/// The two heads the client tracks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Heads {
    /// The newest attested block.
    pub optimistic: LcHead,
    /// The newest finalized block.
    pub finalized: LcHead,
}

/// The light client failed.
#[derive(Debug, thiserror::Error)]
pub enum LightClientError {
    /// The beacon API failed.
    #[error(transparent)]
    Api(#[from] BeaconApiError),
    /// A protocol check failed.
    #[error(transparent)]
    Sync(#[from] SyncError),
    /// A header carries no execution block hash.
    #[error("light client header at slot {slot} carries no execution block hash")]
    NoExecutionHash {
        /// The header's slot.
        slot: u64,
    },
}

/// Fetches the bootstrap for `root`. Beacon nodes compute light client data for a newly
/// finalized checkpoint a little after finalizing it, so a 404 is retried; when the node
/// chose the checkpoint itself it also falls back to the checkpoint one epoch earlier.
async fn bootstrap_with_retry(
    api: &BeaconApi,
    root: H256,
    user_supplied: bool,
) -> Result<(H256, types::LightClientBootstrap), LightClientError> {
    const ATTEMPTS: usize = 6;
    let mut root = root;
    let mut last = None;
    for attempt in 0..ATTEMPTS {
        match api.bootstrap(root).await {
            Ok(bootstrap) => return Ok((root, bootstrap)),
            Err(BeaconApiError::Status { status: 404, path }) => {
                warn!(%root, attempt, "beacon node has no light client bootstrap for this root yet");
                last = Some(BeaconApiError::Status { status: 404, path });
                if user_supplied {
                    tokio::time::sleep(Duration::from_secs(5)).await;
                    continue;
                }
                let header = api.header(&format!("{root:#x}")).await?;
                let slot = header.header.message.slot;
                let epoch_start = slot.saturating_sub(slot % sync::SLOTS_PER_EPOCH);
                // The checkpoint of the previous epoch: its first slot, or the newest block
                // before it when that slot was empty.
                let mut candidate = epoch_start.saturating_sub(sync::SLOTS_PER_EPOCH);
                let mut found = None;
                for _ in 0..4 {
                    if let Ok(previous) = api.header(&candidate.to_string()).await {
                        found = Some(H256(previous.root));
                        break;
                    }
                    candidate = candidate.saturating_sub(1);
                }
                match found {
                    Some(previous) => root = previous,
                    None => tokio::time::sleep(Duration::from_secs(5)).await,
                }
            }
            Err(err) => return Err(err.into()),
        }
    }
    Err(last
        .map(LightClientError::Api)
        .unwrap_or(LightClientError::NoExecutionHash { slot: 0 }))
}

/// A running light client.
pub struct LightClient {
    api: BeaconApi,
    spec: LightClientSpec,
    store: LightClientStore,
    checkpoint: LcHead,
}

impl LightClient {
    /// Bootstraps from `checkpoint`, or from the beacon node's finalized block when none is
    /// given (which trusts that node for the starting point), then syncs to the present.
    pub async fn start(api: BeaconApi, checkpoint: Option<H256>) -> Result<Self, LightClientError> {
        let spec = api.light_client_spec().await?;
        let (checkpoint, user_supplied) = match checkpoint {
            Some(root) => (root, true),
            None => {
                let header = api.header("finalized").await?;
                let root = H256(header.root);
                warn!(
                    %root,
                    "no checkpoint configured; trusting the beacon node's finalized block as the \
                     weak subjectivity checkpoint"
                );
                (root, false)
            }
        };
        let (checkpoint, bootstrap) = bootstrap_with_retry(&api, checkpoint, user_supplied).await?;
        let store = LightClientStore::initialize(&spec, checkpoint, bootstrap)?;
        let checkpoint = LcHead::from_header(&store.finalized_header)?;
        info!(
            slot = checkpoint.slot,
            root = %checkpoint.beacon_root,
            "light client bootstrapped"
        );
        let mut client = Self {
            api,
            spec,
            store,
            checkpoint,
        };
        client.sync_to_now().await?;
        Ok(client)
    }

    /// The network constants.
    pub const fn spec(&self) -> &LightClientSpec {
        &self.spec
    }

    /// The checkpoint the client started from.
    pub const fn checkpoint(&self) -> LcHead {
        self.checkpoint
    }

    /// The slot the wall clock is in.
    pub fn current_slot(&self) -> u64 {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        self.spec.slot_at(now)
    }

    /// The heads as of the last processed update.
    pub fn heads(&self) -> Result<Heads, LightClientError> {
        Ok(Heads {
            optimistic: LcHead::from_header(&self.store.optimistic_header)?,
            finalized: LcHead::from_header(&self.store.finalized_header)?,
        })
    }

    /// Fetches and applies the sync committee updates from the store's period to the
    /// current one.
    pub async fn sync_to_now(&mut self) -> Result<(), LightClientError> {
        let current_slot = self.current_slot();
        let current_period = period_at_slot(current_slot);
        let mut period = self.store.period();
        while period <= current_period {
            let count = current_period
                .saturating_sub(period)
                .saturating_add(1)
                .min(MAX_UPDATES_PER_REQUEST);
            let updates = self.api.updates(period, count).await?;
            if updates.is_empty() {
                break;
            }
            let fetched = u64::try_from(updates.len()).unwrap_or(u64::MAX);
            for update in &updates {
                match self.store.process(&self.spec, update, current_slot) {
                    Ok(progress) => debug!(
                        ?progress,
                        slot = update.attested_header.beacon.slot,
                        "update applied"
                    ),
                    Err(SyncError::Stale) => {}
                    Err(err) => return Err(err.into()),
                }
            }
            if self.store.period() < period.saturating_add(fetched) {
                // The committee for the next period did not become known; stop rather
                // than loop forever.
                if self.store.next_sync_committee.is_none() {
                    break;
                }
            }
            period = period.saturating_add(fetched);
        }
        Ok(())
    }

    /// Applies the latest finality and optimistic updates. Returns the heads afterwards.
    pub async fn poll(&mut self) -> Result<Heads, LightClientError> {
        let current_slot = self.current_slot();
        if period_at_slot(current_slot) > self.store.period() {
            self.sync_to_now().await?;
        }
        match self.api.finality_update().await {
            Ok(update) => match self.store.process(&self.spec, &update, current_slot) {
                Ok(_) | Err(SyncError::Stale) => {}
                Err(err) => warn!(%err, "finality update refused"),
            },
            Err(err) => debug!(%err, "no finality update"),
        }
        match self.api.optimistic_update().await {
            Ok(update) => match self.store.process(&self.spec, &update, current_slot) {
                Ok(_) | Err(SyncError::Stale) => {}
                Err(err) => warn!(%err, "optimistic update refused"),
            },
            Err(err) => debug!(%err, "no optimistic update"),
        }
        if self.store.force_update(current_slot) {
            warn!("light client forced an update after a period without finality");
        }
        self.heads()
    }

    /// Runs the client, polling once per slot, publishing heads on the returned channel
    /// until `shutdown` resolves. The initial value is the heads at start.
    pub fn spawn(
        mut self,
        shutdown: tokio_util::sync::CancellationToken,
    ) -> Result<(watch::Receiver<Heads>, tokio::task::JoinHandle<()>), LightClientError> {
        let (tx, rx) = watch::channel(self.heads()?);
        let interval = Duration::from_secs(self.spec.seconds_per_slot.max(1));
        let handle = tokio::spawn(async move {
            let mut ticker = tokio::time::interval(interval);
            loop {
                tokio::select! {
                    _ = shutdown.cancelled() => break,
                    _ = ticker.tick() => {}
                }
                match self.poll().await {
                    Ok(heads) => {
                        tx.send_if_modified(|current| {
                            if *current != heads {
                                *current = heads;
                                true
                            } else {
                                false
                            }
                        });
                    }
                    Err(err) => warn!(%err, "light client poll failed"),
                }
            }
        });
        Ok((rx, handle))
    }
}
