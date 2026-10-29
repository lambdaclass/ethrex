//! Actor-based subscription manager for WebSocket `eth_subscribe` connections.
//!
//! The `SubscriptionManager` is a GenServer actor that owns all subscription
//! state. It receives `NewHead` messages from block producers / fork choice
//! handlers, and `NewPendingTransaction` messages from the mempool feed (see
//! [`spawn_pending_transactions_feed`]), and fans out notifications to the
//! connected WebSocket clients subscribed to each through per-connection `mpsc`
//! channels.
//!
//! Using an actor removes the need for a `broadcast` channel and eliminates
//! the "lagged subscriber" problem: when a connection drops, its sender is
//! removed during the next `new_head` fan-out rather than silently accumulating
//! unread messages.

use ethrex_common::{H256, types::BlockHeader};
use rand::RngCore;
use serde_json::Value;
use spawned_concurrency::{
    actor,
    error::ActorError,
    protocol,
    tasks::{Actor, ActorRef, ActorStart as _, Context, Handler, Response},
};
use std::collections::HashMap;
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

/// Actor that manages all active WebSocket subscriptions.
///
/// Each subscription is identified by a hex-encoded string ID and backed by a
/// bounded `Sender<String>` that delivers serialised notification JSON to the
/// corresponding WebSocket write-loop.
#[derive(Default)]
pub struct SubscriptionManager {
    /// `newHeads` subscribers.
    subscribers: HashMap<String, Sender<String>>,
    /// `newPendingTransactions` subscribers.
    pending_tx_subscribers: HashMap<String, Sender<String>>,
}

/// Messages understood by the [`SubscriptionManager`].
#[protocol]
pub trait SubscriptionManagerProtocol: Send + Sync {
    /// Broadcast a new block header to all `newHeads` subscribers.
    ///
    /// The actor handles serialization and hash injection. Callers just
    /// pass the raw `BlockHeader`. Dead subscribers are removed automatically
    /// when their channel is closed.
    fn new_head(&self, header: BlockHeader) -> Result<(), ActorError>;

    /// Broadcast the hash of a transaction added to the mempool to all
    /// `newPendingTransactions` subscribers.
    fn new_pending_transaction(&self, hash: H256) -> Result<(), ActorError>;

    /// Register a new subscriber.
    ///
    /// Returns `Some(id)` with the subscription ID that the client should use
    /// in subsequent `eth_unsubscribe` calls, or `None` if the global cap
    /// [`MAX_TOTAL_SUBSCRIPTIONS`] has been reached.
    fn subscribe(&self, sender: Sender<String>) -> Response<Option<String>>;

    /// Register a new `newPendingTransactions` subscriber. Returns its ID like
    /// [`Self::subscribe`], under the same global cap.
    fn subscribe_pending_transactions(&self, sender: Sender<String>) -> Response<Option<String>>;

    /// Remove a subscriber by ID.
    ///
    /// Returns `true` if the subscription existed and was removed, `false`
    /// otherwise.
    fn unsubscribe(&self, id: String) -> Response<bool>;
}

#[actor(protocol = SubscriptionManagerProtocol)]
impl SubscriptionManager {
    /// Spawn the actor and return a handle.
    pub fn spawn() -> ActorRef<SubscriptionManager> {
        SubscriptionManager::default().start()
    }

    #[send_handler]
    async fn handle_new_head(
        &mut self,
        msg: subscription_manager_protocol::NewHead,
        _ctx: &Context<Self>,
    ) {
        if self.subscribers.is_empty() {
            return;
        }

        // Serialize the header and inject the computed block hash.
        let header = msg.header;
        let block_hash = header.hash();
        let mut header_value = match serde_json::to_value(&header) {
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
    async fn handle_unsubscribe(
        &mut self,
        msg: subscription_manager_protocol::Unsubscribe,
        _ctx: &Context<Self>,
    ) -> bool {
        self.subscribers.remove(&msg.id).is_some()
            || self.pending_tx_subscribers.remove(&msg.id).is_some()
    }
}

impl SubscriptionManager {
    /// Whether one more subscription fits under [`MAX_TOTAL_SUBSCRIPTIONS`].
    fn below_cap(&self) -> bool {
        if self.subscribers.len() + self.pending_tx_subscribers.len() >= MAX_TOTAL_SUBSCRIPTIONS {
            warn!(
                cap = MAX_TOTAL_SUBSCRIPTIONS,
                "Global subscription cap reached, refusing new subscriber"
            );
            return false;
        }
        true
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
