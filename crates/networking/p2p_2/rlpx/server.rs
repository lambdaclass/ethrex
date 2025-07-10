use std::{
    collections::{HashMap, HashSet},
    net::SocketAddr,
    sync::Arc,
    time::Duration,
};

use ethrex_blockchain::Blockchain;
use ethrex_common::{
    H256,
    types::{MempoolTransaction, Transaction},
};
use ethrex_storage::Store;
use futures::{SinkExt as _, Stream, stream::SplitSink};
use k256::{PublicKey, ecdsa::SigningKey};
use rand::random;
use spawned_concurrency::{
    messages::Unused,
    tasks::{CastResponse, GenServer, GenServerHandle, send_interval},
};
use tokio::{
    net::TcpStream,
    sync::{Mutex, broadcast, mpsc::Sender},
    task,
};
use tokio_util::codec::Framed;

const PING_INTERVAL: Duration = Duration::from_secs(10);
const TX_BROADCAST_INTERVAL: Duration = Duration::from_millis(500);
const BLOCK_RANGE_UPDATE_INTERVAL: Duration = Duration::from_secs(60);

pub(crate) type RLPxConnBroadcastSender = broadcast::Sender<(tokio::task::Id, Arc<Message>)>;

type MsgResult = Result<OutMessage, RLPxError>;
type RLPxConnectionHandle = GenServerHandle<RLPxConnection>;

#[derive(Clone)]
pub enum RLPxConnectionState {
    Initiator {
        context: P2PContext,
        node: Node,
    },
    Receiver {
        context: P2PContext,
        peer_addr: SocketAddr,
        stream: Arc<TcpStream>,
    },
    Established {
        signer: SigningKey,
        // Sending part of the TcpStream to connect with the remote peer
        // The receiving part is owned by the stream listen loop task
        sink: Arc<Mutex<SplitSink<Framed<TcpStream, RLPxCodec>, Message>>>,
        node: Node,
        storage: Store,
        blockchain: Arc<Blockchain>,
        capabilities: Vec<Capability>,
        negotiated_eth_capability: Option<Capability>,
        negotiated_snap_capability: Option<Capability>,
        last_block_range_update_block: u64,
        broadcasted_txs: HashSet<H256>,
        requested_pooled_txs: HashMap<u64, NewPooledTransactionHashes>,
        client_version: String,
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
        connection_broadcast_send: RLPxConnBroadcastSender,
        table: Arc<Mutex<KademliaTable>>,
        backend_channel: Option<Sender<Message>>,
        inbound: bool,
    },
}

impl RLPxConnectionState {
    pub fn new_as_receiver(context: P2PContext, peer_addr: SocketAddr, stream: TcpStream) -> Self {
        Self::Receiver(Receiver {
            context,
            peer_addr,
            stream: Arc::new(stream),
        })
    }

    pub fn new_as_initiator(context: P2PContext, node: &Node) -> Self {
        Self::Initiator(Initiator {
            context,
            node: node.clone(),
        })
    }
}

#[derive(Clone)]
#[allow(private_interfaces)]
pub enum CastMessage {
    PeerMessage(Message),
    BackendMessage(Message),
    SendPing,
    SendNewPooledTxHashes,
    BlockRangeUpdate,
    BroadcastMessage(task::Id, Arc<Message>),
}

#[derive(Clone)]
#[allow(private_interfaces)]
pub enum OutMessage {
    InitResponse {
        node: Node,
        framed: Arc<Mutex<Framed<TcpStream, RLPxCodec>>>,
    },
    Done,
    Error,
}

#[derive(Debug)]
pub struct RLPxConnection {}

impl RLPxConnection {
    pub async fn spawn_as_receiver(
        context: P2PContext,
        peer_addr: SocketAddr,
        stream: TcpStream,
    ) -> RLPxConnectionHandle {
        let state = RLPxConnectionState::new_as_receiver(context, peer_addr, stream);
        RLPxConnection::start(state)
    }

    pub async fn spawn_as_initiator(context: P2PContext, node: &Node) -> RLPxConnectionHandle {
        let state = RLPxConnectionState::new_as_initiator(context, node);
        RLPxConnection::start(state.clone())
    }
}

impl GenServer for RLPxConnection {
    type CallMsg = Unused;
    type CastMsg = CastMessage;
    type OutMsg = MsgResult;
    type State = RLPxConnectionState;
    type Error = RLPxError;

    fn new() -> Self {
        Self {}
    }

    async fn init(
        &mut self,
        handle: &GenServerHandle<Self>,
        mut state: Self::State,
    ) -> Result<Self::State, Self::Error> {
    }

    async fn handle_cast(
        &mut self,
        message: Self::CastMsg,
        _handle: &RLPxConnectionHandle,
        mut state: Self::State,
    ) -> CastResponse<Self> {
        CastResponse::Unused
    }
}
