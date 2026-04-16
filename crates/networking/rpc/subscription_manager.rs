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

use serde_json::Value;
use spawned_concurrency::{
    actor,
    error::ActorError,
    protocol,
    tasks::{Actor, ActorRef, ActorStart as _, Context, Handler, Response},
};
use std::collections::HashMap;
use tokio::sync::mpsc::UnboundedSender;
use tracing::debug;

/// Actor that manages all active WebSocket subscriptions.
///
/// Each subscription is identified by a hex-encoded string ID and backed by an
/// `UnboundedSender<String>` that delivers serialised notification JSON to the
/// corresponding WebSocket write-loop.
pub struct SubscriptionManager {
    subscribers: HashMap<String, UnboundedSender<String>>,
    next_id: u64,
}

impl Default for SubscriptionManager {
    fn default() -> Self {
        Self {
            subscribers: HashMap::new(),
            next_id: 1,
        }
    }
}

/// Messages understood by the [`SubscriptionManager`].
#[protocol]
pub trait SubscriptionManagerProtocol: Send + Sync {
    /// Broadcast a new block header to all `newHeads` subscribers.
    ///
    /// This is a fire-and-forget message; dead subscribers are removed
    /// automatically when their channel is closed.
    fn new_head(&self, header: Value) -> Result<(), ActorError>;

    /// Register a new subscriber.
    ///
    /// Returns the subscription ID that the client should use in subsequent
    /// `eth_unsubscribe` calls.
    fn subscribe(&self, sender: UnboundedSender<String>) -> Response<String>;

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
        let header = msg.header;
        let mut dead_ids: Vec<String> = Vec::new();

        for (sub_id, sender) in &self.subscribers {
            let notification = build_subscription_notification(sub_id, header.clone());
            if sender.send(notification).is_err() {
                // The receiver (WebSocket write-loop) has been dropped, so the
                // connection is closed. Remove the subscriber.
                dead_ids.push(sub_id.clone());
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
        let id = format!("0x{:016x}", self.next_id);
        self.next_id += 1;
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
pub fn build_subscription_notification(sub_id: &str, result: Value) -> String {
    serde_json::json!({
        "jsonrpc": "2.0",
        "method": "eth_subscription",
        "params": {
            "subscription": sub_id,
            "result": result,
        }
    })
    .to_string()
}
