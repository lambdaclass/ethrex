//! Actor-based subscription manager for WebSocket `eth_subscribe` connections.
//!
//! The `SubscriptionManager` is a GenServer actor that owns all subscription
//! state. It receives `NewHead` messages from block producers / fork choice
//! handlers, and `NewPendingTransaction` messages from the mempool feed (see
//! [`spawn_pending_transactions_feed`]), and fans out notifications to the
//! connected WebSocket clients subscribed to each through per-connection `mpsc`
//! channels. `syncing` subscribers get the current sync status once after
//! subscribing, then each switch between syncing and synced (see
//! [`spawn_syncing_feed`]). All types share [`MAX_TOTAL_SUBSCRIPTIONS`].
//!
//! Using an actor removes the need for a `broadcast` channel and eliminates
//! the "lagged subscriber" problem: when a connection drops, its sender is
//! removed during the next `new_head` fan-out rather than silently accumulating
//! unread messages.

pub use crate::eth::client::{SyncStatus, SyncingStatusRpc};
pub use crate::eth::logs::TopicFilter;
use crate::{
    eth::{
        client::{SYNCED_HEAD_TOLERANCE, SYNCING_HEAD_TOLERANCE, sync_status},
        logs::{collect_receipt_logs, matches_topics},
    },
    rpc::RpcApiContext,
    types::receipt::RpcLog,
    utils::RpcErr,
};
use ethrex_common::{
    H160, H256,
    types::{BlockHeader, BlockNumber},
};
use ethrex_storage::{Store, error::StoreError};
use rand::RngCore;
use serde_json::Value;
use spawned_concurrency::{
    actor,
    error::ActorError,
    protocol,
    tasks::{Actor, ActorRef, ActorStart as _, Context, Handler, Response},
};
use std::{
    collections::{HashMap, HashSet},
    fmt::Write as _,
};
use tokio::sync::{broadcast, mpsc::Sender};
use tokio_util::sync::CancellationToken;
use tracing::{debug, warn};

/// Maximum number of buffered notifications per subscriber.
/// If a subscriber's channel is full (slow WebSocket client), the notification
/// is dropped rather than blocking the actor. Matches Geth's approach of
/// dropping slow clients (Geth uses 20,000; we use a smaller buffer since
/// each notification is already serialized JSON).
pub const SUBSCRIBER_CHANNEL_CAPACITY: usize = 512;

/// Maximum number of active subscriptions allowed per WebSocket connection.
pub const MAX_SUBSCRIPTIONS_PER_CONNECTION: usize = 128;

/// Maximum number of active subscriptions across all connections.
///
/// Bounds the worst-case memory of the actor: each subscriber owns a
/// [`SUBSCRIBER_CHANNEL_CAPACITY`]-slot `mpsc` channel, so the per-connection
/// cap alone (`128`) does not bound total state when many connections are
/// open. `subscribe` returns `None` once this is reached and the calling
/// `eth_subscribe` request fails with an internal error.
pub const MAX_TOTAL_SUBSCRIPTIONS: usize = 10_000;

/// Longest reorg or head jump whose logs are notified; past it, only the new head's are.
pub const MAX_LOGS_ROUTE_BLOCKS: usize = 128;

#[derive(Debug, Clone, Default)]
pub struct LogSubscriptionFilter {
    pub addresses: HashSet<H160>,
    pub topics: Vec<TopicFilter>,
    /// Lowest and highest block whose logs are delivered; `None` for no bound.
    pub from_block: Option<BlockNumber>,
    pub to_block: Option<BlockNumber>,
}

impl LogSubscriptionFilter {
    fn covers(&self, block: BlockNumber) -> bool {
        self.from_block.is_none_or(|from| block >= from)
            && self.to_block.is_none_or(|to| block <= to)
    }

    fn matches(&self, block: BlockNumber, address: &H160, topics: &[H256]) -> bool {
        self.covers(block)
            && (self.addresses.is_empty() || self.addresses.contains(address))
            && matches_topics(topics, &self.topics)
    }
}

/// Actor that manages all active WebSocket subscriptions.
///
/// Each subscription is identified by a hex-encoded string ID and backed by a
/// bounded `Sender<String>` that delivers serialised notification JSON to the
/// corresponding WebSocket write-loop.
pub struct SubscriptionManager {
    store: Store,
    /// `newHeads` subscribers.
    subscribers: HashMap<String, Sender<String>>,
    /// `syncing` subscribers, each with whether it was last told the node is
    /// synced (`None` until its initial status is delivered).
    syncing_subscribers: HashMap<String, (Sender<String>, Option<bool>)>,
    /// `newPendingTransactions` subscribers.
    pending_tx_subscribers: HashMap<String, Sender<String>>,
    /// `logs` subscribers.
    log_subscribers: HashMap<String, (Sender<String>, LogSubscriptionFilter)>,
    last_head: Option<(BlockNumber, H256)>,
}

/// Messages understood by the [`SubscriptionManager`].
#[protocol]
pub trait SubscriptionManagerProtocol: Send + Sync {
    /// Broadcast a new block header to `newHeads` subscribers and its logs to `logs` subscribers.
    ///
    /// The actor handles serialization and hash injection. Callers just
    /// pass the raw `BlockHeader`. Dead subscribers are removed automatically
    /// when their channel is closed.
    fn new_head(&self, header: BlockHeader) -> Result<(), ActorError>;

    /// Broadcast the hash of a transaction added to the mempool to all
    /// `newPendingTransactions` subscribers.
    fn new_pending_transaction(&self, hash: H256) -> Result<(), ActorError>;

    /// Send `status` to the `syncing` subscribers last told otherwise, and to the
    /// ones still owed their initial status. A subscriber whose channel is full
    /// is retried with the next status.
    fn new_sync_status(&self, status: SyncStatus) -> Result<(), ActorError>;

    /// Whether any `syncing` subscriber is registered.
    fn has_syncing_subscribers(&self) -> Response<bool>;

    /// Register a new subscriber.
    ///
    /// Returns `Some(id)` with the subscription ID that the client should use
    /// in subsequent `eth_unsubscribe` calls, or `None` if the global cap
    /// [`MAX_TOTAL_SUBSCRIPTIONS`] has been reached.
    fn subscribe(&self, sender: Sender<String>) -> Response<Option<String>>;

    /// Register a new `syncing` subscriber. Returns its ID like [`Self::subscribe`],
    /// under the same global cap. Its initial status is sent with the next
    /// [`Self::new_sync_status`].
    fn subscribe_syncing(&self, sender: Sender<String>) -> Response<Option<String>>;

    /// Register a new `newPendingTransactions` subscriber. Returns its ID like
    /// [`Self::subscribe`], under the same global cap.
    fn subscribe_pending_transactions(&self, sender: Sender<String>) -> Response<Option<String>>;

    /// Register a new `logs` subscriber that gets the logs matching `filter`.
    /// Returns its ID like [`Self::subscribe`], under the same global cap.
    fn subscribe_logs(
        &self,
        sender: Sender<String>,
        filter: LogSubscriptionFilter,
    ) -> Response<Option<String>>;

    /// Remove a subscriber by ID.
    ///
    /// Returns `true` if the subscription existed and was removed, `false`
    /// otherwise.
    fn unsubscribe(&self, id: String) -> Response<bool>;
}

#[actor(protocol = SubscriptionManagerProtocol)]
impl SubscriptionManager {
    /// Spawn the actor and return a handle.
    pub fn spawn(store: Store) -> ActorRef<SubscriptionManager> {
        SubscriptionManager {
            store,
            subscribers: HashMap::new(),
            syncing_subscribers: HashMap::new(),
            pending_tx_subscribers: HashMap::new(),
            log_subscribers: HashMap::new(),
            last_head: None,
        }
        .start()
    }

    #[send_handler]
    async fn handle_new_head(
        &mut self,
        msg: subscription_manager_protocol::NewHead,
        _ctx: &Context<Self>,
    ) {
        let header = msg.header;
        let block_hash = header.hash();
        if !self.subscribers.is_empty() {
            self.notify_new_head(&header, block_hash);
        }
        self.log_subscribers
            .retain(|_, (sender, _)| !sender.is_closed());
        // A head whose logs could not be read is retried from the same previous head.
        if self.log_subscribers.is_empty() || self.notify_logs(self.last_head, &header).await {
            self.last_head = Some((header.number, block_hash));
        }
    }

    #[send_handler]
    async fn handle_new_pending_transaction(
        &mut self,
        msg: subscription_manager_protocol::NewPendingTransaction,
        _ctx: &Context<Self>,
    ) {
        if self.pending_tx_subscribers.is_empty() {
            return;
        }
        let hash = Value::String(format!("{:#x}", msg.hash));
        fan_out(
            &mut self.pending_tx_subscribers,
            &hash,
            "newPendingTransactions",
        );
    }

    #[send_handler]
    async fn handle_new_sync_status(
        &mut self,
        msg: subscription_manager_protocol::NewSyncStatus,
        _ctx: &Context<Self>,
    ) {
        let synced = matches!(msg.status, SyncStatus::Synced);
        let result = msg.status.to_subscription_value();
        let mut dead_ids: Vec<String> = Vec::new();
        for (sub_id, (sender, last_synced)) in self.syncing_subscribers.iter_mut() {
            if *last_synced == Some(synced) {
                continue;
            }
            match sender.try_send(build_subscription_notification(sub_id, &result)) {
                Ok(()) => *last_synced = Some(synced),
                Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => {
                    dead_ids.push(sub_id.clone());
                }
                Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => {
                    warn!(sub_id = %sub_id, "Subscriber channel full, deferring the sync status");
                }
            }
        }
        for id in dead_ids {
            debug!(sub_id = %id, "Removing closed syncing subscriber");
            self.syncing_subscribers.remove(&id);
        }
    }

    #[request_handler]
    async fn handle_has_syncing_subscribers(
        &mut self,
        _msg: subscription_manager_protocol::HasSyncingSubscribers,
        _ctx: &Context<Self>,
    ) -> bool {
        !self.syncing_subscribers.is_empty()
    }

    #[request_handler]
    async fn handle_subscribe(
        &mut self,
        msg: subscription_manager_protocol::Subscribe,
        _ctx: &Context<Self>,
    ) -> Option<String> {
        if !self.below_cap() {
            return None;
        }
        let id = generate_subscription_id();
        self.subscribers.insert(id.clone(), msg.sender);
        Some(id)
    }

    #[request_handler]
    async fn handle_subscribe_syncing(
        &mut self,
        msg: subscription_manager_protocol::SubscribeSyncing,
        _ctx: &Context<Self>,
    ) -> Option<String> {
        if !self.below_cap() {
            return None;
        }
        let id = generate_subscription_id();
        self.syncing_subscribers
            .insert(id.clone(), (msg.sender, None));
        Some(id)
    }

    #[request_handler]
    async fn handle_subscribe_pending_transactions(
        &mut self,
        msg: subscription_manager_protocol::SubscribePendingTransactions,
        _ctx: &Context<Self>,
    ) -> Option<String> {
        if !self.below_cap() {
            return None;
        }
        let id = generate_subscription_id();
        self.pending_tx_subscribers.insert(id.clone(), msg.sender);
        Some(id)
    }

    #[request_handler]
    async fn handle_subscribe_logs(
        &mut self,
        msg: subscription_manager_protocol::SubscribeLogs,
        _ctx: &Context<Self>,
    ) -> Option<String> {
        if !self.below_cap() {
            return None;
        }
        let id = generate_subscription_id();
        self.log_subscribers
            .insert(id.clone(), (msg.sender, msg.filter));
        Some(id)
    }

    #[request_handler]
    async fn handle_unsubscribe(
        &mut self,
        msg: subscription_manager_protocol::Unsubscribe,
        _ctx: &Context<Self>,
    ) -> bool {
        self.subscribers.remove(&msg.id).is_some()
            || self.syncing_subscribers.remove(&msg.id).is_some()
            || self.pending_tx_subscribers.remove(&msg.id).is_some()
            || self.log_subscribers.remove(&msg.id).is_some()
    }
}

impl SubscriptionManager {
    /// Whether one more subscription fits under [`MAX_TOTAL_SUBSCRIPTIONS`].
    fn below_cap(&self) -> bool {
        if self.subscribers.len()
            + self.syncing_subscribers.len()
            + self.pending_tx_subscribers.len()
            + self.log_subscribers.len()
            >= MAX_TOTAL_SUBSCRIPTIONS
        {
            warn!(
                cap = MAX_TOTAL_SUBSCRIPTIONS,
                "Global subscription cap reached, refusing new subscriber"
            );
            return false;
        }
        true
    }

    fn notify_new_head(&mut self, header: &BlockHeader, block_hash: H256) {
        // Serialize the header and inject the computed block hash.
        let mut header_value = match serde_json::to_value(header) {
            Ok(v) => v,
            Err(e) => {
                warn!("Failed to serialize block header for newHeads: {e}");
                return;
            }
        };
        if let Value::Object(ref mut map) = header_value {
            map.insert(
                "hash".to_string(),
                Value::String(format!("{block_hash:#x}")),
            );
        }

        fan_out(&mut self.subscribers, &header_value, "newHeads");
    }

    /// Returns `false`, having sent nothing, when a block's logs could not be read.
    async fn notify_logs(
        &mut self,
        previous_head: Option<(BlockNumber, H256)>,
        head: &BlockHeader,
    ) -> bool {
        let (removed, added) = match self.logs_route(previous_head, head) {
            Ok(route) => route,
            Err(e) => {
                warn!("Failed to find the blocks for logs notifications: {e}");
                return false;
            }
        };
        let blocks = removed
            .iter()
            .map(|header| (header, true))
            .chain(added.iter().map(|header| (header, false)));
        let mut block_logs = Vec::new();
        for (header, removed) in blocks {
            match self.matching_block_logs(header).await {
                Ok(logs) if !logs.is_empty() => block_logs.push((logs, removed)),
                Ok(_) => {}
                Err(e) => {
                    warn!(
                        block = header.number,
                        "Failed to read logs for logs subscribers: {e:?}"
                    );
                    return false;
                }
            }
        }
        for (logs, removed) in block_logs {
            fan_out_logs(&mut self.log_subscribers, logs, removed);
        }
        true
    }

    async fn matching_block_logs(&self, header: &BlockHeader) -> Result<Vec<RpcLog>, RpcErr> {
        let covered = self
            .log_subscribers
            .values()
            .any(|(_, filter)| filter.covers(header.number));
        if !covered || header.logs_bloom.is_zero() {
            return Ok(Vec::new());
        }
        let hash = header.hash();
        let receipts = self.store.get_receipts_for_block(&hash).await?;
        let matched = receipts
            .iter()
            .flat_map(|receipt| &receipt.logs)
            .any(|log| {
                self.log_subscribers
                    .values()
                    .any(|(_, filter)| filter.matches(header.number, &log.address, &log.topics))
            });
        if !matched {
            return Ok(Vec::new());
        }
        let body = self
            .store
            .get_block_body_by_hash(hash)
            .await?
            .ok_or_else(|| RpcErr::Internal("block body not found".to_string()))?;
        let mut logs = Vec::new();
        collect_receipt_logs(header, &body, &receipts, &HashSet::new(), &[], &mut logs)?;
        Ok(logs)
    }

    /// Blocks `head` removes and adds since `previous_head`, oldest first.
    fn logs_route(
        &self,
        previous_head: Option<(BlockNumber, H256)>,
        head: &BlockHeader,
    ) -> Result<(Vec<BlockHeader>, Vec<BlockHeader>), StoreError> {
        let Some((previous_number, previous_hash)) = previous_head else {
            return Ok((Vec::new(), vec![head.clone()]));
        };
        // Past the limit only a head above the previous one is new.
        let past_limit = || {
            warn!(
                previous = previous_number,
                head = head.number,
                "Logs route past its limit, notifying only the new head's logs"
            );
            let added = if head.number >= previous_number {
                vec![head.clone()]
            } else {
                Vec::new()
            };
            Ok((Vec::new(), added))
        };
        if head.hash() == previous_hash {
            return Ok((Vec::new(), Vec::new()));
        }
        if head.parent_hash == previous_hash {
            return Ok((Vec::new(), vec![head.clone()]));
        }
        if head.number.abs_diff(previous_number) > MAX_LOGS_ROUTE_BLOCKS as u64 {
            return past_limit();
        }
        let Some(mut old) = self.store.get_block_header_by_hash(previous_hash)? else {
            return past_limit();
        };
        let mut new = head.clone();
        let (mut removed, mut added) = (Vec::new(), Vec::new());
        while new.hash() != old.hash() {
            if removed.len() + added.len() == MAX_LOGS_ROUTE_BLOCKS {
                return past_limit();
            }
            let (chain, tip) = if new.number >= old.number {
                (&mut added, &mut new)
            } else {
                (&mut removed, &mut old)
            };
            let Some(parent) = self.store.get_block_header_by_hash(tip.parent_hash)? else {
                return past_limit();
            };
            chain.push(std::mem::replace(tip, parent));
        }
        removed.reverse();
        added.reverse();
        Ok((removed, added))
    }
}

/// One queue entry per block per subscriber, so a busy block fits the channel.
fn fan_out_logs(
    subscribers: &mut HashMap<String, (Sender<String>, LogSubscriptionFilter)>,
    logs: Vec<RpcLog>,
    removed: bool,
) {
    let mut serialized: Vec<Option<String>> = vec![None; logs.len()];
    let mut dead_ids: Vec<String> = Vec::new();

    for (sub_id, (sender, filter)) in subscribers.iter() {
        if sender.is_closed() {
            dead_ids.push(sub_id.clone());
            continue;
        }
        let mut batch = String::new();
        for (log, json) in logs.iter().zip(serialized.iter_mut()) {
            if !filter.matches(log.block_number, &log.log.address, &log.log.topics) {
                continue;
            }
            if json.is_none() {
                let mut log = log.clone();
                log.removed = removed;
                match serde_json::to_string(&log) {
                    Ok(value) => *json = Some(value),
                    Err(e) => {
                        warn!("Failed to serialize log for logs subscribers: {e}");
                        continue;
                    }
                }
            }
            let Some(json) = json else { continue };
            if !batch.is_empty() {
                batch.push('\n');
            }
            let _ = write!(
                batch,
                r#"{{"jsonrpc":"2.0","method":"eth_subscription","params":{{"subscription":"{sub_id}","result":{json}}}}}"#
            );
        }
        if batch.is_empty() {
            continue;
        }
        match sender.try_send(batch) {
            Ok(()) => {}
            Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => {
                dead_ids.push(sub_id.clone());
            }
            Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => {
                warn!(sub_id = %sub_id, "Subscriber channel full, dropping notification");
            }
        }
    }

    for id in dead_ids {
        debug!(sub_id = %id, "Removing closed logs subscriber");
        subscribers.remove(&id);
    }
}

/// Sends `result` to every subscriber in `subscribers`, dropping the ones whose
/// connection closed. `kind` names the subscription type in logs.
fn fan_out(subscribers: &mut HashMap<String, Sender<String>>, result: &Value, kind: &str) {
    let mut dead_ids: Vec<String> = Vec::new();

    for (sub_id, sender) in subscribers.iter() {
        let notification = build_subscription_notification(sub_id, result);
        match sender.try_send(notification) {
            Ok(()) => {}
            Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => {
                dead_ids.push(sub_id.clone());
            }
            Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => {
                warn!(sub_id = %sub_id, "Subscriber channel full, dropping notification");
            }
        }
    }

    for id in dead_ids {
        debug!(sub_id = %id, "Removing closed {kind} subscriber");
        subscribers.remove(&id);
    }
}

/// Forwards the hash of every transaction the mempool announces to `manager`'s
/// `newPendingTransactions` subscribers, until `cancel_token` is cancelled.
pub fn spawn_pending_transactions_feed(
    mut new_transactions: broadcast::Receiver<H256>,
    manager: ActorRef<SubscriptionManager>,
    cancel_token: CancellationToken,
) {
    tokio::spawn(async move {
        loop {
            tokio::select! {
                received = new_transactions.recv() => match received {
                    Ok(hash) => {
                        if manager.new_pending_transaction(hash).is_err() {
                            break;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(skipped)) => {
                        warn!(skipped, "newPendingTransactions feed fell behind the mempool");
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                },
                _ = cancel_token.cancelled() => break,
            }
        }
    });
}

/// How long the syncing feed waits before retrying a failed sync status read.
const SYNC_STATUS_RETRY_DELAY: std::time::Duration = std::time::Duration::from_secs(1);

/// Recomputes the sync status for `syncing` subscribers whenever the store
/// signals that one of its inputs changed, skipping it while there are none.
/// Signals that arrive during a recompute collapse into one more, so a state
/// that lasts only between two of them may go unreported. A no-op without a
/// sync manager.
pub fn spawn_syncing_feed(
    context: &RpcApiContext,
    manager: ActorRef<SubscriptionManager>,
    cancel_token: CancellationToken,
) {
    let Some(syncer) = context.syncer.clone() else {
        return;
    };
    let storage = context.storage.clone();
    let blockchain = context.blockchain.clone();
    let mut changes = storage.sync_status_changes();
    tokio::spawn(async move {
        let mut synced = false;
        let mut failing = false;
        loop {
            tokio::select! {
                changed = changes.changed() => if changed.is_err() {
                    break;
                },
                _ = cancel_token.cancelled() => break,
            }
            match manager.has_syncing_subscribers().await {
                Ok(true) => {}
                Ok(false) => {
                    synced = false;
                    continue;
                }
                Err(_) => break,
            }
            let tolerance = if synced {
                SYNCING_HEAD_TOLERANCE
            } else {
                SYNCED_HEAD_TOLERANCE
            };
            match sync_status(&storage, &blockchain, &syncer, tolerance).await {
                Ok(status) => {
                    failing = false;
                    synced = matches!(status, SyncStatus::Synced);
                    if manager.new_sync_status(status).is_err() {
                        break;
                    }
                }
                Err(e) => {
                    if !failing {
                        warn!(
                            "Failed to read the sync status for syncing subscribers, retrying: {e:?}"
                        );
                    }
                    failing = true;
                    changes.mark_changed();
                    tokio::select! {
                        _ = tokio::time::sleep(SYNC_STATUS_RETRY_DELAY) => {}
                        _ = cancel_token.cancelled() => break,
                    }
                }
            }
        }
    });
}

/// Build the standard Ethereum subscription notification envelope.
///
/// `result` is cloned per subscriber — cheap relative to re-serializing the
/// header. Using `serde_json::json!` avoids hand-rolled string interpolation,
/// which would silently produce malformed JSON if `sub_id` or the result ever
/// contained unescaped characters.
fn build_subscription_notification(sub_id: &str, result: &Value) -> String {
    serde_json::json!({
        "jsonrpc": "2.0",
        "method": "eth_subscription",
        "params": {
            "subscription": sub_id,
            "result": result,
        },
    })
    .to_string()
}

/// Generate a random hex subscription ID (16 bytes / 128 bits).
fn generate_subscription_id() -> String {
    let mut bytes = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut bytes);
    format!("0x{}", hex::encode(bytes))
}
