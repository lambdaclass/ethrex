#[cfg(feature = "l2")]
use crate::rlpx::l2::{
    PERIODIC_BATCH_BROADCAST_INTERVAL, PERIODIC_BLOCK_BROADCAST_INTERVAL,
    l2_connection::{
        self, L2Cast, L2ConnState, handle_based_capability_message, handle_l2_broadcast,
    },
};
use crate::utils::{node_id, public_key_from_signing_key};
use crate::{
    backend,
    metrics::METRICS,
    network::P2PContext,
    peer_table::{PeerTable, PeerTableServerProtocol as _},
    rlpx::{
        Message,
        connection::{codec::RLPxCodec, handshake},
        error::PeerConnectionError,
        eth::{
            block_access_lists::{BlockAccessLists, GetBlockAccessLists},
            blocks::{BlockBodies, BlockHeaders},
            cells::{
                CellsResponseError, GET_CELLS_SOFT_LIMIT_HASHES, GetCells, MAX_CELLS_SERVED,
                cells_per_hash,
            },
            eth72::{
                status::StatusMessage72,
                transactions::{NewPooledTransactionHashes72, PooledTransactions72},
            },
            receipts::{
                GetReceipts68, GetReceipts70, MAX_RECEIPTS_REQUEST_HASHES, Receipts68, Receipts69,
                Receipts70, SOFT_RESPONSE_LIMIT,
            },
            status::{StatusMessage68, StatusMessage69, StatusMessage70, StatusMessage71},
            transactions::{
                GetPooledTransactions, MAX_POOLED_RESPONSE_BYTES, NewPooledTransactionHashes,
            },
            update::BlockRangeUpdate,
        },
        message::{EthCapVersion, SnapCapVersion},
        p2p::{
            self, Capability, DisconnectMessage, DisconnectReason, PingMessage, PongMessage,
            SUPPORTED_ETH_CAPABILITIES, advertised_snap_capabilities,
        },
        snap::{Snap2BlockAccessLists, Snap2GetBlockAccessLists, TrieNodes},
    },
    snap::constants::{BAL_MAX_REQUEST_HASHES, BAL_RESPONSE_SOFT_CAP_BYTES},
    snap::{
        process_account_range_request, process_byte_codes_request, process_storage_ranges_request,
        process_trie_nodes_request,
    },
    tx_broadcaster::{TxBroadcaster, TxBroadcasterProtocol as _, send_tx_hashes},
    types::Node,
};
use ethrex_blockchain::{
    Blockchain,
    sampling::{is_provider_role, pick_random_extra_column},
};
use ethrex_common::H256;
#[cfg(feature = "l2")]
use ethrex_common::types::Transaction;
use ethrex_common::types::block_access_list::BlockAccessList;
use ethrex_common::types::{P2PTransaction, Receipt};
use ethrex_crypto::NativeCrypto;
use ethrex_rlp::encode::RLPEncode;
use ethrex_storage::{Store, error::StoreError};
use ethrex_trie::TrieError;
use futures::{SinkExt as _, Stream, stream::SplitSink};
use rand::random;
use rustc_hash::FxHashMap;
use secp256k1::{PublicKey, SecretKey};
use spawned_concurrency::{
    actor,
    error::ActorError,
    protocol,
    tasks::{Actor, ActorRef, ActorStart as _, Context, Handler, send_interval, spawn_listener},
};
use spawned_rt::tasks::BroadcastStream;
use std::{
    collections::{HashMap, HashSet},
    net::SocketAddr,
    sync::{Arc, RwLock},
    time::{Duration, Instant},
};
use tokio::{
    net::TcpStream,
    sync::{broadcast, oneshot},
    task::{self, Id},
};
use tokio_stream::StreamExt;
use tokio_util::codec::Framed;
use tracing::{debug, error, trace, warn};

const PING_INTERVAL: Duration = Duration::from_secs(10);
const BLOCK_RANGE_UPDATE_INTERVAL: Duration = Duration::from_secs(60);
const INFLIGHT_TX_SWEEP_INTERVAL: Duration = Duration::from_secs(15);
const INFLIGHT_TX_TIMEOUT: Duration = Duration::from_secs(30);
/// Max time a single outbound frame may take to flush to a peer's socket before we
/// consider the peer wedged and drop it. Without this bound, a slow/half-dead peer
/// (TCP send window full, never draining) blocks the connection actor's serial drain
/// indefinitely while its unbounded mailbox keeps growing — the timer/broadcast
/// producers below enqueue regardless of drain progress — leaking memory that is
/// never reclaimed (the actor cannot be stopped while a handler is wedged in `.await`).
const OUTBOUND_SEND_TIMEOUT: Duration = Duration::from_secs(30);
/// Max time the RLPx handshake may take before we abandon the connection. Without
/// this, an inbound peer that opens TCP and then stalls before/at the auth read
/// parks a connection actor + socket indefinitely (the handshake runs inside
/// `started()` with no timeout), so established sockets accumulate far beyond the
/// peer target.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
/// If a peer fails to answer this many consecutive pings, consider it dead and drop it.
/// Catches application-dead-but-TCP-alive peers that would otherwise never be removed.
const MAX_MISSED_PONGS: u8 = 3;
/// Capacity of the per-connection bounded outbound queue. The writer task drains this into
/// the socket; if a peer can't keep up the queue fills and the peer is dropped, so outbound
/// buffering per connection is bounded to this many messages instead of growing unbounded.
const OUTBOUND_QUEUE_CAP: usize = 1024;
/// How often to flush buffered transaction hash requests into a single
/// batched GetPooledTransactions message.
const TX_REQUEST_BATCH_INTERVAL: Duration = Duration::from_millis(50);
/// Fixed (tumbling) time window for incoming request rate limiting.
const SERVE_REQUEST_WINDOW: Duration = Duration::from_secs(60);
/// Maximum number of data-serving requests allowed per peer within the rate-limit window.
const MAX_SERVE_REQUESTS_PER_WINDOW: u64 = 500;
/// Number of transactions sent to a peer before checking for leeching behaviour.
const LEECH_TX_SENT_THRESHOLD: u64 = 10_000;
/// Maximum number of transaction hashes a peer may announce within one rate-limit window
/// (`SERVE_REQUEST_WINDOW`): on average one full announcement, the devp2p soft limit of 4096
/// hashes, per second. A peer dumping its whole pool on connect stays far below it.
const MAX_ANNOUNCED_HASHES_PER_WINDOW: u64 = 4096 * SERVE_REQUEST_WINDOW.as_secs();
/// Strikes at which a peer that keeps answering `GetPooledTransactions` with little of what it
/// announced is dropped. A short answer (see `is_short_tx_answer`) adds one and any other answer
/// takes one off, so an honest peer that now and then no longer holds what it announced (the tx
/// was included in a block meanwhile, which devp2p allows) never gets there.
const MAX_SHORT_TX_ANSWER_STRIKES: u32 = 10;

pub(crate) type PeerConnBroadcastSender = broadcast::Sender<(tokio::task::Id, Arc<Message>)>;

#[protocol]
pub trait PeerConnectionServerProtocol: Send + Sync {
    fn incoming_message(&self, message: Message) -> Result<(), ActorError>;
    fn outgoing_message(&self, message: Message) -> Result<(), ActorError>;
    fn outgoing_request(
        &self,
        message: Message,
        sender: Arc<oneshot::Sender<Message>>,
    ) -> Result<(), ActorError>;
    fn request_timeout(&self, id: u64) -> Result<(), ActorError>;
    fn send_ping(&self) -> Result<(), ActorError>;
    fn block_range_update(&self) -> Result<(), ActorError>;
    fn broadcast_message(&self, task_id: Id, msg: Arc<Message>) -> Result<(), ActorError>;
    fn sweep_inflight_txs(&self) -> Result<(), ActorError>;
    fn flush_pending_tx_requests(&self) -> Result<(), ActorError>;
    fn enqueue_tx_requests(
        &self,
        announcement: NewPooledTransactionHashes,
        hashes: Vec<H256>,
    ) -> Result<(), ActorError>;
}

#[cfg(feature = "l2")]
#[derive(Clone)]
pub struct L2Message {
    pub msg: L2Cast,
}

#[cfg(feature = "l2")]
impl spawned_concurrency::message::Message for L2Message {
    type Result = ();
}

#[derive(Clone, Debug)]
pub struct PeerConnection {
    handle: ActorRef<PeerConnectionServer>,
}

/// Identifies one RLPx connection. A node can have more than one connection at a time
/// (a duplicate dial, or a reconnect before the old connection closes), and the peer
/// table uses this to tell the connection it lists for the node from the others.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ConnectionId(u64);

impl ConnectionId {
    /// An id no other connection in this process has.
    pub(crate) fn next() -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        Self(NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed))
    }
}

impl PeerConnection {
    pub fn spawn_as_receiver(
        context: P2PContext,
        peer_addr: SocketAddr,
        stream: TcpStream,
        admission_permit: tokio::sync::OwnedSemaphorePermit,
    ) -> PeerConnection {
        let state = ConnectionState::Receiver(Receiver {
            context,
            peer_addr,
            stream: Arc::new(stream),
        });
        let connection = PeerConnectionServer {
            state,
            _admission_permit: Some(admission_permit),
        };
        Self {
            handle: connection.start(),
        }
    }

    pub fn spawn_as_initiator(context: P2PContext, node: &Node) -> PeerConnection {
        let state = ConnectionState::Initiator(Initiator {
            context,
            node: node.clone(),
        });
        // Outbound dials are not admission-capped (we initiate them); inbound is the attack surface.
        let connection = PeerConnectionServer {
            state,
            _admission_permit: None,
        };
        Self {
            handle: connection.start(),
        }
    }

    pub async fn outgoing_message(&mut self, message: Message) -> Result<(), PeerConnectionError> {
        self.handle
            .outgoing_message(message)
            .map_err(|err| PeerConnectionError::InternalError(err.to_string()))
    }

    /// Queue tx hashes (with the originating announcement metadata) to be
    /// requested on the next flush tick. Used as a fallback when an in-flight
    /// request on another peer fails.
    pub fn enqueue_tx_requests(
        &self,
        announcement: NewPooledTransactionHashes,
        hashes: Vec<H256>,
    ) -> Result<(), PeerConnectionError> {
        self.handle
            .enqueue_tx_requests(announcement, hashes)
            .map_err(|err| PeerConnectionError::InternalError(err.to_string()))
    }

    pub async fn outgoing_request(
        &mut self,
        message: Message,
        timeout: Duration,
    ) -> Result<Message, PeerConnectionError> {
        let id = message
            .request_id()
            .expect("Cannot wait on request without id");
        let (oneshot_tx, oneshot_rx) = oneshot::channel::<Message>();

        self.handle
            .outgoing_request(message, Arc::new(oneshot_tx))
            .map_err(|err| PeerConnectionError::InternalError(err.to_string()))?;

        // Wait for the response or timeout. This blocks the calling task (and not the ConnectionServer task)
        match tokio::time::timeout(timeout, oneshot_rx).await {
            Ok(Ok(response)) => Ok(response),
            Ok(Err(error)) => Err(PeerConnectionError::RecvError(error.to_string())),
            Err(_timeout) => {
                // Notify timeout on request id
                self.handle
                    .request_timeout(id)
                    .map_err(|err| PeerConnectionError::InternalError(err.to_string()))?;
                // Return timeout error
                Err(PeerConnectionError::Timeout)
            }
        }
    }
}

#[cfg(test)]
impl PeerConnection {
    /// A handle whose actor never completes a handshake. Enough for the peer
    /// table to register and rank the peer, never for sending it anything.
    pub(crate) fn unestablished() -> PeerConnection {
        let connection = PeerConnectionServer {
            state: ConnectionState::HandshakeFailed,
            _admission_permit: None,
        };
        Self {
            handle: connection.start(),
        }
    }
}

#[derive(Debug)]
pub struct Initiator {
    pub(crate) context: P2PContext,
    pub(crate) node: Node,
}

#[derive(Debug)]
pub struct Receiver {
    pub(crate) context: P2PContext,
    pub(crate) peer_addr: SocketAddr,
    pub(crate) stream: Arc<TcpStream>,
}

/// One announced transaction as flattened for an eth/72 `GetPooledTransactions`:
/// hash, type, announced size and the availability its announcer claimed.
type AnnouncedTx = (H256, u8, usize, Option<u128>);

/// EIP-8070 role this node took for the hashes in one `GetPooledTransactions`
/// request, deciding what happens once the bodies arrive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BlobFetchRole {
    /// Full blob payload is fetched alongside the body, so nothing follows.
    Provider,
    /// Body only; custody-aligned cells are requested once the body validates.
    Sampler,
}

/// A per-connection rate-limit counter over a fixed (tumbling) window of
/// `SERVE_REQUEST_WINDOW`: hits are counted until the window is over, then a new one starts.
#[derive(Debug)]
pub(crate) struct WindowCounter {
    start: Instant,
    count: u64,
}

impl WindowCounter {
    pub(crate) fn new() -> Self {
        Self {
            start: Instant::now(),
            count: 0,
        }
    }

    /// Counts `hits` in the current window, starting a new one first if it is over, and returns
    /// whether the window's count is still within `limit`.
    fn hit(&mut self, hits: u64, limit: u64) -> bool {
        let now = Instant::now();
        if now.duration_since(self.start) >= SERVE_REQUEST_WINDOW {
            self.start = now;
            self.count = 0;
        }
        self.count = self.count.saturating_add(hits);
        self.count <= limit
    }

    /// The hits counted in the current window.
    fn count(&self) -> u64 {
        self.count
    }
}

#[derive(Debug)]
pub struct Established {
    pub(crate) signer: SecretKey,
    // Bounded outbound queue to the per-connection writer task (which owns the socket sink).
    // A bounded queue + dropping the peer on overflow keeps per-connection outbound buffering
    // bounded instead of letting the actor mailbox grow when a peer can't keep up. The receiving
    // part of the TcpStream is owned by the stream listen loop task. See `spawn_outbound_writer`.
    pub(crate) outbound_tx: tokio::sync::mpsc::Sender<Message>,
    /// Set by the writer task when it exits because a single send exceeded `OUTBOUND_SEND_TIMEOUT`
    /// (a wedged peer), so `send` can surface `OutboundSendTimeout` once the bounded queue closes.
    pub(crate) outbound_writer_timed_out: std::sync::Arc<std::sync::atomic::AtomicBool>,
    pub(crate) node: Node,
    // Whether the remote peer initiated this connection (we acted as Receiver).
    // `false` when we initiated it (we acted as Initiator).
    pub(crate) is_inbound: bool,
    pub(crate) storage: Store,
    pub(crate) blockchain: Arc<Blockchain>,
    pub(crate) capabilities: Vec<Capability>,
    pub(crate) negotiated_eth_capability: Option<Capability>,
    pub(crate) negotiated_snap_capability: Option<Capability>,
    pub(crate) last_block_range_update_block: u64,
    /// Maps request ID to (original announcement, actually requested hashes, request time).
    /// The announcement is kept for response validation; the hashes track in-flight state.
    pub(crate) requested_pooled_txs: HashMap<u64, (NewPooledTransactionHashes, Vec<H256>, Instant)>,
    /// eth/72 variant of requested_pooled_txs, also carrying the EIP-8070 role we
    /// took for the requested hashes.
    pub(crate) requested_pooled_txs_72: HashMap<
        u64,
        (
            NewPooledTransactionHashes72,
            Vec<H256>,
            BlobFetchRole,
            Instant,
        ),
    >,
    /// Buffered transaction requests waiting to be flushed as a single batch.
    /// Accumulated between flush ticks (TX_REQUEST_BATCH_INTERVAL).
    pub(crate) pending_tx_requests: Vec<(NewPooledTransactionHashes, Vec<H256>)>,
    /// eth/72 variant of pending_tx_requests.
    pub(crate) pending_tx_requests_72:
        Vec<(NewPooledTransactionHashes72, Vec<H256>, BlobFetchRole)>,
    /// EIP-8070: buffered cell requests (tx_hashes, cell_mask) waiting to be
    /// flushed as a single batched GetCells message.
    pub(crate) pending_cell_requests: Vec<(Vec<H256>, u128)>,
    /// EIP-8070: in-flight `GetCells` requests, keyed by request id, holding the
    /// hashes and cell mask we asked for plus the request time. devp2p requires a
    /// `Cells` response to answer an outstanding request, with a hash set and cell
    /// bitmap that are subsets of what was requested; anything else is a
    /// subprotocol violation. Swept on the same tick as the tx requests.
    pub(crate) requested_cells: HashMap<u64, (Vec<H256>, u128, Instant)>,
    /// EIP-8070: last custody generation this connection acted on. When the
    /// mempool's custody generation advances (Engine API FCU v4 changed the
    /// custody set), the sweep re-samples pending blob txs for the new columns.
    pub(crate) last_custody_generation: u64,
    pub(crate) client_version: String,
    //// Send end of the channel used to broadcast messages
    //// to other connected peers, is ok to have it here,
    //// since internally it's an Arc.
    //// The ID is to ignore the message sent from the same task.
    //// This is used both to send messages and to received broadcasted
    //// messages from other connections (sent from other peers).
    //// The receive end is instantiated after the handshake is completed
    //// under `handle_peer`.
    /// TODO: Improve this mechanism
    /// See https://github.com/lambdaclass/ethrex/issues/3388
    pub(crate) connection_broadcast_send: PeerConnBroadcastSender,
    pub(crate) peer_table: PeerTable,
    #[cfg(feature = "l2")]
    pub(crate) l2_state: L2ConnState,
    pub(crate) tx_broadcaster: ActorRef<TxBroadcaster>,
    pub(crate) current_requests: HashMap<u64, (String, oneshot::Sender<Message>)>,
    // We store the disconnection reason to handle it in the teardown
    pub(crate) disconnect_reason: Option<DisconnectReason>,
    // Indicates if the peer has been validated (ie. the connection was established successfully)
    pub(crate) is_validated: bool,
    // Rate limiting: data-serving requests received in the current window
    pub(crate) serve_requests: WindowCounter,
    // Rate limiting: transaction hashes announced in the current window
    pub(crate) announced_hashes: WindowCounter,
    // Under-delivery: net count of short answers to our GetPooledTransactions requests
    pub(crate) short_tx_answer_strikes: u32,
    // Identifies this connection to the peer table, which removes a node's entry only
    // for the connection that registered it.
    pub(crate) connection_id: ConnectionId,
    // Whether the peer table registered this connection for its node. Only then does
    // this connection own the node's tx-broadcaster index, which it frees when it closes.
    pub(crate) registered_in_peer_table: bool,
    // Leech detection: total transactions sent to this peer via GetPooledTransactions responses
    pub(crate) txs_sent_to_peer: u64,
    // Leech detection: whether we have received any transactions from this peer
    pub(crate) received_txs_from_peer: bool,
    // Liveness: consecutive pings sent without a matching pong. Reset to 0 on every Pong,
    // incremented on each Ping we send; at MAX_MISSED_PONGS the peer is considered dead
    // and dropped (catches application-dead-but-TCP-alive peers that otherwise linger).
    pub(crate) missed_pongs: u8,
}

impl Established {
    async fn teardown(&mut self) {
        // Clear any in-flight transaction hashes so other connections can re-request them,
        // then try to re-issue each pending request to an alternate announcer.
        // Order matters: clear first so the alternate's reserve_unknown_hashes sees the
        // hashes as free; otherwise the actor handler can race with clear_in_flight_txs
        // and silently no-op the retry while consuming an alternates slot.
        for (_, (_announced, requested_hashes, _)) in self.requested_pooled_txs.drain() {
            if let Err(e) = self
                .blockchain
                .mempool
                .clear_in_flight_txs(&requested_hashes)
            {
                warn!(error = %e, "Failed to clear in-flight transaction tracking during peer teardown");
            }
            retry_on_alternates(&self.blockchain, &self.peer_table, &requested_hashes).await;
        }
        for (_, (_announced, requested_hashes, _, _)) in self.requested_pooled_txs_72.drain() {
            if let Err(e) = self
                .blockchain
                .mempool
                .clear_in_flight_txs(&requested_hashes)
            {
                warn!(error = %e, "clear_in_flight_txs failed during teardown");
            }
            retry_on_alternates(&self.blockchain, &self.peer_table, &requested_hashes).await;
        }
        // Also clear hashes that were buffered but not yet sent.
        for (_announced, pending_hashes) in self.pending_tx_requests.drain(..) {
            if let Err(e) = self.blockchain.mempool.clear_in_flight_txs(&pending_hashes) {
                warn!(error = %e, "Failed to clear in-flight transaction tracking during peer teardown");
            }
            retry_on_alternates(&self.blockchain, &self.peer_table, &pending_hashes).await;
        }
        for (_announced, pending_hashes, _) in self.pending_tx_requests_72.drain(..) {
            if let Err(e) = self.blockchain.mempool.clear_in_flight_txs(&pending_hashes) {
                warn!(error = %e, "clear_in_flight_txs failed during teardown");
            }
            retry_on_alternates(&self.blockchain, &self.peer_table, &pending_hashes).await;
        }
        // EIP-8070: forget this peer's advertised cell availability so the map
        // does not grow unbounded across reconnects. A session that never started
        // recorded none: what its node id holds belongs to another session of the
        // same node, which keeps it.
        if self.is_validated
            && let Err(e) = self
                .blockchain
                .mempool
                .clear_peer_cell_availability(self.node.node_id())
        {
            warn!(error = %e, "clear_peer_cell_availability failed during teardown");
        }
        // Release what the peer holds of its announced-hash budget as a fallback announcer
        // and blob provider, so a reconnect starts from what its new session reserves. A
        // session that never started announced nothing: what its node id holds belongs to
        // another session of the same node, which keeps it. A second started session of the
        // same node id, if any, registers again as it announces.
        if self.is_validated
            && let Err(e) = self
                .blockchain
                .mempool
                .forget_peer_announcements(self.node.node_id())
        {
            warn!(error = %e, "forget_peer_announcements failed during teardown");
        }
        // The socket sink is owned by the per-connection writer task (`spawn_outbound_writer`),
        // which closes it when this `Established` is dropped (its `outbound_tx` is the only sender).
    }
}

#[derive(Debug)]
pub enum ConnectionState {
    HandshakeFailed,
    Initiator(Initiator),
    Receiver(Receiver),
    Established(Box<Established>),
}

#[derive(Debug)]
pub struct PeerConnectionServer {
    state: ConnectionState,
    /// Inbound-admission permit (Some for inbound connections, None for outbound dials we
    /// initiate). Held for the actor's lifetime and released when the actor is dropped, so the
    /// inbound connection count stays bounded. See `P2PContext::inbound_admission`.
    _admission_permit: Option<tokio::sync::OwnedSemaphorePermit>,
}

#[actor(protocol = PeerConnectionServerProtocol)]
impl PeerConnectionServer {
    #[started]
    async fn started(&mut self, ctx: &Context<Self>) {
        // Set a default eth version that we can update after we negotiate peer capabilities.
        // This eth version will only be used to encode & decode the initial `Hello` messages.
        let eth_version = Arc::new(RwLock::new(EthCapVersion::default()));
        // snap_version starts as None; set after hello-exchange to the negotiated snap version.
        let snap_version: Arc<RwLock<Option<SnapCapVersion>>> = Arc::new(RwLock::new(None));
        // Take ownership of the state, replacing with HandshakeFailed as placeholder
        let state = std::mem::replace(&mut self.state, ConnectionState::HandshakeFailed);
        // Bound the handshake: a peer that opens TCP and then stalls must not park this
        // actor + socket forever (otherwise established sockets accumulate above the peer
        // target). On timeout the handshake future is dropped, closing the socket.
        let handshake_result = match tokio::time::timeout(
            HANDSHAKE_TIMEOUT,
            handshake::perform(state, eth_version.clone(), snap_version.clone()),
        )
        .await
        {
            Ok(result) => result,
            Err(_elapsed) => {
                debug!("Handshake timed out on RLPx connection");
                self.state = ConnectionState::HandshakeFailed;
                ctx.stop();
                return;
            }
        };
        match handshake_result {
            Ok((mut established_state, stream)) => {
                trace!(peer=%established_state.node, "Starting RLPx connection");
                if let Err(reason) = initialize_connection(
                    ctx,
                    &mut established_state,
                    stream,
                    eth_version,
                    snap_version,
                )
                .await
                {
                    match &reason {
                        PeerConnectionError::NoMatchingCapabilities
                        | PeerConnectionError::HandshakeError(_) => {
                            if let Err(e) = established_state
                                .peer_table
                                .set_unwanted(established_state.node.node_id())
                            {
                                debug!("Failed to set peer as unwanted: {e}");
                            }
                        }
                        _ => {}
                    }
                    connection_failed(
                        &mut established_state,
                        "Failed to initialize RLPx connection",
                        &reason,
                    )
                    .await;

                    METRICS.record_new_rlpx_conn_failure(reason).await;

                    self.state = ConnectionState::Established(Box::new(established_state));
                    ctx.stop();
                } else {
                    METRICS
                        .record_new_rlpx_conn_established(
                            &established_state
                                .node
                                .version
                                .clone()
                                .unwrap_or("Unknown".to_string()),
                        )
                        .await;
                    established_state.is_validated = true;
                    // New state
                    self.state = ConnectionState::Established(Box::new(established_state));
                }
            }
            Err(err) => {
                // Handshake failed, just log a debug message.
                // No connection was established so no need to perform any other action
                debug!("Failed Handshake on RLPx connection {err}");
                self.state = ConnectionState::HandshakeFailed;
                ctx.stop();
            }
        }
    }

    #[stopped]
    async fn stopped(&mut self, _ctx: &Context<Self>) {
        match std::mem::replace(&mut self.state, ConnectionState::HandshakeFailed) {
            ConnectionState::Established(mut established_state) => {
                trace!(peer=%established_state.node, "Closing connection with established peer");
                if established_state.is_validated {
                    // If its validated the peer was connected, so we record the disconnection.
                    let reason = established_state
                        .disconnect_reason
                        .unwrap_or(DisconnectReason::NetworkError);
                    METRICS
                        .record_new_rlpx_conn_disconnection(
                            &established_state
                                .node
                                .version
                                .clone()
                                .unwrap_or("Unknown".to_string()),
                            reason,
                        )
                        .await;
                }
                // The peer table removes the node only if it lists this connection for it, so
                // a duplicate connection to an already-connected node leaves the live one
                // listed. Sent even when registration failed: a registration the peer table
                // applied after this connection stopped waiting for it is undone here.
                if let Err(e) = established_state.peer_table.remove_peer(
                    established_state.node.node_id(),
                    established_state.connection_id,
                ) {
                    debug!("Failed to remove peer from table: {e}");
                }
                // Free the peer's tx-broadcaster index (and clear its bit across known txs)
                // so the broadcaster's per-peer index map / PeerMask widths stay bounded to
                // live peers. The index belongs to the registered connection, so a duplicate
                // leaves the live one's index alone.
                if established_state.registered_in_peer_table
                    && let Err(e) = established_state
                        .tx_broadcaster
                        .remove_peer(established_state.node.node_id())
                {
                    debug!("Failed to remove peer from tx broadcaster: {e}");
                }
                established_state.teardown().await;
            }
            _ => {
                // Nothing to do if the connection was not established
            }
        };
    }

    #[send_handler]
    async fn handle_incoming_message(
        &mut self,
        msg: peer_connection_server_protocol::IncomingMessage,
        ctx: &Context<Self>,
    ) {
        if let ConnectionState::Established(ref mut established_state) = self.state {
            trace!(
                peer=%established_state.node,
                message=%msg.message,
                "Received incoming message",
            );
            let result = handle_incoming_message(established_state, msg.message).await;
            Self::process_cast_error(&self.state, result, ctx);
        } else {
            debug!("Connection not yet established");
        }
    }

    #[send_handler]
    async fn handle_outgoing_message(
        &mut self,
        msg: peer_connection_server_protocol::OutgoingMessage,
        ctx: &Context<Self>,
    ) {
        if let ConnectionState::Established(ref mut established_state) = self.state {
            trace!(
                peer=%established_state.node,
                message=%msg.message,
                "Received outgoing request",
            );
            let result = handle_outgoing_message(established_state, msg.message).await;
            Self::process_cast_error(&self.state, result, ctx);
        } else {
            debug!("Connection not yet established");
        }
    }

    #[send_handler]
    async fn handle_outgoing_request(
        &mut self,
        msg: peer_connection_server_protocol::OutgoingRequest,
        ctx: &Context<Self>,
    ) {
        if let ConnectionState::Established(ref mut established_state) = self.state {
            trace!(
                peer=%established_state.node,
                message=%msg.message,
                "Received outgoing request",
            );
            let Some(sender) = Arc::<oneshot::Sender<Message>>::into_inner(msg.sender) else {
                debug!("Could not obtain sender channel: Arc has multiple references");
                return;
            };
            let result = handle_outgoing_request(established_state, msg.message, sender).await;
            Self::process_cast_error(&self.state, result, ctx);
        } else {
            debug!("Connection not yet established");
        }
    }

    #[send_handler]
    async fn handle_request_timeout(
        &mut self,
        msg: peer_connection_server_protocol::RequestTimeout,
        _ctx: &Context<Self>,
    ) {
        if let ConnectionState::Established(ref mut established_state) = self.state {
            // Discard the request from current requests
            if let Some((msg_type, _)) = established_state.current_requests.remove(&msg.id) {
                debug!(
                    peer=%established_state.node,
                    %msg_type,
                    id=%msg.id,
                    "Request timedout",
                );
            }
        } else {
            debug!("Connection not yet established");
        }
    }

    #[send_handler]
    async fn handle_send_ping(
        &mut self,
        _msg: peer_connection_server_protocol::SendPing,
        ctx: &Context<Self>,
    ) {
        if let ConnectionState::Established(ref mut established_state) = self.state {
            // Liveness: if the peer hasn't answered the last MAX_MISSED_PONGS pings, treat it
            // as dead and drop it instead of pinging a corpse (and keeping its actor) forever.
            if established_state.missed_pongs >= MAX_MISSED_PONGS {
                debug!(peer=%established_state.node, missed=MAX_MISSED_PONGS, "Peer missed max pongs, dropping");
                // Attribute the drop as `PingTimeout` so an unresponsive-peer drop is
                // distinguishable from a generic `NetworkError` in `ethrex_p2p_disconnections`.
                established_state.disconnect_reason = Some(DisconnectReason::PingTimeout);
                ctx.stop();
                return;
            }
            established_state.missed_pongs = established_state.missed_pongs.saturating_add(1);
            let result = send(established_state, Message::Ping(PingMessage {})).await;
            Self::process_cast_error(&self.state, result, ctx);
        } else {
            debug!("Connection not yet established");
        }
    }

    #[send_handler]
    async fn handle_block_range_update(
        &mut self,
        _msg: peer_connection_server_protocol::BlockRangeUpdate,
        ctx: &Context<Self>,
    ) {
        if let ConnectionState::Established(ref mut established_state) = self.state {
            trace!(
                peer=%established_state.node,
                "Block Range Update"
            );
            let result = handle_block_range_update(established_state).await;
            Self::process_cast_error(&self.state, result, ctx);
        } else {
            debug!("Connection not yet established");
        }
    }

    #[send_handler]
    async fn handle_sweep_inflight_txs(
        &mut self,
        _msg: peer_connection_server_protocol::SweepInflightTxs,
        _ctx: &Context<Self>,
    ) {
        if let ConnectionState::Established(ref mut state) = self.state {
            let now = Instant::now();
            let stale_ids: Vec<u64> = state
                .requested_pooled_txs
                .iter()
                .filter(|(_, (_, _, ts))| now.duration_since(*ts) > INFLIGHT_TX_TIMEOUT)
                .map(|(id, _)| *id)
                .collect();
            for id in stale_ids {
                if let Some((_announced, hashes, _)) = state.requested_pooled_txs.remove(&id) {
                    // Clear in-flight before retry so the alternate's reserve_unknown_hashes
                    // doesn't race against still-in-flight state and silently no-op.
                    if let Err(e) = state.blockchain.mempool.clear_in_flight_txs(&hashes) {
                        warn!(error = %e, "Failed to clear in-flight transaction tracking while sweeping stale requests");
                    }
                    retry_on_alternates(&state.blockchain, &state.peer_table, &hashes).await;
                }
            }
            // Sweep eth/72 in-flight requests.
            let stale_ids_72: Vec<u64> = state
                .requested_pooled_txs_72
                .iter()
                .filter(|(_, (_, _, _, ts))| now.duration_since(*ts) > INFLIGHT_TX_TIMEOUT)
                .map(|(id, _)| *id)
                .collect();
            for id in stale_ids_72 {
                if let Some((_announced, hashes, _, _)) = state.requested_pooled_txs_72.remove(&id)
                {
                    if let Err(e) = state.blockchain.mempool.clear_in_flight_txs(&hashes) {
                        warn!(error = %e, "clear_in_flight_txs failed while sweeping stale v72 requests");
                    }
                    retry_on_alternates(&state.blockchain, &state.peer_table, &hashes).await;
                }
            }
            // EIP-8070: drop in-flight GetCells entries the peer never answered, so
            // the map can't grow unbounded on a peer that silently ignores requests.
            // Unlike tx requests there is nothing to retry: the sampler re-requests
            // cells on the next announcement or custody change.
            state
                .requested_cells
                .retain(|_, (_, _, ts)| now.duration_since(*ts) <= INFLIGHT_TX_TIMEOUT);
            // EIP-8070: prune cell entries for txs that left the pool.
            if let Err(e) = state.blockchain.mempool.prune_cells() {
                warn!(error = %e, "prune_cells failed during sweep");
            }
            if let Err(e) = state
                .blockchain
                .mempool
                .prune_alternates(INFLIGHT_TX_TIMEOUT)
            {
                warn!(error = %e, "prune_alternates failed during sweep");
            }
            // EIP-8070: when the custody set changed (Engine API FCU v4) or eager
            // mode latched on, fetch the now-wanted columns this peer can serve for
            // pending blob txs. Inert unless sampling is enabled, and only ever
            // asked of an eth/72 peer.
            if state.blockchain.mempool.blob_sampling_enabled && supports_eth72(state) {
                let generation = state.blockchain.mempool.custody_generation();
                if generation != state.last_custody_generation {
                    state.last_custody_generation = generation;
                    // Unknown availability => assume the peer can serve any column
                    // (matches the sampler-fetch default).
                    let peer_available = state
                        .blockchain
                        .mempool
                        .peer_cell_mask(state.node.node_id())
                        .unwrap_or(None)
                        .unwrap_or(u128::MAX);
                    match state.blockchain.mempool.blob_txs_missing_cells() {
                        Ok(missing_list) => {
                            for (tx_hash, missing) in missing_list {
                                let fetch_mask = missing & peer_available;
                                if fetch_mask != 0 {
                                    state
                                        .pending_cell_requests
                                        .push((vec![tx_hash], fetch_mask));
                                }
                            }
                        }
                        Err(e) => {
                            warn!(error = %e, "blob_txs_missing_cells failed during sweep")
                        }
                    }
                }
            }
        }
    }

    #[send_handler]
    async fn handle_flush_pending_tx_requests(
        &mut self,
        _msg: peer_connection_server_protocol::FlushPendingTxRequests,
        ctx: &Context<Self>,
    ) {
        if let ConnectionState::Established(ref mut established_state) = self.state {
            let result = flush_pending_tx_requests(established_state).await;
            let result72 = flush_pending_tx_requests_72(established_state).await;
            let result_cells = flush_pending_cell_requests(established_state).await;
            Self::process_cast_error(&self.state, result, ctx);
            Self::process_cast_error(&self.state, result72, ctx);
            Self::process_cast_error(&self.state, result_cells, ctx);
        }
    }

    #[send_handler]
    async fn handle_enqueue_tx_requests(
        &mut self,
        msg: peer_connection_server_protocol::EnqueueTxRequests,
        _ctx: &Context<Self>,
    ) {
        if let ConnectionState::Established(ref mut state) = self.state {
            queue_retried_request(state, &msg.announcement);
        }
    }

    #[send_handler]
    async fn handle_broadcast_message(
        &mut self,
        msg: peer_connection_server_protocol::BroadcastMessage,
        ctx: &Context<Self>,
    ) {
        if let ConnectionState::Established(ref mut established_state) = self.state {
            trace!(
                peer=%established_state.node,
                message=%msg.msg,
                "Received broadcasted message",
            );
            let result = handle_broadcast(established_state, (msg.task_id, msg.msg)).await;
            Self::process_cast_error(&self.state, result, ctx);
        } else {
            debug!("Connection not yet established");
        }
    }

    #[cfg(feature = "l2")]
    #[send_handler]
    async fn handle_l2_message(&mut self, msg: L2Message, ctx: &Context<Self>) {
        if let ConnectionState::Established(ref mut established_state) = self.state {
            let peer_supports_l2 = established_state.l2_state.connection_state().is_ok();
            let result = if peer_supports_l2 {
                trace!(
                    peer=%established_state.node,
                    message=?msg.msg,
                    "Handling cast for L2 msg"
                );
                match msg.msg {
                    L2Cast::BatchBroadcast => {
                        let res = l2_connection::send_sealed_batch(established_state).await;
                        res.and(l2_connection::process_batches_on_queue(established_state).await)
                    }
                    L2Cast::BlockBroadcast => {
                        l2_connection::send_new_block(established_state).await
                    }
                }
            } else {
                Err(PeerConnectionError::MessageNotHandled(
                    "Unknown message or capability not handled".to_string(),
                ))
            };
            Self::process_cast_error(&self.state, result, ctx);
        } else {
            debug!("Connection not yet established");
        }
    }

    fn process_cast_error(
        state: &ConnectionState,
        result: Result<(), PeerConnectionError>,
        ctx: &Context<Self>,
    ) {
        if let Err(e) = result
            && let ConnectionState::Established(established_state) = state
        {
            match e {
                PeerConnectionError::Disconnected
                | PeerConnectionError::DisconnectReceived(_)
                | PeerConnectionError::DisconnectSent(_)
                | PeerConnectionError::HandshakeError(_)
                | PeerConnectionError::NoMatchingCapabilities
                | PeerConnectionError::InvalidPeerId
                | PeerConnectionError::InvalidMessageLength
                | PeerConnectionError::StateError(_)
                | PeerConnectionError::InvalidRecoveryId
                | PeerConnectionError::OutboundSendTimeout
                | PeerConnectionError::OutboundQueueFull => {
                    trace!(peer=%established_state.node, error=e.to_string(), "Peer connection error");
                    ctx.stop();
                }
                PeerConnectionError::IoError(ref io_e)
                    if io_e.kind() == std::io::ErrorKind::BrokenPipe =>
                {
                    // TODO: we need to check if this message is ocurring commonly due to a problem
                    // with our concurrency model
                    debug!(peer=%established_state.node, "Broken pipe with peer, disconnected");
                    ctx.stop();
                }
                PeerConnectionError::StoreError(StoreError::Trie(TrieError::InconsistentTree(
                    _,
                ))) => {
                    if established_state.blockchain.is_synced() {
                        // If we're responding with inconsistent trie while synced, our trie may be broken
                        // If this error is non sporadic we should investigate
                        error!(
                            peer=%established_state.node,
                            error=%e,
                            "Inconsistent trie while serving peer request; local state may be corrupted",
                        );
                    } else {
                        // If we're not synced, we expect to have inconsistent trie errors
                        trace!(
                            peer=%established_state.node,
                            error=%e,
                            "Error handling cast message",
                        );
                    }
                }
                _ => {
                    // We should check why we're failling to handle the cast message
                    debug!(
                        peer=%established_state.node,
                        capabilities=?established_state.capabilities,
                        error=%e,
                        "Error handling cast message",
                    );
                }
            }
        }
    }
}

async fn initialize_connection<S>(
    ctx: &Context<PeerConnectionServer>,
    state: &mut Established,
    mut stream: S,
    eth_version: Arc<RwLock<EthCapVersion>>,
    snap_version: Arc<RwLock<Option<SnapCapVersion>>>,
) -> Result<(), PeerConnectionError>
where
    S: Unpin + Send + Stream<Item = Result<Message, PeerConnectionError>> + 'static,
{
    if state.peer_table.target_peers_reached().await? {
        debug!(peer=%state.node, "Reached target peer connections, discarding.");
        return Err(PeerConnectionError::TooManyPeers);
    }
    exchange_hello_messages(state, &mut stream).await?;

    // Update eth capability version to the negotiated version for further message decoding.
    let version = match &state.negotiated_eth_capability {
        Some(cap) if cap == &Capability::eth(68) => EthCapVersion::V68,
        Some(cap) if cap == &Capability::eth(69) => EthCapVersion::V69,
        Some(cap) if cap == &Capability::eth(70) => EthCapVersion::V70,
        Some(cap) if cap == &Capability::eth(71) => EthCapVersion::V71,
        Some(cap) if cap == &Capability::eth(72) => EthCapVersion::V72,
        _ => EthCapVersion::default(),
    };
    *eth_version
        .write()
        .map_err(|err| PeerConnectionError::InternalError(err.to_string()))? = version;

    // Update snap capability version to the negotiated version.
    let snap_ver = match &state.negotiated_snap_capability {
        Some(cap) if cap == &Capability::snap(1) => Some(SnapCapVersion::V1),
        Some(cap) if cap == &Capability::snap(2) => Some(SnapCapVersion::V2),
        _ => None,
    };
    *snap_version
        .write()
        .map_err(|err| PeerConnectionError::InternalError(err.to_string()))? = snap_ver;

    init_capabilities(state, &mut stream).await?;

    let mut connection = PeerConnection {
        handle: ctx.actor_ref(),
    };

    let negotiated_capabilities: Vec<Capability> = state
        .negotiated_eth_capability
        .iter()
        .chain(state.negotiated_snap_capability.iter())
        .cloned()
        .collect();

    let registered = state
        .peer_table
        .new_connected_peer(
            state.node.clone(),
            connection.clone(),
            state.connection_id,
            state.capabilities.clone(),
            negotiated_capabilities,
            state.negotiated_eth_capability.clone(),
            state.is_inbound,
        )
        .await?;
    if !registered {
        return Err(PeerConnectionError::AlreadyConnected);
    }
    state.registered_in_peer_table = true;

    trace!(peer=%state.node, "Peer connection initialized.");

    // Send transactions transaction hashes from mempool at connection start
    send_all_pooled_tx_hashes(state, &mut connection).await?;

    // Periodic Pings repeated events.
    send_interval(
        PING_INTERVAL,
        ctx.clone(),
        peer_connection_server_protocol::SendPing,
    );

    // Periodic block range update.
    send_interval(
        BLOCK_RANGE_UPDATE_INTERVAL,
        ctx.clone(),
        peer_connection_server_protocol::BlockRangeUpdate,
    );

    // Periodic sweep of stale in-flight transaction requests.
    send_interval(
        INFLIGHT_TX_SWEEP_INTERVAL,
        ctx.clone(),
        peer_connection_server_protocol::SweepInflightTxs,
    );

    // Periodic flush of buffered transaction requests.
    send_interval(
        TX_REQUEST_BATCH_INTERVAL,
        ctx.clone(),
        peer_connection_server_protocol::FlushPendingTxRequests,
    );

    #[cfg(feature = "l2")]
    // Periodic L2 messages events.
    if state.l2_state.connection_state().is_ok() {
        send_interval(
            PERIODIC_BLOCK_BROADCAST_INTERVAL,
            ctx.clone(),
            L2Message {
                msg: L2Cast::BlockBroadcast,
            },
        );
        send_interval(
            PERIODIC_BATCH_BROADCAST_INTERVAL,
            ctx.clone(),
            L2Message {
                msg: L2Cast::BatchBroadcast,
            },
        );
    }

    spawn_listener(
        ctx.clone(),
        stream.filter_map(|result| match result {
            Ok(msg) => Some(peer_connection_server_protocol::IncomingMessage { message: msg }),
            Err(e) => {
                debug!(error=?e, "Error receiving RLPx message");
                // Skipping invalid data
                None
            }
        }),
    );

    if state.negotiated_eth_capability.is_some() {
        let stream: BroadcastStream<(Id, Arc<Message>)> =
            BroadcastStream::new(state.connection_broadcast_send.subscribe());
        let message_stream = stream.filter_map(|result| {
            result.ok().map(
                |(id, msg)| peer_connection_server_protocol::BroadcastMessage { task_id: id, msg },
            )
        });
        spawn_listener(ctx.clone(), message_stream);
    }

    Ok(())
}

async fn send_all_pooled_tx_hashes(
    state: &mut Established,
    connection: &mut PeerConnection,
) -> Result<(), PeerConnectionError> {
    // --mempool.private: locally-submitted private txs MUST NOT be
    // disclosed via the new-peer pooled-hashes dump.
    let txs = state.blockchain.mempool.get_txs_for_new_peer_dump()?;
    if !txs.is_empty() {
        state
            .tx_broadcaster
            .add_txs(
                txs.iter().map(|tx| tx.hash(&NativeCrypto)).collect(),
                state.node.node_id(),
            )
            .map_err(|e| PeerConnectionError::BroadcastError(e.to_string()))?;
        send_tx_hashes(
            txs,
            state.negotiated_eth_capability.clone(),
            connection,
            state.node.node_id(),
            &state.blockchain,
        )
        .await
        .map_err(|e| PeerConnectionError::SendMessage(e.to_string()))?;
    }
    Ok(())
}

async fn send_block_range_update(state: &mut Established) -> Result<(), PeerConnectionError> {
    // BlockRangeUpdate was introduced in eth/69
    if state
        .negotiated_eth_capability
        .as_ref()
        .is_some_and(|eth| eth.version >= 69)
    {
        trace!(peer=%state.node, "Sending BlockRangeUpdate");
        let update = BlockRangeUpdate::new(&state.storage).await?;
        let latest_block = update.latest_block;
        send(state, Message::BlockRangeUpdate(update)).await?;
        state.last_block_range_update_block = latest_block - (latest_block % 32);
    }
    Ok(())
}

fn should_send_block_range_update(state: &Established) -> Result<bool, PeerConnectionError> {
    let latest_block = state.storage.get_latest_block_number()?;
    if latest_block < state.last_block_range_update_block
        || latest_block - state.last_block_range_update_block >= 32
    {
        return Ok(true);
    }
    Ok(false)
}

async fn init_capabilities<S>(
    state: &mut Established,
    stream: &mut S,
) -> Result<(), PeerConnectionError>
where
    S: Unpin + Stream<Item = Result<Message, PeerConnectionError>>,
{
    // Sending eth Status if peer supports it
    if let Some(eth) = state.negotiated_eth_capability.clone() {
        let status = match eth.version {
            68 => Message::Status68(StatusMessage68::new(&state.storage)?),
            69 => Message::Status69(StatusMessage69::new(&state.storage).await?),
            70 => Message::Status70(StatusMessage70::new(&state.storage).await?),
            71 => Message::Status71(StatusMessage71::new(&state.storage).await?),
            72 => Message::Status72(StatusMessage72::new(&state.storage).await?),
            ver => {
                return Err(PeerConnectionError::HandshakeError(format!(
                    "Invalid eth version {ver}"
                )));
            }
        };
        trace!(peer=%state.node, "Sending status");
        send(state, status).await?;
        // The next immediate message in the ETH protocol is the
        // status, reference here:
        // https://github.com/ethereum/devp2p/blob/master/caps/eth.md#status-0x00
        let msg = match receive(stream).await {
            Some(msg) => msg?,
            None => return Err(PeerConnectionError::Disconnected),
        };
        #[cfg(feature = "l2")]
        state.l2_state.set_peer_head(&state.storage, &msg);
        match msg {
            Message::Status68(msg_data) => {
                trace!(peer=%state.node, "Received Status(68)");
                backend::validate_status(msg_data, &state.storage, &eth)?
            }
            Message::Status69(msg_data) => {
                trace!(peer=%state.node, "Received Status(69)");
                backend::validate_status(msg_data, &state.storage, &eth)?
            }
            Message::Status70(msg_data) => {
                trace!(peer=%state.node, "Received Status(70)");
                backend::validate_status(msg_data, &state.storage, &eth)?
            }
            Message::Status71(msg_data) => {
                trace!(peer=%state.node, "Received Status(71)");
                backend::validate_status(msg_data, &state.storage, &eth)?
            }
            Message::Status72(msg_data) => {
                trace!(peer=%state.node, "Received Status(72)");
                backend::validate_status(msg_data, &state.storage, &eth)?
            }
            Message::Disconnect(disconnect) => {
                return Err(PeerConnectionError::HandshakeError(format!(
                    "Peer disconnected due to: {}",
                    disconnect.reason()
                )));
            }
            _ => {
                return Err(PeerConnectionError::HandshakeError(
                    "Expected a Status message".to_string(),
                ));
            }
        }
    }
    Ok(())
}

async fn send_disconnect_message(state: &mut Established, reason: Option<DisconnectReason>) {
    send(state, Message::Disconnect(DisconnectMessage { reason }))
        .await
        .unwrap_or_else(|_| {
            debug!(
                peer=%state.node,
                ?reason,
                "Could not send Disconnect message",
            );
        });
}

async fn connection_failed(state: &mut Established, error_text: &str, error: &PeerConnectionError) {
    debug!(
        peer=%state.node,
        %error_text,
        %error,
        "connection failure"
    );

    // Send disconnect message only if error is different than RLPxError::DisconnectRequested
    // because if it is a DisconnectRequested error it means that the peer requested the disconnection, not us.
    if !matches!(error, PeerConnectionError::DisconnectReceived(_)) {
        send_disconnect_message(state, match_disconnect_reason(error)).await;
    }

    // Discard peer from kademlia table in some cases
    match error {
        // already connected, don't discard it
        PeerConnectionError::AlreadyConnected
        | PeerConnectionError::DisconnectReceived(DisconnectReason::AlreadyConnected)
        | PeerConnectionError::DisconnectSent(DisconnectReason::AlreadyConnected) => {
            debug!(
                peer=%state.node,
                %error_text,
                %error,
                "Peer already connected, don't replace it"
            );
        }
        _ => {
            debug!(
                peer=%state.node,
                %error_text,
                %error,
                remote_public_key=%state.node.public_key,
                "discarding peer",
            );
        }
    }
}

fn match_disconnect_reason(error: &PeerConnectionError) -> Option<DisconnectReason> {
    match error {
        PeerConnectionError::DisconnectSent(reason) => Some(*reason),
        PeerConnectionError::DisconnectReceived(reason) => Some(*reason),
        PeerConnectionError::RLPDecodeError(_) => Some(DisconnectReason::NetworkError),
        PeerConnectionError::TooManyPeers => Some(DisconnectReason::TooManyPeers),
        PeerConnectionError::AlreadyConnected => Some(DisconnectReason::AlreadyConnected),
        // TODO build a proper matching between error types and disconnection reasons
        _ => None,
    }
}

async fn exchange_hello_messages<S>(
    state: &mut Established,
    stream: &mut S,
) -> Result<(), PeerConnectionError>
where
    S: Unpin + Stream<Item = Result<Message, PeerConnectionError>>,
{
    // eth/72 (EIP-8070) is only safe to negotiate when blob sampling is enabled:
    // it always elides blob payloads in PooledTransactions, and a node that does
    // not run the sampler/provider cell-fetch loop would receive blob txs it can
    // never reconstruct. With sampling off we cap at eth/71 so default nodes keep
    // full-blob propagation unchanged. The EIP's Backwards Compatibility section
    // explicitly supports this gradual, version-gated rollout.
    let offer_eth72 = state.blockchain.mempool.blob_sampling_enabled;
    // This allow is because in l2 we mut the capabilities
    // to include the l2 cap
    let snap_capabilities =
        advertised_snap_capabilities(state.blockchain.state_sync_needs_trie_nodes());
    #[allow(unused_mut)]
    let mut supported_capabilities: Vec<Capability> = SUPPORTED_ETH_CAPABILITIES
        .iter()
        .filter(|cap| offer_eth72 || cap.version < 72)
        .chain(snap_capabilities.iter())
        .cloned()
        .collect();
    #[cfg(feature = "l2")]
    if state.l2_state.is_supported() {
        supported_capabilities.push(crate::rlpx::l2::SUPPORTED_BASED_CAPABILITIES[0].clone());
    }
    let hello_msg = Message::Hello(p2p::HelloMessage::new(
        supported_capabilities,
        PublicKey::from_secret_key(secp256k1::SECP256K1, &state.signer),
        state.client_version.clone(),
    ));

    send(state, hello_msg).await?;

    // Receive Hello message
    let msg = match receive(stream).await {
        Some(msg) => msg?,
        None => return Err(PeerConnectionError::Disconnected),
    };

    match msg {
        Message::Hello(hello_message) => {
            let mut negotiated_eth_version = 0;
            let mut negotiated_snap_version = 0;

            trace!(
                peer=%state.node,
                capabilities=?hello_message.capabilities,
                "Hello message capabilities",
            );

            // Check if we have any capability in common and store the highest version
            for cap in &hello_message.capabilities {
                match cap.protocol() {
                    "eth" => {
                        // Don't negotiate a version we didn't advertise: eth/72 is
                        // only offered when blob sampling is enabled (see above).
                        if SUPPORTED_ETH_CAPABILITIES.contains(cap)
                            && (offer_eth72 || cap.version < 72)
                            && cap.version > negotiated_eth_version
                        {
                            negotiated_eth_version = cap.version;
                        }
                    }
                    "snap" => {
                        // Match against what this connection actually advertised, not the
                        // full set: negotiating a version we withheld would hand back the
                        // snap/2 the sync gate above deliberately kept off the wire.
                        if snap_capabilities.contains(cap) && cap.version > negotiated_snap_version
                        {
                            negotiated_snap_version = cap.version;
                        }
                    }
                    #[cfg(feature = "l2")]
                    "based" if state.l2_state.is_supported() => {
                        state.l2_state.set_established()?;
                    }
                    _ => {}
                }
            }

            state.capabilities = hello_message.capabilities;

            if negotiated_eth_version == 0 {
                return Err(PeerConnectionError::NoMatchingCapabilities);
            }
            debug!("Negotiated eth version: eth/{}", negotiated_eth_version);
            state.negotiated_eth_capability = Some(Capability::eth(negotiated_eth_version));

            if negotiated_snap_version != 0 {
                debug!("Negotiated snap version: snap/{}", negotiated_snap_version);
                state.negotiated_snap_capability = Some(Capability::snap(negotiated_snap_version));
            }

            state.node.version = Some(hello_message.client_id);

            Ok(())
        }
        Message::Disconnect(disconnect) => {
            Err(PeerConnectionError::DisconnectReceived(disconnect.reason()))
        }
        _ => {
            // Fail if it is not a hello message
            Err(PeerConnectionError::BadRequest(
                "Expected Hello message".to_string(),
            ))
        }
    }
}

pub(crate) async fn send(
    state: &mut Established,
    message: Message,
) -> Result<(), PeerConnectionError> {
    #[cfg(feature = "metrics")]
    {
        use ethrex_metrics::p2p::METRICS_P2P;
        METRICS_P2P.inc_outgoing_message(message.metric_label());
    }
    // Hand off to the per-connection writer task via a BOUNDED queue (non-blocking). This
    // decouples the actor's serial drain from the network write, so a slow/wedged peer can
    // never back up the (unbounded) actor mailbox. If the bounded queue is full the peer
    // can't keep up: surface OutboundQueueFull so `process_cast_error` drops it.
    match state.outbound_tx.try_send(message) {
        Ok(()) => Ok(()),
        Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => {
            // The peer can't drain our outbound fast enough. Attribute the drop as `UselessPeer`
            // so it is distinguishable from a generic `NetworkError` in `ethrex_p2p_disconnections`
            // (a spike here flags slow peers or our own over-sending, rather than hiding it).
            state
                .disconnect_reason
                .get_or_insert(DisconnectReason::UselessPeer);
            Err(PeerConnectionError::OutboundQueueFull)
        }
        Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => {
            // The writer task is gone. If it exited because a single send exceeded
            // `OUTBOUND_SEND_TIMEOUT`, the peer was wedged: surface `OutboundSendTimeout` and
            // attribute it likewise; otherwise the socket simply closed (`Disconnected`).
            if state
                .outbound_writer_timed_out
                .load(std::sync::atomic::Ordering::Acquire)
            {
                state
                    .disconnect_reason
                    .get_or_insert(DisconnectReason::UselessPeer);
                Err(PeerConnectionError::OutboundSendTimeout)
            } else {
                Err(PeerConnectionError::Disconnected)
            }
        }
    }
}

/// Spawns the per-connection writer task that owns the `sink` and drains a bounded outbound
/// queue into it. Keeping the network write off the actor thread means the actor's mailbox is
/// never gated on a slow peer; the only outbound buffer is this bounded channel (capacity
/// `OUTBOUND_QUEUE_CAP`). The task exits — closing the socket — when the connection is dropped
/// (all senders gone), when a send errors, or when a single send exceeds `OUTBOUND_SEND_TIMEOUT`
/// (a wedged peer). Once it exits, further `send()`s observe a closed queue and the peer is dropped.
pub(crate) fn spawn_outbound_writer(
    mut sink: SplitSink<Framed<TcpStream, RLPxCodec>, Message>,
) -> (
    tokio::sync::mpsc::Sender<Message>,
    std::sync::Arc<std::sync::atomic::AtomicBool>,
) {
    let (tx, mut rx) = tokio::sync::mpsc::channel::<Message>(OUTBOUND_QUEUE_CAP);
    // Set when the writer exits because a single send exceeded `OUTBOUND_SEND_TIMEOUT` (a wedged
    // peer), so `send` can surface `OutboundSendTimeout` instead of a generic `Disconnected` once
    // the bounded queue closes.
    let timed_out = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let writer_timed_out = timed_out.clone();
    tokio::spawn(async move {
        while let Some(message) = rx.recv().await {
            match tokio::time::timeout(OUTBOUND_SEND_TIMEOUT, sink.send(message)).await {
                Ok(Ok(())) => {}
                // Send error: the socket is gone; let the closed queue surface as `Disconnected`.
                Ok(Err(_)) => break,
                // The send itself timed out: the peer is wedged. Flag it so the drop is attributed.
                Err(_) => {
                    writer_timed_out.store(true, std::sync::atomic::Ordering::Release);
                    break;
                }
            }
        }
        let _ = sink.close().await;
    });
    (tx, timed_out)
}

/// Reads from the frame until a frame is available.
///
/// Returns `None` when the stream buffer is 0. This could indicate that the client has disconnected,
/// but we cannot safely assume an EOF, as per the Tokio documentation.
///
/// If the handshake has not been established, it is reasonable to terminate the connection.
///
/// For an established connection, [`check_periodic_task`] will detect actual disconnections
/// while sending pings and you should not assume a disconnection.
///
/// See [`Framed::new`] for more details.
async fn receive<S>(stream: &mut S) -> Option<Result<Message, PeerConnectionError>>
where
    S: Unpin + Stream<Item = Result<Message, PeerConnectionError>>,
{
    stream.next().await
}

/// Whether an answer to `GetPooledTransactions` delivered too little of what was asked. devp2p
/// lets a peer skip transactions it no longer holds and stop at its own response limit
/// (go-ethereum stops past 2 MiB, we stop just before it, and one blob transaction can take up
/// to ~1 MiB), so an answer is short only when it delivered under a quarter of the requested
/// transactions *and* under a quarter of that limit in bytes: an answer cut by a size limit
/// always carries more. The bytes are those of the transactions as delivered, not their
/// announced sizes: an eth/72 answer carries blob transactions without their blobs, and
/// nothing checks a blob transaction's announced size against what it delivers. An empty
/// answer to a non-empty request is short.
fn is_short_tx_answer(requested_hashes: &[H256], delivered: &HashMap<H256, usize>) -> bool {
    let mut delivered_count = 0usize;
    let mut delivered_bytes = 0usize;
    for size in requested_hashes
        .iter()
        .filter_map(|hash| delivered.get(hash))
    {
        delivered_count += 1;
        delivered_bytes = delivered_bytes.saturating_add(*size);
    }
    delivered_count * 4 < requested_hashes.len() && delivered_bytes < MAX_POOLED_RESPONSE_BYTES / 4
}

/// Drops the alternate announcers of transactions this peer delivered for one of our requests;
/// see `Mempool::forget_alternates`.
fn forget_delivered_alternates(state: &Established, delivered: HashMap<H256, usize>) {
    let delivered: Vec<H256> = delivered.into_keys().collect();
    if let Err(e) = state.blockchain.mempool.forget_alternates(&delivered) {
        warn!(error = %e, "forget_alternates failed");
    }
}

/// Scores how much of one `GetPooledTransactions` request a peer's answer delivered. What it
/// left out that is settled since (see `Mempool::settled_txs`: in the pool, or included in a
/// recent block, which an honest peer drops as well) is not counted, and its fallback
/// announcers are forgotten rather than asked. A short answer adds a strike and has the rest of
/// what it left out requested from other announcers, so a peer withholding what it announced
/// does not keep those transactions from us; any other answer takes a strike off. At
/// `MAX_SHORT_TX_ANSWER_STRIKES` every requested hash still wanted is retried on other
/// announcers, since the delivered ones are not processed either, and the peer is disconnected.
/// The peer's score is left alone: it ranks peers for sync, which transaction gossip says
/// nothing about.
async fn account_tx_answer(
    state: &mut Established,
    requested_hashes: &[H256],
    delivered: &HashMap<H256, usize>,
) -> Result<(), PeerConnectionError> {
    if !is_short_tx_answer(requested_hashes, delivered) {
        state.short_tx_answer_strikes = state.short_tx_answer_strikes.saturating_sub(1);
        return Ok(());
    }
    let undelivered: Vec<H256> = requested_hashes
        .iter()
        .filter(|hash| !delivered.contains_key(*hash))
        .copied()
        .collect();
    let settled: HashSet<H256> = state
        .blockchain
        .mempool
        .settled_txs(&undelivered)?
        .into_iter()
        .collect();
    if !settled.is_empty() {
        let settled: Vec<H256> = settled.iter().copied().collect();
        state.blockchain.mempool.forget_alternates(&settled)?;
    }
    let wanted: Vec<H256> = requested_hashes
        .iter()
        .filter(|hash| !settled.contains(*hash))
        .copied()
        .collect();
    if !is_short_tx_answer(&wanted, delivered) {
        state.short_tx_answer_strikes = state.short_tx_answer_strikes.saturating_sub(1);
        return Ok(());
    }
    state.short_tx_answer_strikes += 1;
    if state.short_tx_answer_strikes < MAX_SHORT_TX_ANSWER_STRIKES {
        let withheld: Vec<H256> = undelivered
            .into_iter()
            .filter(|hash| !settled.contains(hash))
            .collect();
        retry_on_alternates(&state.blockchain, &state.peer_table, &withheld).await;
        return Ok(());
    }
    debug!(
        peer = %state.node,
        strikes = state.short_tx_answer_strikes,
        "Disconnecting peer: keeps answering transaction requests with little of what it announced",
    );
    retry_on_alternates(&state.blockchain, &state.peer_table, &wanted).await;
    send_disconnect_message(state, Some(DisconnectReason::UselessPeer)).await;
    Err(PeerConnectionError::DisconnectSent(
        DisconnectReason::UselessPeer,
    ))
}

async fn handle_incoming_message(
    state: &mut Established,
    message: Message,
) -> Result<(), PeerConnectionError> {
    #[cfg(feature = "metrics")]
    {
        use ethrex_metrics::p2p::METRICS_P2P;
        METRICS_P2P.inc_incoming_message(message.metric_label());
    }

    // Rate-limit incoming data-serving requests to prevent resource exhaustion.
    let is_data_request = matches!(
        message,
        Message::GetBlockHeaders(_)
            | Message::GetBlockBodies(_)
            | Message::GetReceipts68(_)
            | Message::GetReceipts69(_)
            | Message::GetReceipts70(_)
            | Message::GetPooledTransactions(_)
            | Message::GetAccountRange(_)
            | Message::GetStorageRanges(_)
            | Message::GetByteCodes(_)
            | Message::GetTrieNodes(_)
            | Message::GetBlockAccessLists(_)
            | Message::Snap2GetBlockAccessLists(_)
            | Message::GetCells(_)
    );
    if is_data_request && !state.serve_requests.hit(1, MAX_SERVE_REQUESTS_PER_WINDOW) {
        debug!(
            peer = %state.node,
            window_requests = state.serve_requests.count(),
            "Disconnecting peer: exceeded incoming request rate limit",
        );
        send_disconnect_message(state, Some(DisconnectReason::UselessPeer)).await;
        return Err(PeerConnectionError::DisconnectSent(
            DisconnectReason::UselessPeer,
        ));
    }

    // Rate-limit transaction announcements by the hashes they carry: each one costs a pass over
    // its hashes under the mempool lock, even when every hash is already known. One counts at
    // least one hash, so a flood of empty announcements is limited too.
    let announced_hashes = match &message {
        Message::NewPooledTransactionHashes(announcement) => {
            Some(announcement.transaction_hashes.len())
        }
        Message::NewPooledTransactionHashes72(announcement) => {
            Some(announcement.transaction_hashes.len())
        }
        _ => None,
    };
    if let Some(hashes) = announced_hashes
        && !state
            .announced_hashes
            .hit(hashes.max(1) as u64, MAX_ANNOUNCED_HASHES_PER_WINDOW)
    {
        debug!(
            peer = %state.node,
            window_hashes = state.announced_hashes.count(),
            "Disconnecting peer: exceeded transaction announcement rate limit",
        );
        send_disconnect_message(state, Some(DisconnectReason::UselessPeer)).await;
        return Err(PeerConnectionError::DisconnectSent(
            DisconnectReason::UselessPeer,
        ));
    }

    let peer_supports_eth = state.negotiated_eth_capability.is_some();
    #[cfg(feature = "l2")]
    let peer_supports_l2 = state.l2_state.connection_state().is_ok();
    match message {
        Message::Disconnect(msg_data) => {
            let reason = msg_data.reason();
            trace!(
                peer=%state.node,
                ?reason,
                "Received Disconnect"
            );
            state.disconnect_reason = Some(reason);

            // TODO handle the disconnection request

            return Err(PeerConnectionError::DisconnectReceived(reason));
        }
        Message::Ping(_) => {
            trace!(peer=%state.node, "Sending pong message");
            send(state, Message::Pong(PongMessage {})).await?;
        }
        Message::Pong(_) => {
            // Liveness: the peer answered our ping; reset the missed-pong counter.
            state.missed_pongs = 0;
        }
        Message::Status68(msg_data) => {
            if let Some(eth) = &state.negotiated_eth_capability {
                backend::validate_status(msg_data, &state.storage, eth)?
            };
        }
        Message::Status69(msg_data) => {
            if let Some(eth) = &state.negotiated_eth_capability {
                backend::validate_status(msg_data, &state.storage, eth)?
            };
        }
        Message::Status70(msg_data) => {
            if let Some(eth) = &state.negotiated_eth_capability {
                backend::validate_status(msg_data, &state.storage, eth)?
            };
        }
        Message::Status71(msg_data) => {
            if let Some(eth) = &state.negotiated_eth_capability {
                backend::validate_status(msg_data, &state.storage, eth)?
            };
        }
        Message::Status72(msg_data) => {
            if let Some(eth) = &state.negotiated_eth_capability {
                backend::validate_status(msg_data, &state.storage, eth)?
            };
        }
        Message::GetAccountRange(req) => {
            let response = process_account_range_request(req, state.storage.clone()).await?;
            send(state, Message::AccountRange(response)).await?
        }
        Message::Transactions(txs) if peer_supports_eth => {
            // https://github.com/ethereum/devp2p/blob/master/caps/eth.md#transactions-0x02
            if !txs.transactions.is_empty() {
                state.received_txs_from_peer = true;
            }
            if state.blockchain.is_synced() {
                let tx_hashes: Vec<_> = txs
                    .transactions
                    .iter()
                    .map(|tx| tx.hash(&NativeCrypto))
                    .collect();

                // Tell the broadcaster right away that the peer knows these, before
                // any of them can reach the pool: it only tracks hashes to avoid
                // re-broadcasting to the sender, and the actual broadcast happens on
                // a periodic timer that queries the mempool directly. Those the pool
                // does not keep are forgotten again once insertion is done.
                let peer_id = state.node.node_id();
                let marked = state.tx_broadcaster.add_txs(tx_hashes.clone(), peer_id);

                // Offload pool insertion to a background task so we don't block
                // the ConnectionServer (validation + signature recovery are expensive).
                let blockchain = state.blockchain.clone();
                let tx_broadcaster = state.tx_broadcaster.clone();
                let peer = state.node.to_string();
                #[cfg(feature = "l2")]
                let is_l2_mode = state.l2_state.is_supported();
                tokio::spawn(async move {
                    for tx in txs.transactions {
                        #[cfg(feature = "l2")]
                        if (is_l2_mode && matches!(tx, Transaction::EIP4844Transaction(_)))
                            || tx.is_privileged()
                        {
                            let tx_type = tx.tx_type();
                            debug!(peer=%peer, "Rejecting transaction in L2 mode - {tx_type} transactions are not broadcasted in L2");
                            continue;
                        }

                        if let Err(e) = blockchain.add_transaction_to_pool(tx).await {
                            debug!(
                                peer=%peer,
                                error=%e,
                                "Error adding transaction"
                            );
                        }
                    }
                    let not_admitted: Vec<H256> = tx_hashes
                        .into_iter()
                        .filter(|hash| !blockchain.mempool.contains_tx(*hash).unwrap_or(false))
                        .collect();
                    if !not_admitted.is_empty()
                        && let Err(e) = tx_broadcaster.forget_txs(not_admitted, peer_id)
                    {
                        debug!(peer=%peer, error=%e, "Failed to forget unadmitted transactions");
                    }
                });

                marked.map_err(|e| PeerConnectionError::BroadcastError(e.to_string()))?;
            }
        }
        Message::GetBlockHeaders(msg_data) if peer_supports_eth => {
            let response = BlockHeaders {
                id: msg_data.id,
                block_headers: msg_data.fetch_headers(&state.storage).await,
            };
            send(state, Message::BlockHeaders(response)).await?;
        }
        Message::GetBlockBodies(msg_data) if peer_supports_eth => {
            let response = BlockBodies {
                id: msg_data.id,
                block_bodies: msg_data.fetch_blocks(&state.storage).await,
            };
            send(state, Message::BlockBodies(response)).await?;
        }
        Message::GetBlockAccessLists(GetBlockAccessLists { id, block_hashes })
            if peer_supports_eth =>
        {
            // Offload synchronous storage/RLP/hashing work off the connection task,
            // as for the snap/2 variant below.
            let storage = state.storage.clone();
            let response = tokio::task::spawn_blocking(move || {
                build_block_access_lists_response(id, &block_hashes, &storage)
            })
            .await
            .map_err(|e| PeerConnectionError::InternalError(e.to_string()))?;
            send(state, Message::BlockAccessLists(response)).await?;
        }
        Message::GetReceipts68(GetReceipts68 { id, block_hashes }) if peer_supports_eth => {
            let receipts = build_receipts_response(&block_hashes, true, &state.storage).await?;
            send(state, Message::Receipts68(Receipts68::new(id, receipts))).await?;
        }
        Message::GetReceipts69(GetReceipts68 { id, block_hashes }) if peer_supports_eth => {
            let receipts = build_receipts_response(&block_hashes, false, &state.storage).await?;
            send(state, Message::Receipts69(Receipts69::new(id, receipts))).await?;
        }
        // EIP-7975: eth/70 partial receipt requests
        Message::GetReceipts70(GetReceipts70 {
            id,
            first_block_receipt_index,
            block_hashes,
        }) if peer_supports_eth => {
            let block_hashes = &block_hashes[..block_hashes.len().min(MAX_RECEIPTS_REQUEST_HASHES)];
            let mut all_receipts: Vec<Vec<Receipt>> = Vec::new();
            let mut total_size: usize = 0;
            let mut last_block_incomplete = false;

            for (i, hash) in block_hashes.iter().enumerate() {
                let start_index = if i == 0 { first_block_receipt_index } else { 0 };
                let block_receipts = state
                    .storage
                    .get_receipts_for_block_from_index(hash, start_index, None)
                    .await?;

                let mut block_receipt_list = Vec::new();
                let mut hit_limit = false;
                for receipt in block_receipts {
                    let receipt_size = receipt.length();
                    if total_size + receipt_size > SOFT_RESPONSE_LIMIT
                        && (!block_receipt_list.is_empty() || !all_receipts.is_empty())
                    {
                        hit_limit = true;
                        // Only mark incomplete when the current block actually
                        // has a partial receipt list. When the limit is hit
                        // before any receipt from this block fits, the previous
                        // block is complete — setting the flag would cause the
                        // peer to re-request an already-complete block.
                        if !block_receipt_list.is_empty() {
                            last_block_incomplete = true;
                        }
                        break;
                    }
                    total_size += receipt_size;
                    block_receipt_list.push(receipt);
                }

                // Don't push an empty list when the limit was hit before any
                // receipt from this block could be included — an empty trailing
                // list would mislead the peer into thinking the block has no
                // transactions.
                if !block_receipt_list.is_empty() || !hit_limit {
                    all_receipts.push(block_receipt_list);
                }

                if hit_limit {
                    break;
                }
            }

            let response =
                Message::Receipts70(Receipts70::new(id, last_block_incomplete, all_receipts));
            send(state, response).await?;
        }
        Message::BlockRangeUpdate(update) => {
            trace!(
                peer=%state.node,
                range_from=update.earliest_block,
                range_to=update.latest_block,
                "Block range update",
            );
            // We will only validate the incoming update, we may decide to store and use this information in the future
            if let Err(err) = update.validate() {
                debug!(
                    peer=%state.node,
                    reason=%err,
                    "Disconnecting peer: invalid block range update",
                );
                send_disconnect_message(state, Some(DisconnectReason::SubprotocolError)).await;
                return Err(PeerConnectionError::DisconnectSent(
                    DisconnectReason::SubprotocolError,
                ));
            }
        }
        Message::NewPooledTransactionHashes(new_pooled_transaction_hashes) if peer_supports_eth => {
            // Don't request transactions if we're not synced — we won't be building blocks soon.
            if state.blockchain.is_synced() {
                let hashes = new_pooled_transaction_hashes
                    .get_transactions_to_request(&state.blockchain, state.node.node_id())?;
                if !hashes.is_empty() {
                    // Buffer hashes for batched requesting instead of sending immediately.
                    // The periodic flush_pending_tx_requests handler will send them. Keep
                    // only the reserved entries, so the queue is bounded by the peer's
                    // announced-hash budget rather than by the announcement's size.
                    state
                        .pending_tx_requests
                        .push((new_pooled_transaction_hashes.filter_to(&hashes), hashes));
                }
            }
        }
        // eth/72 (EIP-8070): provider/sampler split based on blob_sampling_enabled.
        Message::NewPooledTransactionHashes72(announcement) if peer_supports_eth => {
            if state.blockchain.is_synced() {
                let peer_id = state.node.node_id();
                // Record peer cell availability from the announced mask, but only when
                // the announcement carries a blob tx: otherwise a peer sending an
                // all-zero mask on a non-blob announcement would overwrite its real
                // availability with an empty set and stop us sampling from it for good.
                // See `NewPooledTransactionHashes72::announces_blob_tx`.
                if announcement.announces_blob_tx()
                    && let Some(mask) = announcement.cell_mask
                    && let Err(e) = state
                        .blockchain
                        .mempool
                        .record_peer_cell_availability(peer_id, mask)
                {
                    warn!(error = %e, "record_peer_cell_availability failed");
                }

                let hashes =
                    announcement.get_transactions_to_request(&state.blockchain, peer_id)?;
                let records_providers = state.blockchain.mempool.blob_sampling_enabled
                    && announcement.cell_mask == Some(u128::MAX);
                if !hashes.is_empty() || records_providers {
                    let roles = BlobRoles::new(state);
                    if records_providers {
                        record_blob_providers(state, &roles, &announcement, &hashes);
                    }
                    queue_requests_72(state, &roles, &announcement, hashes);
                }
            }
        }
        Message::GetPooledTransactions(msg) => {
            // eth/72: respond with PooledTransactions72 (elided blob payload).
            let is_eth72 = state
                .negotiated_eth_capability
                .as_ref()
                .is_some_and(|cap| cap.version == 72);
            let response = msg.handle(&state.blockchain, is_eth72)?;
            let batch_size = response.pooled_transactions.len() as u64;
            // Leech detection: disconnect peers that drain transactions but never contribute any.
            if state.txs_sent_to_peer + batch_size > LEECH_TX_SENT_THRESHOLD
                && !state.received_txs_from_peer
            {
                debug!(
                    peer = %state.node,
                    txs_sent = state.txs_sent_to_peer,
                    "Disconnecting peer: leech detected (sent many txs but received none)",
                );
                send_disconnect_message(state, Some(DisconnectReason::UselessPeer)).await;
                return Err(PeerConnectionError::DisconnectSent(
                    DisconnectReason::UselessPeer,
                ));
            }
            if is_eth72 {
                let response72 =
                    PooledTransactions72::new(response.id, response.pooled_transactions);
                send(state, Message::PooledTransactions72(response72)).await?;
            } else {
                // A blob tx ingested over eth/72 may sit in the pool with an
                // empty bundle (cells fetched separately). Pre-72 responses
                // carry blob txs with their full payload, and a wrapper whose
                // blob count disagrees with its commitments is a protocol
                // violation the peer disconnects us for — skip those instead,
                // which `GetPooledTransactions` explicitly allows.
                let mut response = response;
                response.pooled_transactions.retain(|tx| match tx {
                    P2PTransaction::EIP4844TransactionWithBlobs(wrapped) => {
                        wrapped.blobs_bundle.blobs.len() == wrapped.blobs_bundle.commitments.len()
                    }
                    _ => true,
                });
                send(state, Message::PooledTransactions(response)).await?;
            }
            state.txs_sent_to_peer += batch_size;
        }
        Message::PooledTransactions(msg) if peer_supports_eth => {
            if !msg.pooled_transactions.is_empty() {
                state.received_txs_from_peer = true;
            }
            // Always clear in-flight tracking for this response, regardless of sync status,
            // so other connections can re-request these hashes if needed.
            let removed_request = state.requested_pooled_txs.remove(&msg.id);
            if let Some((_, ref requested_hashes, _)) = removed_request {
                state
                    .blockchain
                    .mempool
                    .clear_in_flight_txs(requested_hashes)?;
            }
            // If we receive a blob transaction without blobs or with blobs that don't match the versioned hashes we must disconnect from the peer
            for tx in &msg.pooled_transactions {
                if let P2PTransaction::EIP4844TransactionWithBlobs(itx) = tx
                    && (itx.blobs_bundle.is_empty()
                        || itx
                            .blobs_bundle
                            .validate_blob_commitment_hashes(&itx.tx.blob_versioned_hashes)
                            .is_err())
                {
                    debug!(
                        peer=%state.node,
                        "Disconnecting peer: invalid or missing blobs",
                    );
                    if let Some((_announced, requested_hashes, _)) = &removed_request {
                        retry_on_alternates(&state.blockchain, &state.peer_table, requested_hashes)
                            .await;
                    }
                    send_disconnect_message(state, Some(DisconnectReason::SubprotocolError)).await;
                    return Err(PeerConnectionError::DisconnectSent(
                        DisconnectReason::SubprotocolError,
                    ));
                }
            }
            if state.blockchain.is_synced() {
                let mut delivered = None;
                if let Some((announced, requested_hashes, _)) = &removed_request {
                    let fork = state.blockchain.current_fork()?;
                    let sizes = match msg.validate_requested(announced, fork) {
                        Ok(sizes) => sizes,
                        Err(error) => {
                            debug!(
                                peer=%state.node,
                                reason=%error,
                                "Disconnecting peer: invalid pooled transactions response",
                            );
                            retry_on_alternates(
                                &state.blockchain,
                                &state.peer_table,
                                requested_hashes,
                            )
                            .await;
                            send_disconnect_message(
                                state,
                                Some(DisconnectReason::SubprotocolError),
                            )
                            .await;
                            return Err(PeerConnectionError::DisconnectSent(
                                DisconnectReason::SubprotocolError,
                            ));
                        }
                    };
                    account_tx_answer(state, &announced.transaction_hashes, &sizes).await?;
                    delivered = Some(sizes);
                }
                #[cfg(feature = "l2")]
                let is_l2_mode = state.l2_state.is_supported();

                #[cfg(not(feature = "l2"))]
                let is_l2_mode = false;
                if let Err(error) = msg.handle(&state.node, &state.blockchain, is_l2_mode).await {
                    if matches!(
                        error,
                        ethrex_blockchain::error::MempoolError::BlobsBundleError(_)
                    ) {
                        debug!(
                            peer=%state.node,
                            reason=%error,
                            "Disconnecting peer: invalid pooled transactions response",
                        );
                        if let Some((_announced, requested_hashes, _)) = &removed_request {
                            retry_on_alternates(
                                &state.blockchain,
                                &state.peer_table,
                                requested_hashes,
                            )
                            .await;
                        }
                        send_disconnect_message(state, Some(DisconnectReason::SubprotocolError))
                            .await;
                        return Err(PeerConnectionError::DisconnectSent(
                            DisconnectReason::SubprotocolError,
                        ));
                    }
                    return Err(error.into());
                }
                if let Some(delivered) = delivered {
                    forget_delivered_alternates(state, delivered);
                }
            }
        }
        // eth/72 (EIP-8070): PooledTransactions72 handler.
        // Blob txs arrive with elided blobs — do NOT trigger the missing-blob disconnect.
        Message::PooledTransactions72(msg) if peer_supports_eth => {
            if !msg.pooled_transactions.is_empty() {
                state.received_txs_from_peer = true;
            }
            let removed_request = state.requested_pooled_txs_72.remove(&msg.id);
            if let Some((_, ref requested_hashes, _, _)) = removed_request {
                state
                    .blockchain
                    .mempool
                    .clear_in_flight_txs(requested_hashes)?;
            }
            if state.blockchain.is_synced() {
                let mut delivered = None;
                if let Some((announced, requested_hashes, _, _)) = &removed_request {
                    let fork = state.blockchain.current_fork()?;
                    let sizes = match msg.validate_requested(announced, fork) {
                        Ok(sizes) => sizes,
                        Err(error) => {
                            warn!(
                                peer=%state.node,
                                reason=%error,
                                "disconnected from peer (eth/72 PooledTransactions72)",
                            );
                            retry_on_alternates(
                                &state.blockchain,
                                &state.peer_table,
                                requested_hashes,
                            )
                            .await;
                            send_disconnect_message(
                                state,
                                Some(DisconnectReason::SubprotocolError),
                            )
                            .await;
                            return Err(PeerConnectionError::DisconnectSent(
                                DisconnectReason::SubprotocolError,
                            ));
                        }
                    };
                    account_tx_answer(state, &announced.transaction_hashes, &sizes).await?;
                    delivered = Some(sizes);
                }
                #[cfg(feature = "l2")]
                let is_l2_mode = state.l2_state.is_supported();
                #[cfg(not(feature = "l2"))]
                let is_l2_mode = false;
                if let Err(error) = msg.handle(&state.node, &state.blockchain, is_l2_mode).await {
                    if matches!(
                        error,
                        ethrex_blockchain::error::MempoolError::BlobsBundleError(_)
                    ) {
                        warn!(
                            peer=%state.node,
                            reason=%error,
                            "disconnected from peer (eth/72 blob error)",
                        );
                        if let Some((_announced, requested_hashes, _, _)) = &removed_request {
                            retry_on_alternates(
                                &state.blockchain,
                                &state.peer_table,
                                requested_hashes,
                            )
                            .await;
                        }
                        send_disconnect_message(state, Some(DisconnectReason::SubprotocolError))
                            .await;
                        return Err(PeerConnectionError::DisconnectSent(
                            DisconnectReason::SubprotocolError,
                        ));
                    }
                    return Err(error.into());
                }
                if let Some(delivered) = delivered {
                    forget_delivered_alternates(state, delivered);
                }
                // EIP-8070 sampler: after tx validation, check if we have enough provider
                // announcements to start fetching cells.
                if state.blockchain.mempool.blob_sampling_enabled {
                    let peer_id = state.node.node_id();
                    let mempool = &state.blockchain.mempool;
                    let local_pubkey = public_key_from_signing_key(&state.signer);
                    let local_node_id = node_id(&local_pubkey);
                    if let Some((announced, requested_hashes, role, _)) = &removed_request {
                        // Provider role: the eth/72 PooledTransactions response is
                        // always elided, so the blobs never arrive with the body.
                        // A provider must end up holding the full payload (EIP-8070),
                        // and the only way to obtain it on an eth/72 connection is
                        // GetCells — request every column from this peer, which
                        // advertised full availability. Cells already held are
                        // subtracted at flush time.
                        if *role == BlobFetchRole::Provider {
                            let blob_hashes: Vec<H256> = announced
                                .transaction_types
                                .iter()
                                .zip(announced.transaction_hashes.iter())
                                .filter(|&(&ty, hash)| ty == 3 && requested_hashes.contains(hash))
                                .map(|(_, &hash)| hash)
                                .collect();
                            if !blob_hashes.is_empty() {
                                state.pending_cell_requests.push((blob_hashes, u128::MAX));
                            }
                        }
                        if *role == BlobFetchRole::Sampler {
                            for &tx_hash in requested_hashes.iter() {
                                // Providers were recorded at announce time (provider-gated);
                                // here we only read the distinct-provider count to decide
                                // whether the 2-provider sampling threshold is met.
                                let count = match mempool.provider_announcer_count(tx_hash) {
                                    Ok(n) => n,
                                    Err(e) => {
                                        warn!(error = %e, "provider_announcer_count failed");
                                        continue;
                                    }
                                };
                                if count
                                    >= ethrex_blockchain::mempool::MIN_PROVIDERS_BEFORE_SAMPLING
                                {
                                    // Compute target columns = custody | extra.
                                    let custody = match mempool.get_custody_columns() {
                                        Ok(c) => c,
                                        Err(e) => {
                                            warn!(error = %e, "get_custody_columns failed");
                                            continue;
                                        }
                                    };
                                    // C_extra is only added when the target peer
                                    // is a provider (advertised full availability).
                                    let peer_mask = mempool
                                        .peer_cell_mask(peer_id)
                                        .unwrap_or(None)
                                        .unwrap_or(u128::MAX);
                                    let mut target = custody;
                                    if peer_mask == u128::MAX
                                        && let Some(extra_col) = pick_random_extra_column(
                                            custody,
                                            local_node_id,
                                            tx_hash,
                                        )
                                    {
                                        target |= 1u128 << extra_col;
                                    }
                                    let fetch_mask = target & peer_mask;
                                    if fetch_mask != 0 {
                                        state
                                            .pending_cell_requests
                                            .push((vec![tx_hash], fetch_mask));
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        // eth/72 (EIP-8070): GetCells handler — serve cells we hold.
        Message::GetCells(req) if peer_supports_eth => {
            // Serving cells expands every blob of every served tx through KZG,
            // which is CPU-bound; keep it off the connection task and the shared
            // runtime workers, as for `GetBlockAccessLists`.
            let blockchain = state.blockchain.clone();
            let response = tokio::task::spawn_blocking(move || req.handle(&blockchain.mempool))
                .await
                .map_err(|e| PeerConnectionError::InternalError(e.to_string()))?;
            send(state, Message::Cells(response)).await?;
        }
        // eth/72 (EIP-8070): Cells ingest — verify received cells against the
        // sidecar KZG proofs (carried in the elided PooledTransactions72), then
        // store. Cells that fail verification, or lack a matching sidecar proof,
        // cause a disconnect: accepting unverified cells would defeat DAS.
        ref message @ Message::Cells(_) if peer_supports_eth => {
            #[allow(unused_mut)]
            let mut verify_failed = false;
            // devp2p `caps/eth.md`: a `Cells` response must answer an outstanding
            // `GetCells`, and both its hash list and its `cells` bitmap must be
            // subsets of that request's. A peer sending unrequested elements must be
            // disconnected — unlike a KZG failure this is checkable without c-kzg,
            // and it stops a peer from pushing cells we never asked for.
            if let Message::Cells(cells_msg) = message {
                let outcome = match state.requested_cells.remove(&cells_msg.id) {
                    Some((requested_hashes, requested_mask, _)) => {
                        cells_msg.validate_requested(&requested_hashes, requested_mask)
                    }
                    None => Err(CellsResponseError::UnknownRequestId),
                };
                if let Err(error) = outcome {
                    debug!(
                        peer = %state.node,
                        id = cells_msg.id,
                        %error,
                        "Rejecting Cells response",
                    );
                    verify_failed = true;
                }
            }
            // Skip the KZG pass when the response already failed the framing checks:
            // nothing from an unrequested response should reach the cell store.
            #[cfg(feature = "c-kzg")]
            if let Message::Cells(cells_msg) = message
                && !verify_failed
            {
                use ethrex_common::types::{BYTES_PER_CELL, CELLS_PER_EXT_BLOB, Commitment, Proof};
                use ethrex_crypto::kzg::verify_cell_kzg_proof_batch_partial;
                let mempool = &state.blockchain.mempool;
                // Columns carried in this response (ascending order of set bits).
                let cols: Vec<u64> = (0..CELLS_PER_EXT_BLOB as u64)
                    .filter(|bit| (cells_msg.cell_mask >> bit) & 1 == 1)
                    .collect();

                'txs: for (tx_idx, tx_hash) in cells_msg.transaction_hashes.iter().enumerate() {
                    let Some(bundle) = mempool.get_blobs_bundle(*tx_hash).unwrap_or(None) else {
                        continue;
                    };
                    // Cells are packed index-major per devp2p `caps/eth.md`:
                    // [col0 over blobs, col1 over blobs, ...].
                    let tx_cells = cells_msg.cells.get(tx_idx).cloned().unwrap_or_default();
                    let blob_count = bundle.commitments.len();

                    let mut to_store: Vec<(usize, usize, Box<[u8; BYTES_PER_CELL]>)> = Vec::new();
                    let mut v_commitments: Vec<Commitment> = Vec::new();
                    let mut v_cols: Vec<u64> = Vec::new();
                    let mut v_cells: Vec<[u8; BYTES_PER_CELL]> = Vec::new();
                    let mut v_proofs: Vec<Proof> = Vec::new();

                    for blob_idx in 0..blob_count {
                        for (col_pos, &col) in cols.iter().enumerate() {
                            let cell_pos = col_pos * blob_count + blob_idx;
                            let Some(&cell) = tx_cells.get(cell_pos) else {
                                continue;
                            };
                            // Sidecar cell proof for (blob_idx, col).
                            let proof_idx = blob_idx * CELLS_PER_EXT_BLOB + col as usize;
                            match (
                                bundle.proofs.get(proof_idx),
                                bundle.commitments.get(blob_idx),
                            ) {
                                (Some(&proof), Some(&commitment)) => {
                                    v_commitments.push(commitment);
                                    v_cols.push(col);
                                    v_cells.push(cell);
                                    v_proofs.push(proof);
                                    to_store.push((blob_idx, col as usize, Box::new(cell)));
                                }
                                // A received cell with no matching sidecar proof can't be trusted.
                                _ => {
                                    verify_failed = true;
                                    break 'txs;
                                }
                            }
                        }
                    }

                    if v_cells.is_empty() {
                        continue;
                    }
                    match verify_cell_kzg_proof_batch_partial(
                        &v_commitments,
                        &v_cols,
                        &v_cells,
                        &v_proofs,
                    ) {
                        Ok(true) => {}
                        Ok(false) | Err(_) => {
                            verify_failed = true;
                            break;
                        }
                    }
                    if let Err(e) = mempool.store_cells(*tx_hash, blob_count, to_store) {
                        warn!(error = %e, "store_cells failed");
                    }
                }
            }
            if verify_failed {
                warn!(peer=%state.node, "disconnected: invalid cell KZG proofs");
                if let Message::Cells(c) = message {
                    let hashes = c.transaction_hashes.clone();
                    retry_on_alternates(&state.blockchain, &state.peer_table, &hashes).await;
                }
                send_disconnect_message(state, Some(DisconnectReason::SubprotocolError)).await;
                return Err(PeerConnectionError::DisconnectSent(
                    DisconnectReason::SubprotocolError,
                ));
            }
        }
        Message::GetStorageRanges(req) => {
            let response = process_storage_ranges_request(req, state.storage.clone()).await?;
            send(state, Message::StorageRanges(response)).await?
        }
        Message::GetByteCodes(req) => {
            let storage_clone = state.storage.clone();
            let response = process_byte_codes_request(req, storage_clone)
                .await
                .map_err(|_| {
                    PeerConnectionError::InternalError(
                        "Failed to execute bytecode retrieval task".to_string(),
                    )
                })?;
            send(state, Message::ByteCodes(response)).await?
        }
        Message::GetTrieNodes(req) => {
            let id = req.id;
            match process_trie_nodes_request(req, state.storage.clone()).await {
                Ok(response) => send(state, Message::TrieNodes(response)).await?,
                Err(_) => send(state, Message::TrieNodes(TrieNodes { id, nodes: vec![] })).await?,
            }
        }
        Message::Snap2GetBlockAccessLists(req) => {
            // Defense-in-depth: only serve if the peer negotiated snap/2.
            if state.negotiated_snap_capability != Some(Capability::snap(2)) {
                warn!(
                    peer = %state.node,
                    "Received Snap2GetBlockAccessLists from peer that did not negotiate snap/2; disconnecting"
                );
                send_disconnect_message(state, Some(DisconnectReason::ProtocolError)).await;
                return Err(PeerConnectionError::DisconnectSent(
                    DisconnectReason::ProtocolError,
                ));
            }
            // Offload synchronous storage/RLP work off the connection task
            // so other peers/messages keep flowing on this tokio worker.
            let storage = state.storage.clone();
            let response =
                tokio::task::spawn_blocking(move || build_snap2_bal_response(req, &storage))
                    .await
                    .map_err(|e| PeerConnectionError::InternalError(e.to_string()))??;
            send(state, Message::Snap2BlockAccessLists(response)).await?
        }
        #[cfg(feature = "l2")]
        Message::L2(req) if peer_supports_l2 => {
            handle_based_capability_message(state, req).await?;
        }
        // Send response messages to the backend
        message @ Message::AccountRange(_)
        | message @ Message::StorageRanges(_)
        | message @ Message::ByteCodes(_)
        | message @ Message::TrieNodes(_)
        | message @ Message::BlockBodies(_)
        | message @ Message::BlockHeaders(_)
        | message @ Message::Receipts68(_)
        | message @ Message::Receipts69(_)
        | message @ Message::Receipts70(_)
        | message @ Message::BlockAccessLists(_)
        | message @ Message::Snap2BlockAccessLists(_) => {
            if let Some((_, tx)) = message
                .request_id()
                .and_then(|id| state.current_requests.remove(&id))
            {
                tx.send(message)
                    .map_err(|e| PeerConnectionError::SendMessage(e.to_string()))?
            } else {
                return Err(PeerConnectionError::ExpectedRequestId(format!("{message}")));
            }
        }
        // TODO: Add new message types and handlers as they are implemented
        message => return Err(PeerConnectionError::MessageNotHandled(format!("{message}"))),
    };
    Ok(())
}

/// Build the receipt lists of an eth/68 or eth/69 `Receipts` reply to a `GetReceipts` request.
///
/// Entries follow the request order, one list per served hash; an unknown block gets an
/// empty list. At most [`MAX_RECEIPTS_REQUEST_HASHES`] hashes are served, a hash repeated
/// within one request is read from storage once, and the reply stops growing once the
/// encoded size of its receipts reaches [`SOFT_RESPONSE_LIMIT`] (the first list is always
/// served), as the eth/70 arm already does. `with_bloom` says whether each receipt goes
/// out with its logs bloom (eth/68) or without (eth/69), which the budget accounts for.
/// Without these bounds, one request naming a heavy block's hash tens of thousands of
/// times made the node load and encode that block's receipts once per repetition until
/// it was killed for running out of memory.
pub async fn build_receipts_response(
    block_hashes: &[H256],
    with_bloom: bool,
    storage: &Store,
) -> Result<Vec<Vec<Receipt>>, StoreError> {
    let hashes = &block_hashes[..block_hashes.len().min(MAX_RECEIPTS_REQUEST_HASHES)];
    // Where each served hash's list sits in the reply, so a repetition copies it from
    // there instead of reading it again.
    let mut served: HashMap<H256, usize> = HashMap::new();
    let mut receipts: Vec<Vec<Receipt>> = Vec::with_capacity(hashes.len());
    let mut total_size: usize = 0;
    for hash in hashes {
        let block_receipts = match served.get(hash) {
            Some(&index) => receipts[index].clone(),
            None => storage.get_receipts_for_block(hash).await?,
        };
        let size: usize = block_receipts
            .iter()
            .map(|receipt| receipt_wire_size(receipt, with_bloom))
            .sum();
        if !receipts.is_empty() && total_size + size > SOFT_RESPONSE_LIMIT {
            break;
        }
        total_size += size;
        served.insert(*hash, receipts.len());
        receipts.push(block_receipts);
    }
    Ok(receipts)
}

/// What the 256-byte logs bloom adds to a receipt's encoding: the bloom's bytes, its 3-byte
/// RLP string header, and up to 5 bytes the headers of the items around it grow by. An upper
/// bound, so the budget never under-counts an eth/68 item.
const ENCODED_BLOOM_SIZE: usize = ethereum_types::Bloom::len_bytes() + 8;

/// Encoded size of one receipt in a `Receipts` reply. eth/68 items carry the logs bloom
/// that eth/69 (EIP-7642) dropped; it is counted without being computed.
fn receipt_wire_size(receipt: &Receipt, with_bloom: bool) -> usize {
    receipt.length() + if with_bloom { ENCODED_BLOOM_SIZE } else { 0 }
}

/// Build a `BlockAccessLists` (eth/71, EIP-8159) response for a `GetBlockAccessLists` request.
///
/// Entries follow the request order; a block access list that is unknown, or does not
/// match its header's `block_access_list_hash`, is returned as unavailable (`None`).
/// The response stops growing once its encoded size reaches
/// [`BAL_RESPONSE_SOFT_CAP_BYTES`] (the first entry is always served), like the other
/// data-serving replies: a request may name up to [`BLOCK_ACCESS_LIST_LIMIT`] hashes, and
/// without a byte budget a request repeating the hash of one large block access list
/// would make the node read, verify and encode that list once per repetition. A hash
/// repeated within one request is read and verified once.
pub fn build_block_access_lists_response(
    id: u64,
    block_hashes: &[H256],
    storage: &ethrex_storage::Store,
) -> BlockAccessLists {
    use crate::rlpx::eth::block_access_lists::BLOCK_ACCESS_LIST_LIMIT;
    let hashes = &block_hashes[..block_hashes.len().min(BLOCK_ACCESS_LIST_LIMIT)];
    let mut verified: HashMap<H256, Option<BlockAccessList>> = HashMap::new();
    let mut block_access_lists = Vec::with_capacity(hashes.len());
    let mut bytes_used: u64 = 0;
    for hash in hashes {
        if !block_access_lists.is_empty() && bytes_used >= BAL_RESPONSE_SOFT_CAP_BYTES {
            break;
        }
        let bal = verified
            .entry(*hash)
            .or_insert_with(|| verified_block_access_list(*hash, storage))
            .clone();
        bytes_used += bal.as_ref().map_or(1, |bal| bal.length() as u64);
        block_access_lists.push(bal);
    }
    BlockAccessLists::new(id, block_access_lists)
}

/// EIP-8159: only serve a BAL that matches the block header's commitment. A stored BAL
/// that doesn't hash to the header's `block_access_list_hash` (e.g. a stale/empty entry
/// from a prior regeneration) is reported as unavailable (`0x80`) rather than served as
/// a wrong BAL, which receiving peers would reject.
fn verified_block_access_list(
    hash: H256,
    storage: &ethrex_storage::Store,
) -> Option<BlockAccessList> {
    match storage.get_block_access_list(hash) {
        Ok(Some(bal)) => {
            let commitment = match storage.get_block_header_by_hash(hash) {
                Ok(Some(header)) => header.block_access_list_hash,
                Ok(None) => None,
                Err(err) => {
                    // Don't serve an unverified BAL: degrade to 0x80 (unavailable), but
                    // log so an operator can tell a committed BAL was refused due to a
                    // DB error.
                    warn!(
                        "Failed to read header for BAL commitment check (hash {hash:#x}): {err}; reporting BAL unavailable"
                    );
                    None
                }
            };
            bal.matches_commitment(commitment, &NativeCrypto)
                .then_some(bal)
        }
        Ok(None) => None,
        Err(err) => {
            error!("Error accessing DB while building BAL response for peer: {err}");
            None
        }
    }
}

/// Build a `Snap2BlockAccessLists` response for a `Snap2GetBlockAccessLists` request.
///
/// Per EIP-8189:
/// - §50: always respond; push `None` for unknown/pruned/pre-Amsterdam blocks.
/// - §51: truncate from tail (preserve order) once the byte budget is exceeded.
/// - §52: orphaned blocks are served the same way as canonical blocks (keyed by hash).
/// - §60: enforce `min(response_bytes, 2 MiB)`; `0` means 2 MiB.
/// - §100: push `None` for blocks whose header has `block_access_list_hash == None`.
pub fn build_snap2_bal_response(
    req: Snap2GetBlockAccessLists,
    storage: &ethrex_storage::Store,
) -> Result<Snap2BlockAccessLists, PeerConnectionError> {
    let cap = if req.response_bytes == 0 {
        BAL_RESPONSE_SOFT_CAP_BYTES
    } else {
        req.response_bytes.min(BAL_RESPONSE_SOFT_CAP_BYTES)
    };

    // Defend against tiny-BAL flood DoS: truncate hash list before doing any
    // storage work. Matches go-ethereum's `maxAccessListLookups`.
    let hashes: &[H256] = if req.block_hashes.len() > BAL_MAX_REQUEST_HASHES {
        &req.block_hashes[..BAL_MAX_REQUEST_HASHES]
    } else {
        &req.block_hashes
    };

    // No per-hash header lookup is needed to satisfy §100: a stored BAL implies the block
    // was admitted post-Amsterdam (BAL storage is gated on the Amsterdam fork), so a
    // present entry is served directly. When no BAL is stored — pre-Amsterdam, pruned, or
    // unknown block — the slot is `None` regardless. Lists are read one at a time and only
    // until the byte budget is spent, so the budget also bounds what gets decoded.
    let mut bals: Vec<Option<BlockAccessList>> = Vec::with_capacity(hashes.len());
    let mut bytes_used: u64 = 0;

    for hash in hashes {
        // Keep at least one entry then stop once cap is exceeded.
        if !bals.is_empty() && bytes_used >= cap {
            break;
        }
        let raw_bal = storage
            .get_block_access_list(*hash)
            .map_err(|e| PeerConnectionError::InternalError(e.to_string()))?;

        bytes_used += match &raw_bal {
            Some(bal) => bal.length() as u64,
            None => 1, // RLP empty string (0x80) = 1 byte
        };
        bals.push(raw_bal);
    }

    Ok(Snap2BlockAccessLists { id: req.id, bals })
}

async fn handle_outgoing_message(
    state: &mut Established,
    message: Message,
) -> Result<(), PeerConnectionError> {
    trace!(
        peer=%state.node,
        %message,
        "Sending message"
    );
    send(state, message).await?;
    Ok(())
}

async fn handle_outgoing_request(
    state: &mut Established,
    message: Message,
    sender: oneshot::Sender<Message>,
) -> Result<(), PeerConnectionError> {
    // Insert the request in the request map if it supports a request id.
    message.request_id().and_then(|id| {
        state
            .current_requests
            .insert(id, (format!("{message}"), sender))
    });
    trace!(
        peer=%state.node,
        %message,
        "Sending request"
    );
    send(state, message).await?;
    Ok(())
}

async fn handle_broadcast(
    state: &mut Established,
    (id, broadcasted_msg): (task::Id, Arc<Message>),
) -> Result<(), PeerConnectionError> {
    if id != tokio::task::id() {
        match broadcasted_msg.as_ref() {
            #[cfg(feature = "l2")]
            l2_msg @ Message::L2(_) => {
                handle_l2_broadcast(state, l2_msg).await?;
            }
            msg => {
                error!(
                    peer=%state.node,
                    message=%msg,
                    "Non-supported message broadcasted"
                );
                let error_message = format!("Non-supported message broadcasted: {msg}");
                return Err(PeerConnectionError::BroadcastError(error_message));
            }
        }
    }
    Ok(())
}

async fn handle_block_range_update(state: &mut Established) -> Result<(), PeerConnectionError> {
    if should_send_block_range_update(state)? {
        send_block_range_update(state).await
    } else {
        Ok(())
    }
}

/// What this node takes on for each transaction announced over eth/72 (EIP-8070). With blob
/// sampling off, every transaction is fetched whole. With it on, a blob transaction is fetched
/// whole, with all its cells, only from a peer claiming every column and when this node is a
/// provider for it; otherwise only its body is fetched and its cells are sampled.
struct BlobRoles {
    sampling: bool,
    local_node_id: H256,
    epoch_seed: u64,
    eager: bool,
}

impl BlobRoles {
    fn new(state: &Established) -> Self {
        let sampling = state.blockchain.mempool.blob_sampling_enabled;
        let epoch_seed = if sampling {
            match state.storage.get_latest_block_number() {
                Ok(n) => n / 32,
                Err(e) => {
                    warn!(error = %e, "eth/72: head block unavailable; using epoch 0 for role split");
                    0
                }
            }
        } else {
            0
        };
        Self {
            sampling,
            // Per-node entropy for the provider decision.
            local_node_id: node_id(&public_key_from_signing_key(&state.signer)),
            epoch_seed,
            eager: state.blockchain.mempool.is_eager_provider(),
        }
    }

    /// The role for `hash`, announced as `tx_type` by a peer claiming `cell_mask`. Only a
    /// type-3 transaction announced with a mask counts as a blob transaction.
    fn role(&self, hash: H256, tx_type: u8, cell_mask: Option<u128>) -> BlobFetchRole {
        let is_blob = cell_mask.is_some() && tx_type == 3;
        if !self.sampling
            || !is_blob
            || (cell_mask == Some(u128::MAX)
                && is_provider_role(self.local_node_id, hash, self.epoch_seed, self.eager))
        {
            BlobFetchRole::Provider
        } else {
            BlobFetchRole::Sampler
        }
    }

    /// The availability a request in `role` is queued with: with sampling on, a provider
    /// request asks for every column and a sampler request claims none; with it off, the
    /// announcer's own.
    fn request_mask(&self, role: BlobFetchRole, announced: Option<u128>) -> Option<u128> {
        match (self.sampling, role) {
            (false, _) => announced,
            (true, BlobFetchRole::Provider) => Some(u128::MAX),
            (true, BlobFetchRole::Sampler) => None,
        }
    }
}

/// Queues the eth/72 requests for `hashes`, those reserved from `announcement`, one per role
/// (see `BlobRoles`), keeping each hash's announced type and size. Walks the announcement once.
fn queue_requests_72(
    state: &mut Established,
    roles: &BlobRoles,
    announcement: &NewPooledTransactionHashes72,
    hashes: Vec<H256>,
) {
    if hashes.is_empty() {
        return;
    }
    /// The announced entries going into one role's request.
    #[derive(Default)]
    struct Entries {
        types: Vec<u8>,
        sizes: Vec<usize>,
        hashes: Vec<H256>,
    }
    let mut reserved: HashSet<H256> = hashes.into_iter().collect();
    let (mut provider, mut sampler) = (Entries::default(), Entries::default());
    for ((&tx_type, &size), &hash) in announcement
        .transaction_types
        .iter()
        .zip(&announcement.transaction_sizes)
        .zip(&announcement.transaction_hashes)
    {
        // Removing it also skips a hash announced twice, keeping its first entry.
        if !reserved.remove(&hash) {
            continue;
        }
        let entries = match roles.role(hash, tx_type, announcement.cell_mask) {
            BlobFetchRole::Provider => &mut provider,
            BlobFetchRole::Sampler => &mut sampler,
        };
        entries.types.push(tx_type);
        entries.sizes.push(size);
        entries.hashes.push(hash);
    }
    for (role, entries) in [
        (BlobFetchRole::Provider, provider),
        (BlobFetchRole::Sampler, sampler),
    ] {
        if entries.hashes.is_empty() {
            continue;
        }
        let request = NewPooledTransactionHashes72::from_raw(
            entries.types.into(),
            entries.sizes,
            entries.hashes.clone(),
            roles.request_mask(role, announcement.cell_mask),
        );
        state
            .pending_tx_requests_72
            .push((request, entries.hashes, role));
    }
}

/// Records the announcer of `announcement`, which claims every column, as a provider of each
/// blob transaction it announces, except those fetched whole from it (`reserved` from it with a
/// provider role): the sampler path waits for `MIN_PROVIDERS_BEFORE_SAMPLING` of them, however
/// many announced the transaction after it was first requested or pooled. When the threshold is
/// reached for a transaction already in the pool, its cells are requested from this peer.
fn record_blob_providers(
    state: &mut Established,
    roles: &BlobRoles,
    announcement: &NewPooledTransactionHashes72,
    reserved: &[H256],
) {
    let peer_id = state.node.node_id();
    let reserved: HashSet<&H256> = reserved.iter().collect();
    let mut seen = HashSet::new();
    let mempool = &state.blockchain.mempool;
    for (&tx_type, &hash) in announcement
        .transaction_types
        .iter()
        .zip(&announcement.transaction_hashes)
    {
        if tx_type != 3 || !seen.insert(hash) {
            continue;
        }
        if reserved.contains(&hash)
            && roles.role(hash, tx_type, announcement.cell_mask) == BlobFetchRole::Provider
        {
            continue;
        }
        let count = match mempool.record_provider_announcement(hash, peer_id) {
            Ok(n) => n,
            Err(e) => {
                warn!(error = %e, "record_provider_announcement failed");
                continue;
            }
        };
        // retrigger: threshold just reached and tx body already known.
        if count == ethrex_blockchain::mempool::MIN_PROVIDERS_BEFORE_SAMPLING
            && mempool.contains_tx(hash).unwrap_or(false)
        {
            let custody = match mempool.get_custody_columns() {
                Ok(c) => c,
                Err(e) => {
                    warn!(error = %e, "get_custody_columns failed (D6b)");
                    continue;
                }
            };
            // only add C_extra when this peer is a provider.
            let mut target = custody;
            if let Some(extra_col) = pick_random_extra_column(custody, roles.local_node_id, hash) {
                target |= 1u128 << extra_col;
            }
            // A provider holds every column, so the whole target can be requested from it.
            if target != 0 {
                state.pending_cell_requests.push((vec![hash], target));
            }
        }
    }
}

/// Queues a request to this peer for transactions another announcer did not deliver, built by
/// `retry_on_alternates` from this peer's own announcement of them. On an eth/72 session it goes
/// to the eth/72 queue, as the peer answers with `PooledTransactions72`, with each hash's role
/// taken from the cell availability the peer last announced.
fn queue_retried_request(state: &mut Established, announcement: &NewPooledTransactionHashes) {
    // Re-reserve in-flight against this peer. If any hashes are already
    // in-flight (race), drop them; we don't want duplicate requests.
    let to_request: Vec<H256> = match state.blockchain.mempool.reserve_unknown_hashes(
        &announcement.transaction_hashes,
        &announcement.transaction_types,
        &announcement.transaction_sizes,
        state.node.node_id(),
    ) {
        Ok(unknown) => unknown,
        Err(_) => return,
    };
    if to_request.is_empty() {
        return;
    }
    if !supports_eth72(state) {
        let trimmed = announcement.filter_to(&to_request);
        state.pending_tx_requests.push((trimmed, to_request));
        return;
    }
    let cell_mask = state
        .blockchain
        .mempool
        .peer_cell_mask(state.node.node_id())
        .unwrap_or(None);
    let announcement72 = NewPooledTransactionHashes72::from_raw(
        announcement.transaction_types.clone(),
        announcement.transaction_sizes.clone(),
        announcement.transaction_hashes.clone(),
        cell_mask,
    );
    let roles = BlobRoles::new(state);
    queue_requests_72(state, &roles, &announcement72, to_request);
}

/// Drains the pending transaction request buffer and sends batched
/// GetPooledTransactions requests, respecting the 256-hash-per-request
/// limit from the devp2p ETH spec.
async fn flush_pending_tx_requests(state: &mut Established) -> Result<(), PeerConnectionError> {
    if state.pending_tx_requests.is_empty() {
        return Ok(());
    }

    let pending = std::mem::take(&mut state.pending_tx_requests);

    // Queued announcements already hold only the hashes we're requesting, with their original
    // types and sizes for response validation.
    let mut all_hashes: Vec<H256> = Vec::new();
    let mut all_types: Vec<u8> = Vec::new();
    let mut all_sizes: Vec<usize> = Vec::new();

    for (announcement, _hashes) in pending {
        all_hashes.extend(announcement.transaction_hashes);
        all_types.extend_from_slice(&announcement.transaction_types);
        all_sizes.extend(announcement.transaction_sizes);
    }

    // Send in chunks of MAX_HASHES_PER_REQUEST per the devp2p spec.
    const MAX_HASHES_PER_REQUEST: usize = 256;
    for (i, chunk) in all_hashes.chunks(MAX_HASHES_PER_REQUEST).enumerate() {
        let offset = i * MAX_HASHES_PER_REQUEST;
        let chunk_types = &all_types[offset..offset + chunk.len()];
        let chunk_sizes = &all_sizes[offset..offset + chunk.len()];

        let announcement = NewPooledTransactionHashes::from_raw(
            chunk_types.to_vec().into(),
            chunk_sizes.to_vec(),
            chunk.to_vec(),
        );
        let request = GetPooledTransactions::new(random(), chunk.to_vec());
        let request_id = request.id;
        // Send first, only register in requested_pooled_txs on success.
        // This ensures we never track hashes for messages that were not transmitted.
        if let Err(e) = send(state, Message::GetPooledTransactions(request)).await {
            // Clear in-flight for the current chunk (failed to send) and all remaining chunks,
            // then try alternate announcers. Order matters: clear first so the alternate's
            // reserve_unknown_hashes sees the hashes as free.
            // Build an announcement covering every unsent hash (later chunks too) so the
            // alternate can validate its response against the original type/size metadata.
            let unsent = &all_hashes[offset..];
            if !unsent.is_empty() {
                if let Err(clear_err) = state.blockchain.mempool.clear_in_flight_txs(unsent) {
                    warn!(error = %clear_err, "Failed to clear in-flight transaction tracking after send error");
                }
                retry_on_alternates(&state.blockchain, &state.peer_table, unsent).await;
            }
            return Err(e);
        }
        state
            .requested_pooled_txs
            .insert(request_id, (announcement, chunk.to_vec(), Instant::now()));
    }

    Ok(())
}

/// eth/72 variant of flush_pending_tx_requests.
/// Sends GetPooledTransactions for v72 announcements and stores them in
/// requested_pooled_txs_72 for validation when PooledTransactions72 arrives.
///
/// Hashes are grouped by [`BlobFetchRole`] before chunking: the role decides
/// whether cells are fetched once the bodies arrive, so provider and sampler
/// hashes must never share a request.
///
/// Per-announcement cell_mask tracking: each hash carries the availability its
/// announcer claimed. When hashes are chunked across multiple requests, each
/// chunk's mask is the union of only the masks relevant to the hashes in that
/// chunk (not a global OR across all announcements).
async fn flush_pending_tx_requests_72(state: &mut Established) -> Result<(), PeerConnectionError> {
    if state.pending_tx_requests_72.is_empty() {
        return Ok(());
    }

    let pending = std::mem::take(&mut state.pending_tx_requests_72);

    // Flatten per-hash metadata into one run per role.
    let mut by_role: Vec<(BlobFetchRole, Vec<AnnouncedTx>)> = vec![
        (BlobFetchRole::Provider, Vec::new()),
        (BlobFetchRole::Sampler, Vec::new()),
    ];
    for (announcement, _hashes, role) in &pending {
        let Some((_, entries)) = by_role.iter_mut().find(|(r, _)| r == role) else {
            continue;
        };
        for (i, hash) in announcement.transaction_hashes.iter().enumerate() {
            entries.push((
                *hash,
                announcement.transaction_types[i],
                announcement.transaction_sizes[i],
                announcement.cell_mask,
            ));
        }
    }

    // Every hash still to be sent, in send order, so a failed write can release the
    // in-flight reservation for all of them and not just the current role's tail.
    let mut unsent: Vec<H256> = by_role
        .iter()
        .flat_map(|(_, entries)| entries.iter().map(|(hash, ..)| *hash))
        .collect();

    const MAX_HASHES_PER_REQUEST: usize = 256;
    for (role, entries) in &by_role {
        for chunk in entries.chunks(MAX_HASHES_PER_REQUEST) {
            let chunk_hashes: Vec<H256> = chunk.iter().map(|(hash, ..)| *hash).collect();
            // Compute the mask for this chunk only: OR of masks for hashes in this chunk.
            let chunk_cell_mask: Option<u128> =
                chunk
                    .iter()
                    .fold(None, |acc, (.., mask)| match (acc, mask) {
                        (None, None) => None,
                        (Some(a), None) => Some(a),
                        (None, Some(b)) => Some(*b),
                        (Some(a), Some(b)) => Some(a | b),
                    });

            let announcement = NewPooledTransactionHashes72::from_raw(
                chunk
                    .iter()
                    .map(|(_, ty, ..)| *ty)
                    .collect::<Vec<_>>()
                    .into(),
                chunk.iter().map(|(_, _, size, _)| *size).collect(),
                chunk_hashes.clone(),
                chunk_cell_mask,
            );
            let request = GetPooledTransactions::new(random(), chunk_hashes.clone());
            let request_id = request.id;
            if let Err(e) = send(state, Message::GetPooledTransactions(request)).await {
                if !unsent.is_empty() {
                    if let Err(clear_err) = state.blockchain.mempool.clear_in_flight_txs(&unsent) {
                        warn!(error = %clear_err, "clear_in_flight_txs failed after send error (v72)");
                    }
                    retry_on_alternates(&state.blockchain, &state.peer_table, &unsent).await;
                }
                return Err(e);
            }
            // Chunks are emitted in the order `unsent` was built, so the sent ones
            // are always its prefix.
            unsent.drain(..chunk_hashes.len());
            state.requested_pooled_txs_72.insert(
                request_id,
                (announcement, chunk_hashes, *role, Instant::now()),
            );
        }
    }

    Ok(())
}

/// EIP-8070: flush buffered cell requests as batched GetCells messages.
/// Mirrors flush_pending_tx_requests_72 but sends GetCells instead of
/// GetPooledTransactions.
async fn flush_pending_cell_requests(state: &mut Established) -> Result<(), PeerConnectionError> {
    if state.pending_cell_requests.is_empty() {
        return Ok(());
    }
    let pending = std::mem::take(&mut state.pending_cell_requests);
    // GetCells (0x14) only exists in eth/72; on an older negotiated version that
    // message id lands outside the eth range and the peer drops the connection.
    if !supports_eth72(state) {
        return Ok(());
    }

    // Merge all pending requests into a flat list of hashes with their masks,
    // dropping the columns already held: requests are queued from several
    // triggers (announce retrigger, tx-body response, custody sweep) and by
    // every connection, so without this the same cells are re-requested after
    // another peer already delivered them.
    let mut all_hashes: Vec<H256> = Vec::new();
    let mut all_masks: Vec<u128> = Vec::new();
    for (hashes, mask) in &pending {
        for &h in hashes {
            let missing = mask & !state.blockchain.mempool.available_cell_mask(h);
            if missing != 0 {
                all_hashes.push(h);
                all_masks.push(missing);
            }
        }
    }
    if all_hashes.is_empty() {
        return Ok(());
    }

    // Stay under the devp2p `caps/eth.md` soft limit of 64 hashes per GetCells, and
    // tighten it further so a peer applying the `MAX_CELLS_SERVED` budget can answer
    // in full: it omits trailing transactions, and an omitted tail is not re-requested
    // until the next announcement or custody change. The widest mask in the batch
    // bounds every chunk's mask, so this is conservative.
    let widest_mask = all_masks.iter().fold(0u128, |acc, &m| acc | m);
    let hashes_per_request =
        (MAX_CELLS_SERVED / cells_per_hash(widest_mask)).clamp(1, GET_CELLS_SOFT_LIMIT_HASHES);
    for (i, chunk) in all_hashes.chunks(hashes_per_request).enumerate() {
        let offset = i * hashes_per_request;
        let chunk_len = chunk.len();
        let chunk_masks = &all_masks[offset..offset + chunk_len];
        // Merge masks for this chunk.
        let merged_mask = chunk_masks.iter().fold(0u128, |acc, &m| acc | m);
        let id = random();
        let request = GetCells::new(id, chunk.to_vec(), merged_mask);
        send(state, Message::GetCells(request)).await?;
        // Register only after a successful send, so a failed write doesn't leave a
        // phantom in-flight entry (mirrors `flush_pending_tx_requests`).
        state
            .requested_cells
            .insert(id, (chunk.to_vec(), merged_mask, Instant::now()));
    }
    Ok(())
}

/// EIP-8070 messages (`GetCells`, `Cells`) exist only in eth/72, so they may only
/// be sent once eth/72 is the negotiated version for this connection.
fn supports_eth72(state: &Established) -> bool {
    state
        .negotiated_eth_capability
        .as_ref()
        .is_some_and(|cap| cap.version >= 72)
}

/// For each hash that has a remaining alternate announcer, look up that
/// peer's connection and enqueue the request there. Each alternate carries
/// the (type, size) metadata it originally announced, so the retry request
/// is built from the alternate's own announcement rather than the failing
/// peer's; otherwise validation against the failing peer's sizes would
/// reject the alternate's response when the two announcements differ (e.g.
/// bare blob tx vs full sidecar).
///
/// If a popped alternate is no longer reachable, keep popping until a live
/// peer is found or alternates for that hash are exhausted, so a disconnected
/// alternate doesn't burn the only fallback slot.
async fn retry_on_alternates(
    blockchain: &Arc<Blockchain>,
    peer_table: &PeerTable,
    hashes: &[H256],
) {
    if hashes.is_empty() {
        return;
    }
    // Group hashes by chosen live alternate, carrying their own type/size.
    // We walk per-hash so a dead alternate for hash X doesn't consume the
    // slot that hash Y could use. The `PeerConnection` handle from the
    // liveness probe is stashed in `by_peer` and reused at enqueue time,
    // so there's no second lookup (and no race where the connection drops
    // between probe and use).
    type AltGroup = (PeerConnection, Vec<(H256, u8, usize)>);
    let mut by_peer: FxHashMap<H256, AltGroup> = FxHashMap::default();
    for hash in hashes {
        loop {
            let alt = match blockchain.mempool.pop_alternate(*hash) {
                Ok(Some(a)) => a,
                Ok(None) => break,
                Err(e) => {
                    warn!(error = %e, "pop_alternate failed");
                    break;
                }
            };
            // Reuse the connection we already grabbed for this peer.
            if let Some((_, list)) = by_peer.get_mut(&alt.peer_id) {
                list.push((*hash, alt.tx_type, alt.tx_size));
                break;
            }
            match peer_table.get_peer_connection(alt.peer_id).await {
                Ok(Some(conn)) => {
                    by_peer.insert(alt.peer_id, (conn, vec![(*hash, alt.tx_type, alt.tx_size)]));
                    break;
                }
                Ok(None) => continue, // dead peer, try next alternate
                Err(e) => {
                    warn!(error = %e, "get_peer_connection failed");
                    break;
                }
            }
        }
    }

    for (_, (conn, entries)) in by_peer {
        let mut types = Vec::with_capacity(entries.len());
        let mut sizes = Vec::with_capacity(entries.len());
        let mut hash_list = Vec::with_capacity(entries.len());
        for (h, t, s) in &entries {
            hash_list.push(*h);
            types.push(*t);
            sizes.push(*s);
        }
        let announcement =
            NewPooledTransactionHashes::from_raw(types.into(), sizes, hash_list.clone());
        if let Err(e) = conn.enqueue_tx_requests(announcement, hash_list) {
            debug!(error = %e, "Failed to enqueue tx requests on alternate peer");
        }
    }
}

#[cfg(test)]
mod tx_announcement_tests {
    use super::*;
    use crate::{peer_table::PeerTableServer, rlpx::eth::transactions::PooledTransactions};
    use bytes::Bytes;
    use ethrex_blockchain::{
        BlockchainOptions,
        mempool::{MAX_ANNOUNCED_HASHES_PER_PEER, MIN_PROVIDERS_BEFORE_SAMPLING},
    };
    use ethrex_common::types::{
        BlobsBundle, EIP1559Transaction, EIP4844Transaction, WrappedEIP4844Transaction,
    };
    use ethrex_storage::EngineType;
    use std::net::{IpAddr, Ipv4Addr};
    use tokio::sync::mpsc;

    /// An established, synced session with a peer speaking `eth_version`, and the receiving end
    /// of its outbound queue, to see what the handler sends back.
    fn session(eth_version: u8) -> (Established, mpsc::Receiver<Message>) {
        let store = Store::new("memory", EngineType::InMemory).expect("in-memory store");
        let blockchain = Arc::new(Blockchain::default_with_store(store.clone()));
        blockchain.set_synced();
        let peer_table = PeerTableServer::spawn(H256::zero(), 10, store.clone());
        let tx_broadcaster =
            TxBroadcaster::spawn(peer_table.clone(), blockchain.clone(), 3_600_000)
                .expect("tx broadcaster");
        let signer = SecretKey::from_slice(&[7; 32]).expect("signing key");
        let peer_key = SecretKey::from_slice(&[9; 32]).expect("peer key");
        let node = Node::new(
            IpAddr::V4(Ipv4Addr::LOCALHOST),
            30303,
            30303,
            public_key_from_signing_key(&peer_key),
        );
        let (outbound_tx, outbound_rx) = mpsc::channel(OUTBOUND_QUEUE_CAP);
        let state = Established {
            signer,
            outbound_tx,
            outbound_writer_timed_out: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            node,
            is_inbound: true,
            storage: store,
            blockchain,
            capabilities: vec![],
            negotiated_eth_capability: Some(Capability::eth(eth_version)),
            negotiated_snap_capability: None,
            last_block_range_update_block: 0,
            requested_pooled_txs: HashMap::new(),
            requested_pooled_txs_72: HashMap::new(),
            pending_tx_requests: Vec::new(),
            pending_tx_requests_72: Vec::new(),
            pending_cell_requests: Vec::new(),
            requested_cells: HashMap::new(),
            last_custody_generation: 0,
            client_version: String::new(),
            connection_broadcast_send: broadcast::channel(16).0,
            peer_table,
            #[cfg(feature = "l2")]
            l2_state: L2ConnState::Unsupported,
            tx_broadcaster,
            current_requests: HashMap::new(),
            disconnect_reason: None,
            is_validated: true,
            serve_requests: WindowCounter::new(),
            announced_hashes: WindowCounter::new(),
            short_tx_answer_strikes: 0,
            connection_id: ConnectionId::next(),
            registered_in_peer_table: false,
            txs_sent_to_peer: 0,
            received_txs_from_peer: false,
            missed_pongs: 0,
        };
        (state, outbound_rx)
    }

    /// Distinct well-formed transactions. They are never admitted (unsigned), which is fine:
    /// what is checked is how they are requested and delivered.
    fn txs(from_nonce: u64, count: usize) -> Vec<P2PTransaction> {
        (from_nonce..from_nonce + count as u64)
            .map(|nonce| {
                P2PTransaction::EIP1559Transaction(EIP1559Transaction {
                    nonce,
                    ..Default::default()
                })
            })
            .collect()
    }

    fn announcement(eth_version: u8, txs: &[P2PTransaction]) -> Message {
        let types: Bytes = txs
            .iter()
            .map(|tx| tx.tx_type() as u8)
            .collect::<Vec<_>>()
            .into();
        let sizes = txs.iter().map(|tx| tx.encode_canonical_len()).collect();
        let hashes = txs.iter().map(|tx| tx.compute_hash()).collect();
        if eth_version >= 72 {
            Message::NewPooledTransactionHashes72(NewPooledTransactionHashes72::from_raw(
                types, sizes, hashes, None,
            ))
        } else {
            Message::NewPooledTransactionHashes(NewPooledTransactionHashes::from_raw(
                types, sizes, hashes,
            ))
        }
    }

    fn answer(eth_version: u8, id: u64, txs: Vec<P2PTransaction>) -> Message {
        if eth_version >= 72 {
            Message::PooledTransactions72(PooledTransactions72::new(id, txs))
        } else {
            Message::PooledTransactions(PooledTransactions::new(id, txs))
        }
    }

    /// Flushes the queued requests and returns what was sent, as (request id, hashes).
    async fn flush_requests(
        state: &mut Established,
        outbound: &mut mpsc::Receiver<Message>,
    ) -> Vec<(u64, Vec<H256>)> {
        flush_pending_tx_requests(state).await.expect("flush");
        flush_pending_tx_requests_72(state).await.expect("flush");
        let mut requests = Vec::new();
        while let Ok(message) = outbound.try_recv() {
            if let Message::GetPooledTransactions(request) = message {
                requests.push((request.id, request.transaction_hashes));
            }
        }
        requests
    }

    fn queued_hashes(state: &Established) -> usize {
        state
            .pending_tx_requests
            .iter()
            .map(|(_, h)| h.len())
            .sum::<usize>()
            + state
                .pending_tx_requests_72
                .iter()
                .map(|(_, h, _)| h.len())
                .sum::<usize>()
    }

    fn tracked(state: &Established) -> usize {
        state
            .blockchain
            .mempool
            .announced_hashes_tracked_for(state.node.node_id())
            .unwrap()
    }

    #[tokio::test]
    async fn announcements_past_the_budget_are_trimmed_and_released() {
        for eth_version in [68, 72] {
            let (mut state, mut outbound) = session(eth_version);
            let announced = txs(0, MAX_ANNOUNCED_HASHES_PER_PEER + 100);

            handle_incoming_message(&mut state, announcement(eth_version, &announced))
                .await
                .expect("an over-budget announcement is trimmed, not refused");
            assert_eq!(queued_hashes(&state), MAX_ANNOUNCED_HASHES_PER_PEER);
            assert_eq!(tracked(&state), MAX_ANNOUNCED_HASHES_PER_PEER);
            let queued_entries: usize = state
                .pending_tx_requests
                .iter()
                .map(|(a, _)| a.transaction_types.len())
                .chain(
                    state
                        .pending_tx_requests_72
                        .iter()
                        .map(|(a, _, _)| a.transaction_types.len()),
                )
                .sum();
            assert_eq!(
                queued_entries, MAX_ANNOUNCED_HASHES_PER_PEER,
                "queued announcements hold only the reserved entries"
            );

            // Nothing more is taken from this peer while its budget is full.
            handle_incoming_message(&mut state, announcement(eth_version, &txs(1_000_000, 10)))
                .await
                .unwrap();
            assert_eq!(queued_hashes(&state), MAX_ANNOUNCED_HASHES_PER_PEER);

            // An answer releases what it answered, and the rest goes with the session.
            let requests = flush_requests(&mut state, &mut outbound).await;
            let (id, first) = &requests[0];
            let delivered: Vec<P2PTransaction> = announced
                .iter()
                .filter(|tx| first.contains(&tx.compute_hash()))
                .cloned()
                .collect();
            handle_incoming_message(&mut state, answer(eth_version, *id, delivered))
                .await
                .unwrap();
            assert_eq!(
                tracked(&state),
                MAX_ANNOUNCED_HASHES_PER_PEER - first.len(),
                "eth/{eth_version}"
            );
            state.teardown().await;
            assert_eq!(tracked(&state), 0, "eth/{eth_version}");
        }
    }

    #[tokio::test]
    async fn a_session_ending_releases_all_the_peer_holds() {
        let (mut state, _outbound) = session(68);
        let peer = state.node.node_id();
        let mempool = &state.blockchain.mempool;
        let hash = H256::from_low_u64_be(0xabc);
        mempool
            .reserve_unknown_hashes(&[hash], &[2], &[100], H256::from_low_u64_be(1))
            .unwrap();
        mempool
            .reserve_unknown_hashes(&[hash], &[2], &[100], peer)
            .unwrap();
        mempool
            .record_provider_announcement(H256::from_low_u64_be(0xb10b), peer)
            .unwrap();
        assert_eq!(tracked(&state), 2);

        state.teardown().await;
        assert_eq!(tracked(&state), 0);
    }

    #[tokio::test]
    async fn a_session_that_never_started_leaves_its_node_announcements_alone() {
        let (mut state, _outbound) = session(68);
        // A second dial of a connected node that failed before the session started (too many
        // peers, a failed hello): it announced nothing, so what the node holds is the other
        // session's.
        state.is_validated = false;
        let mempool = &state.blockchain.mempool;
        let hash = H256::from_low_u64_be(0xabc);
        mempool
            .reserve_unknown_hashes(&[hash], &[2], &[100], H256::from_low_u64_be(1))
            .unwrap();
        mempool
            .reserve_unknown_hashes(&[hash], &[2], &[100], state.node.node_id())
            .unwrap();

        state.teardown().await;
        assert_eq!(tracked(&state), 1);
    }

    #[tokio::test]
    async fn repeated_empty_answers_disconnect_the_peer() {
        for eth_version in [68, 72] {
            let (mut state, mut outbound) = session(eth_version);
            for round in 1..=MAX_SHORT_TX_ANSWER_STRIKES {
                let tx = txs(u64::from(round), 1);
                handle_incoming_message(&mut state, announcement(eth_version, &tx))
                    .await
                    .unwrap();
                let requests = flush_requests(&mut state, &mut outbound).await;
                let result =
                    handle_incoming_message(&mut state, answer(eth_version, requests[0].0, vec![]))
                        .await;
                if round < MAX_SHORT_TX_ANSWER_STRIKES {
                    result.expect("below the strike limit the peer is kept");
                } else {
                    assert!(
                        matches!(
                            result,
                            Err(PeerConnectionError::DisconnectSent(
                                DisconnectReason::UselessPeer
                            ))
                        ),
                        "eth/{eth_version}: got {result:?}"
                    );
                }
            }
            assert!(matches!(
                outbound.try_recv(),
                Ok(Message::Disconnect(DisconnectMessage {
                    reason: Some(DisconnectReason::UselessPeer)
                }))
            ));
        }
    }

    #[tokio::test]
    async fn what_a_short_answer_leaves_out_is_retried_on_other_announcers() {
        for eth_version in [68, 72] {
            let (mut state, mut outbound) = session(eth_version);
            let announced = txs(0, 8);
            handle_incoming_message(&mut state, announcement(eth_version, &announced))
                .await
                .unwrap();
            // Another peer announced the same transactions meanwhile and is their fallback.
            let other_peer = H256::from_low_u64_be(0xfa11);
            let hashes: Vec<H256> = announced.iter().map(|tx| tx.compute_hash()).collect();
            state
                .blockchain
                .mempool
                .reserve_unknown_hashes(&hashes, &[2; 8], &[100; 8], other_peer)
                .unwrap();
            let (id, _) = flush_requests(&mut state, &mut outbound).await[0];

            handle_incoming_message(&mut state, answer(eth_version, id, announced[..1].to_vec()))
                .await
                .unwrap();
            // The fallback was taken for every hash left out (it is not connected here, so
            // nothing is sent to it).
            for hash in &hashes[1..] {
                assert!(
                    state
                        .blockchain
                        .mempool
                        .pop_alternate(*hash)
                        .unwrap()
                        .is_none(),
                    "eth/{eth_version}"
                );
            }
        }
    }

    #[tokio::test]
    async fn answers_that_deliver_what_was_announced_clear_strikes() {
        for eth_version in [68, 72] {
            let (mut state, mut outbound) = session(eth_version);
            // Each empty answer is followed by a full one, many times over the strike limit.
            for round in 0..u64::from(MAX_SHORT_TX_ANSWER_STRIKES) * 3 {
                let tx = txs(round, 1);
                handle_incoming_message(&mut state, announcement(eth_version, &tx))
                    .await
                    .unwrap();
                let (id, _) = flush_requests(&mut state, &mut outbound).await[0];
                let delivered = if round % 2 == 0 { vec![] } else { tx };
                handle_incoming_message(&mut state, answer(eth_version, id, delivered))
                    .await
                    .expect("an occasional empty answer is not punished");
            }
            assert_eq!(state.short_tx_answer_strikes, 0, "eth/{eth_version}");

            // An answer cut short of a large request still counts if it carries most of it.
            let announced = txs(1_000, 8);
            handle_incoming_message(&mut state, announcement(eth_version, &announced))
                .await
                .unwrap();
            let (id, _) = flush_requests(&mut state, &mut outbound).await[0];
            handle_incoming_message(&mut state, answer(eth_version, id, announced[..2].to_vec()))
                .await
                .unwrap();
            assert_eq!(state.short_tx_answer_strikes, 0, "eth/{eth_version}");
        }
    }

    /// `session(72)` with blob sampling on, as on a node that offers eth/72.
    fn sampling_session() -> (Established, mpsc::Receiver<Message>) {
        let (mut state, outbound) = session(72);
        let blockchain = Blockchain::new(
            state.storage.clone(),
            BlockchainOptions {
                blob_sampling_enabled: true,
                mempool_prewarm_enabled: false,
                ..BlockchainOptions::default()
            },
        );
        blockchain.set_synced();
        state.blockchain = Arc::new(blockchain);
        (state, outbound)
    }

    #[tokio::test]
    async fn every_peer_claiming_all_columns_counts_as_a_provider() {
        let (mut state, _outbound) = sampling_session();
        let roles = BlobRoles::new(&state);
        // A blob transaction this node samples rather than fetches whole.
        let blob_tx = (0..)
            .map(H256::from_low_u64_be)
            .find(|hash| roles.role(*hash, 3, Some(u128::MAX)) == BlobFetchRole::Sampler)
            .expect("most blob transactions are sampled");
        // Another peer claiming every column announced it first: it is requested from that
        // peer, which is recorded as its first provider.
        let first = H256::from_low_u64_be(0xf1);
        let mempool = &state.blockchain.mempool;
        mempool
            .reserve_unknown_hashes(&[blob_tx], &[3], &[131_072], first)
            .unwrap();
        mempool
            .record_provider_announcement(blob_tx, first)
            .unwrap();

        let announcement =
            Message::NewPooledTransactionHashes72(NewPooledTransactionHashes72::from_raw(
                vec![3u8].into(),
                vec![131_072],
                vec![blob_tx],
                Some(u128::MAX),
            ));
        handle_incoming_message(&mut state, announcement)
            .await
            .unwrap();
        assert_eq!(
            state
                .blockchain
                .mempool
                .provider_announcer_count(blob_tx)
                .unwrap(),
            MIN_PROVIDERS_BEFORE_SAMPLING
        );
    }

    #[tokio::test]
    async fn a_retry_is_matched_with_its_answer() {
        for eth_version in [68, 72] {
            let (mut state, mut outbound) = session(eth_version);
            let retried = txs(0, 2);
            // As `retry_on_alternates` builds it, whatever the peer's eth version.
            let Message::NewPooledTransactionHashes(retry) = announcement(68, &retried) else {
                unreachable!()
            };
            queue_retried_request(&mut state, &retry);
            let (id, _) = flush_requests(&mut state, &mut outbound).await[0];

            handle_incoming_message(&mut state, answer(eth_version, id, retried))
                .await
                .unwrap();
            assert!(state.requested_pooled_txs.is_empty(), "eth/{eth_version}");
            assert!(
                state.requested_pooled_txs_72.is_empty(),
                "eth/{eth_version}"
            );
            assert_eq!(tracked(&state), 0, "eth/{eth_version}");
        }
    }

    #[tokio::test]
    async fn short_answers_leave_the_peer_score_alone() {
        for eth_version in [68, 72] {
            let (mut state, mut outbound) = session(eth_version);
            let peer = state.node.node_id();
            let registered = state
                .peer_table
                .new_connected_peer(
                    state.node.clone(),
                    PeerConnection::unestablished(),
                    state.connection_id,
                    vec![],
                    vec![],
                    None,
                    true,
                )
                .await
                .expect("register peer");
            assert!(registered, "each session starts with an empty table");
            handle_incoming_message(&mut state, announcement(eth_version, &txs(0, 1)))
                .await
                .unwrap();
            let (id, _) = flush_requests(&mut state, &mut outbound).await[0];
            handle_incoming_message(&mut state, answer(eth_version, id, vec![]))
                .await
                .unwrap();

            // The strike is what judges the peer; the score, which picks peers for sync, is
            // not lowered by transaction gossip.
            assert_eq!(state.short_tx_answer_strikes, 1, "eth/{eth_version}");
            assert_eq!(
                state.peer_table.get_score(peer).await.unwrap(),
                0,
                "eth/{eth_version}"
            );
        }
    }

    #[tokio::test]
    async fn transactions_included_meanwhile_are_not_held_against_the_peer() {
        for eth_version in [68, 72] {
            let (mut state, mut outbound) = session(eth_version);
            let announced = txs(0, 4);
            let hashes: Vec<H256> = announced.iter().map(|tx| tx.compute_hash()).collect();
            handle_incoming_message(&mut state, announcement(eth_version, &announced))
                .await
                .unwrap();
            let fallback = H256::from_low_u64_be(0xfa11);
            state
                .blockchain
                .mempool
                .reserve_unknown_hashes(&hashes, &[2; 4], &[100; 4], fallback)
                .unwrap();
            let (id, _) = flush_requests(&mut state, &mut outbound).await[0];

            // A block included them before the peer answered, so it no longer holds them.
            state
                .blockchain
                .mempool
                .note_included_txs(hashes.clone())
                .unwrap();
            handle_incoming_message(&mut state, answer(eth_version, id, vec![]))
                .await
                .unwrap();
            assert_eq!(state.short_tx_answer_strikes, 0, "eth/{eth_version}");
            // Nobody is asked for them again, and their fallbacks hold no budget.
            assert_eq!(
                state
                    .blockchain
                    .mempool
                    .announced_hashes_tracked_for(fallback)
                    .unwrap(),
                0,
                "eth/{eth_version}"
            );
            for hash in &hashes {
                assert!(
                    state
                        .blockchain
                        .mempool
                        .pop_alternate(*hash)
                        .unwrap()
                        .is_none()
                );
            }
        }
    }

    #[test]
    fn short_answers_are_judged_by_count_and_delivered_size() {
        let hashes: Vec<H256> = (0..8).map(H256::from_low_u64_be).collect();
        let delivered = |n: usize, size: usize| {
            hashes[..n]
                .iter()
                .map(|hash| (*hash, size))
                .collect::<HashMap<H256, usize>>()
        };
        assert!(is_short_tx_answer(&hashes, &delivered(0, 200)));
        assert!(is_short_tx_answer(&hashes, &delivered(1, 200)));
        assert!(!is_short_tx_answer(&hashes, &delivered(2, 200)));
        // One blob-sized tx out of eight is what a responder's size limit allows.
        assert!(!is_short_tx_answer(
            &hashes,
            &delivered(1, MAX_POOLED_RESPONSE_BYTES / 4)
        ));
        assert!(!is_short_tx_answer(&[], &delivered(0, 200)));
    }

    #[tokio::test]
    async fn an_elided_blob_tx_is_credited_by_what_it_delivers() {
        let (mut state, mut outbound) = session(72);
        // Seven filler hashes and one blob tx announced at a full-blob size; nothing checks
        // that size against an elided delivery, so it must not count towards the answer.
        let blob_tx = P2PTransaction::EIP4844TransactionWithBlobs(WrappedEIP4844Transaction {
            tx: EIP4844Transaction::default(),
            wrapper_version: None,
            blobs_bundle: BlobsBundle::default(),
        });
        let fillers = txs(0, 7);
        let mut types: Vec<u8> = fillers.iter().map(|tx| tx.tx_type() as u8).collect();
        let mut sizes: Vec<usize> = fillers.iter().map(|tx| tx.encode_canonical_len()).collect();
        let mut hashes: Vec<H256> = fillers.iter().map(|tx| tx.compute_hash()).collect();
        types.push(3);
        sizes.push(MAX_POOLED_RESPONSE_BYTES);
        hashes.push(blob_tx.compute_hash());
        let announcement = Message::NewPooledTransactionHashes72(
            NewPooledTransactionHashes72::from_raw(types.into(), sizes, hashes, None),
        );
        handle_incoming_message(&mut state, announcement)
            .await
            .unwrap();
        let (id, _) = flush_requests(&mut state, &mut outbound).await[0];

        let _ = handle_incoming_message(&mut state, answer(72, id, vec![blob_tx])).await;
        assert_eq!(state.short_tx_answer_strikes, 1);
    }

    #[test]
    fn a_window_counter_admits_hits_up_to_its_limit() {
        let mut counter = WindowCounter::new();
        assert!(counter.hit(3, 4));
        assert!(counter.hit(1, 4));
        assert!(!counter.hit(1, 4));
        assert_eq!(counter.count(), 5);
        // Within one window, every later hit is past the limit too.
        assert!(!counter.hit(0, 4));
    }

    #[tokio::test]
    async fn announcing_past_the_rate_limit_disconnects_the_peer() {
        for eth_version in [68, 72] {
            let (mut state, _outbound) = session(eth_version);
            let full = announcement(eth_version, &txs(0, 4096));
            let allowed = MAX_ANNOUNCED_HASHES_PER_WINDOW / 4096;
            for _ in 0..allowed {
                handle_incoming_message(&mut state, full.clone())
                    .await
                    .expect("within the announcement rate limit");
            }
            let result = handle_incoming_message(&mut state, announcement(eth_version, &[])).await;
            assert!(
                matches!(
                    result,
                    Err(PeerConnectionError::DisconnectSent(
                        DisconnectReason::UselessPeer
                    ))
                ),
                "eth/{eth_version}: got {result:?}"
            );
        }
    }
}

#[cfg(test)]
mod bal_serving_tests {
    use super::*;
    use crate::rlpx::eth::block_access_lists::BLOCK_ACCESS_LIST_LIMIT;
    use crate::snap::constants::{BAL_MAX_REQUEST_HASHES, BAL_RESPONSE_SOFT_CAP_BYTES};
    use ethrex_common::{
        Address,
        types::{
            BlockHeader,
            block_access_list::{AccountChanges, BlockAccessList},
        },
    };
    use ethrex_storage::{EngineType, Store, api::tables::HEADERS, rlp::BlockHeaderRLP};

    fn store() -> Store {
        Store::new("memory", EngineType::InMemory).expect("in-memory store")
    }

    fn bal_with_accounts(n: u64) -> BlockAccessList {
        BlockAccessList::from_accounts(
            (0..n)
                .map(|i| AccountChanges::new(Address::from_low_u64_be(i + 1)))
                .collect(),
        )
    }

    /// Stores `bal` under `hash` together with a post-Amsterdam header committing to it,
    /// or to `commitment` when given (to simulate a stored list that does not match).
    fn store_committed_bal(
        store: &Store,
        hash: H256,
        bal: &BlockAccessList,
        commitment: Option<H256>,
    ) {
        let header = BlockHeader {
            base_fee_per_gas: Some(0),
            withdrawals_root: Some(H256::zero()),
            blob_gas_used: Some(0),
            excess_blob_gas: Some(0),
            parent_beacon_block_root: Some(H256::zero()),
            requests_hash: Some(H256::zero()),
            block_access_list_hash: Some(
                commitment.unwrap_or_else(|| bal.compute_hash(&NativeCrypto)),
            ),
            ..Default::default()
        };
        store
            .write(
                HEADERS,
                hash.encode_to_vec(),
                BlockHeaderRLP::from(header).into_vec(),
            )
            .expect("store header");
        store.store_block_access_list(hash, bal).expect("store BAL");
    }

    fn encoded_len(bals: &[Option<BlockAccessList>]) -> u64 {
        bals.iter()
            .map(|b| b.as_ref().map_or(1, |b| b.length() as u64))
            .sum()
    }

    /// A request may repeat one hash up to the entry limit. The response SHALL stop at
    /// the byte budget instead of carrying one copy of the list per repetition.
    #[test]
    fn repeated_hash_of_a_large_list_stays_within_the_byte_budget() {
        let store = store();
        let hash = H256::from_low_u64_be(7);
        let bal = bal_with_accounts(40_000);
        let one = bal.length() as u64;
        assert!(
            one > BAL_RESPONSE_SOFT_CAP_BYTES / 4,
            "fixture must be large ({one} bytes)"
        );
        store_committed_bal(&store, hash, &bal, None);

        let resp =
            build_block_access_lists_response(1, &vec![hash; BLOCK_ACCESS_LIST_LIMIT], &store);

        assert!(resp.block_access_lists.len() < BLOCK_ACCESS_LIST_LIMIT);
        assert!(
            resp.block_access_lists[0].is_some(),
            "the first entry is always served"
        );
        assert!(
            encoded_len(&resp.block_access_lists) < BAL_RESPONSE_SOFT_CAP_BYTES + one,
            "response carries at most one list beyond the budget"
        );
    }

    /// Entries keep the request order; unknown hashes and lists that do not match their
    /// header's commitment are reported unavailable.
    #[test]
    fn entries_follow_request_order_and_unverified_lists_are_unavailable() {
        let store = store();
        let good = H256::from_low_u64_be(1);
        let mismatched = H256::from_low_u64_be(2);
        let unknown = H256::from_low_u64_be(3);
        store_committed_bal(&store, good, &bal_with_accounts(3), None);
        store_committed_bal(
            &store,
            mismatched,
            &bal_with_accounts(4),
            Some(H256::repeat_byte(0xAB)),
        );

        let resp = build_block_access_lists_response(9, &[good, mismatched, unknown, good], &store);

        assert_eq!(resp.id, 9);
        let served: Vec<bool> = resp
            .block_access_lists
            .iter()
            .map(Option::is_some)
            .collect();
        assert_eq!(served, vec![true, false, false, true]);
    }

    /// Small lists stay far below the budget, so every requested entry is served.
    #[test]
    fn repeated_small_lists_are_all_served() {
        let store = store();
        let hash = H256::from_low_u64_be(5);
        store_committed_bal(&store, hash, &bal_with_accounts(1), None);

        let resp = build_block_access_lists_response(2, &vec![hash; 64], &store);

        assert_eq!(resp.block_access_lists.len(), 64);
        assert!(resp.block_access_lists.iter().all(Option::is_some));
    }

    /// More hashes than the protocol allows are cut to the entry limit.
    #[test]
    fn requests_are_cut_to_the_entry_limit() {
        let store = store();
        let hashes: Vec<H256> = (0..(BLOCK_ACCESS_LIST_LIMIT as u64 + 10))
            .map(H256::from_low_u64_be)
            .collect();

        let resp = build_block_access_lists_response(3, &hashes, &store);

        assert_eq!(resp.block_access_lists.len(), BLOCK_ACCESS_LIST_LIMIT);
    }

    /// The snap/2 variant SHALL read lists only until its budget is spent, so repeating
    /// one large list's hash does not decode it once per repetition first.
    #[test]
    fn snap2_repeated_hash_of_a_large_list_stays_within_the_byte_budget() {
        let store = store();
        let hash = H256::from_low_u64_be(11);
        let bal = bal_with_accounts(40_000);
        let one = bal.length() as u64;
        store_committed_bal(&store, hash, &bal, None);

        let req = Snap2GetBlockAccessLists {
            id: 4,
            block_hashes: vec![hash; BAL_MAX_REQUEST_HASHES],
            response_bytes: 0,
        };
        let resp = build_snap2_bal_response(req, &store).expect("response");

        assert!(resp.bals.len() < BAL_MAX_REQUEST_HASHES);
        assert!(encoded_len(&resp.bals) < BAL_RESPONSE_SOFT_CAP_BYTES + one);
    }
}

#[cfg(test)]
mod receipts_serving_tests {
    use super::*;
    use bytes::Bytes;
    use ethrex_common::{
        Address,
        types::{Log, TxType},
    };
    use ethrex_storage::EngineType;

    fn store() -> Store {
        Store::new("memory", EngineType::InMemory).expect("in-memory store")
    }

    /// `n` receipts, each with one log carrying `data_len` bytes of data.
    fn receipts(n: usize, data_len: usize) -> Vec<Receipt> {
        (0..n)
            .map(|i| {
                Receipt::new(
                    TxType::EIP1559,
                    true,
                    21_000 * (i as u64 + 1),
                    vec![Log {
                        address: Address::from_low_u64_be(1),
                        topics: vec![H256::from_low_u64_be(i as u64)],
                        data: Bytes::from(vec![0xAB; data_len]),
                    }],
                )
            })
            .collect()
    }

    fn encoded_len(lists: &[Vec<Receipt>], with_bloom: bool) -> usize {
        lists
            .iter()
            .flatten()
            .map(|receipt| receipt_wire_size(receipt, with_bloom))
            .sum()
    }

    /// One request may name a heavy block's hash once per allowed entry. The reply SHALL
    /// stop at the byte budget instead of carrying that block's receipts once per repetition.
    #[tokio::test]
    async fn repeated_hash_of_a_heavy_block_stays_within_the_byte_budget() {
        let store = store();
        let hash = H256::from_low_u64_be(7);
        let heavy = receipts(2_560, 1024);
        let one = encoded_len(std::slice::from_ref(&heavy), true);
        assert!(
            one > SOFT_RESPONSE_LIMIT / 8,
            "fixture must be heavy ({one} bytes)"
        );
        store
            .add_receipts(hash, heavy)
            .await
            .expect("store receipts");

        for with_bloom in [true, false] {
            let reply = build_receipts_response(
                &vec![hash; MAX_RECEIPTS_REQUEST_HASHES],
                with_bloom,
                &store,
            )
            .await
            .expect("reply");

            assert!(reply.len() < MAX_RECEIPTS_REQUEST_HASHES);
            assert_eq!(
                reply[0].len(),
                2_560,
                "the first list is always served whole"
            );
            assert!(
                encoded_len(&reply, with_bloom) <= SOFT_RESPONSE_LIMIT + one,
                "reply carries at most one list beyond the budget"
            );
        }
    }

    /// Entries keep the request order; an unknown block is an empty list and a repeated
    /// hash gets the same list again.
    #[tokio::test]
    async fn entries_follow_request_order() {
        let store = store();
        let a = H256::from_low_u64_be(1);
        let b = H256::from_low_u64_be(2);
        let unknown = H256::from_low_u64_be(3);
        store
            .add_receipts(a, receipts(3, 8))
            .await
            .expect("store a");
        store
            .add_receipts(b, receipts(5, 8))
            .await
            .expect("store b");

        let reply = build_receipts_response(&[a, unknown, b, a], true, &store)
            .await
            .expect("reply");

        let lengths: Vec<usize> = reply.iter().map(Vec::len).collect();
        assert_eq!(lengths, vec![3, 0, 5, 3]);
        assert_eq!(reply[0], reply[3]);
    }

    /// More hashes than the protocol allows are cut to the entry limit.
    #[tokio::test]
    async fn requests_are_cut_to_the_hash_limit() {
        let store = store();
        let hashes: Vec<H256> = (0..(MAX_RECEIPTS_REQUEST_HASHES as u64 + 10))
            .map(H256::from_low_u64_be)
            .collect();
        for hash in &hashes {
            store
                .add_receipts(*hash, receipts(1, 8))
                .await
                .expect("store");
        }

        let reply = build_receipts_response(&hashes, false, &store)
            .await
            .expect("reply");

        assert_eq!(reply.len(), MAX_RECEIPTS_REQUEST_HASHES);
        assert!(reply.iter().all(|list| list.len() == 1));
    }

    /// The eth/68 budget counts the bloom each receipt goes out with, so a block of many
    /// log-less receipts is not under-counted by the ~260 bytes each bloom adds; the count
    /// is an upper bound within a few bytes of the item on the wire.
    #[test]
    fn eth68_wire_size_counts_the_bloom() {
        use crate::rlpx::eth::eth68::receipts::ReceiptItem68;
        for receipt in [
            Receipt::new(TxType::EIP1559, true, 21_000, vec![]),
            Receipt::new(
                TxType::Legacy,
                false,
                1 << 20,
                receipts(1, 100).remove(0).logs,
            ),
        ] {
            let on_the_wire = ReceiptItem68(receipt.clone()).length();
            let counted = receipt_wire_size(&receipt, true);
            assert!(
                (on_the_wire..on_the_wire + 8).contains(&counted),
                "counted {counted} bytes for a {on_the_wire}-byte eth/68 item"
            );
            assert_eq!(receipt_wire_size(&receipt, false), receipt.length());
        }
    }
}
