//! Actor-based subscription manager for WebSocket `eth_subscribe` connections.
//!
//! The `SubscriptionManager` is a GenServer actor that owns all subscription
//! state. It receives `NewHead` messages from block producers / fork choice
//! handlers and fans out notifications to all connected WebSocket clients
//! through per-connection `mpsc` channels.
//!
//! Using an actor removes the need for a `broadcast` channel and eliminates
//! the "lagged subscriber" problem: when a connection drops, its sender is
//! removed during the next `new_head` fan-out rather than silently accumulating
//! unread messages.

use ethrex_common::types::BlockHeader;
use rand::RngCore;
use serde_json::Value;
use spawned_concurrency::{
    actor,
    error::ActorError,
    protocol,
    tasks::{Actor, ActorRef, ActorStart as _, Context, Handler, Response},
};
use std::collections::HashMap;
use tokio::sync::mpsc::Sender;
use tracing::{debug, warn};

/// Maximum number of buffered notifications per subscriber.
/// If a subscriber's channel is full (slow WebSocket client), the notification
/// is dropped rather than blocking the actor. Matches Geth's approach of
/// dropping slow clients (Geth uses 20,000; we use a smaller buffer since
/// each notification is already serialized JSON).
pub const SUBSCRIBER_CHANNEL_CAPACITY: usize = 512;

/// Maximum number of active subscriptions allowed per WebSocket connection.
pub const MAX_SUBSCRIPTIONS_PER_CONNECTION: usize = 128;

/// Actor that manages all active WebSocket subscriptions.
///
/// Each subscription is identified by a hex-encoded string ID and backed by a
/// bounded `Sender<String>` that delivers serialised notification JSON to the
/// corresponding WebSocket write-loop.
#[derive(Default)]
pub struct SubscriptionManager {
    subscribers: HashMap<String, Sender<String>>,
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

    /// Register a new subscriber.
    ///
    /// Returns the subscription ID that the client should use in subsequent
    /// `eth_unsubscribe` calls.
    fn subscribe(&self, sender: Sender<String>) -> Response<String>;

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

        // Serialize the header result once; each subscriber gets its own
        // notification envelope with a different subscription ID.
        let result_json = header_value.to_string();

        let mut dead_ids: Vec<String> = Vec::new();

        for (sub_id, sender) in &self.subscribers {
            let notification = build_subscription_notification(sub_id, &result_json);
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
            debug!(sub_id = %id, "Removing closed newHeads subscriber");
            self.subscribers.remove(&id);
        }
    }

    #[request_handler]
    async fn handle_subscribe(
        &mut self,
        msg: subscription_manager_protocol::Subscribe,
        _ctx: &Context<Self>,
    ) -> String {
        let id = generate_subscription_id();
        self.subscribers.insert(id.clone(), msg.sender);
        id
    }

    #[request_handler]
    async fn handle_unsubscribe(
        &mut self,
        msg: subscription_manager_protocol::Unsubscribe,
        _ctx: &Context<Self>,
    ) -> bool {
        self.subscribers.remove(&msg.id).is_some()
    }
}

/// Build the standard Ethereum subscription notification envelope.
///
/// Takes a pre-serialized `result_json` string to avoid re-serializing the
/// header for every subscriber during fan-out.
fn build_subscription_notification(sub_id: &str, result_json: &str) -> String {
    format!(
        r#"{{"jsonrpc":"2.0","method":"eth_subscription","params":{{"subscription":"{sub_id}","result":{result_json}}}}}"#
    )
}

/// Generate a random hex subscription ID (16 bytes / 128 bits).
fn generate_subscription_id() -> String {
    let mut bytes = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut bytes);
    format!("0x{}", hex::encode(bytes))
}
