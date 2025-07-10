use std::{collections::HashMap, sync::Arc};

use spawned_concurrency::{
    messages::Unused,
    tasks::{CastResponse, GenServer},
};
use tokio::sync::Mutex;

#[derive(Debug, thiserror::Error)]
pub enum DiscoveryServerError {}

#[derive(Debug, Clone)]
pub struct DiscoveryServerState {
    kademlia: Arc<Mutex<HashMap<String, String>>>,
}

#[derive(Debug, Clone)]
pub enum InMessage {
    Listen {
        listener: Arc<tokio::net::TcpListener>,
    },
}

#[derive(Debug, Clone)]
pub enum OutMessage {
    Done,
}

pub struct DiscoveryServer;

impl GenServer for DiscoveryServer {
    type CallMsg = Unused;
    type CastMsg = InMessage;
    type OutMsg = OutMessage;
    type State = DiscoveryServerState;
    type Error = DiscoveryServerError;

    fn new() -> Self {
        Self {}
    }

    async fn handle_cast(
        &mut self,
        message: Self::CastMsg,
        handle: &spawned_concurrency::tasks::GenServerHandle<Self>,
        state: Self::State,
    ) -> CastResponse<Self> {
        let revalidate_period = std::time::Duration::from_secs(60);
        let revalidate_period = std::time::Duration::from_secs(60);
        let lookup_period = std::time::Duration::from_secs(60);
        match message {
            Self::CastMsg::Listen { listener } => {
                handle_listens(&state, listener).await;
                CastResponse::Stop
            }
        }
    }
}

async fn handle_listens(state: &DiscoveryServerState, listener: Arc<tokio::net::TcpListener>) {
    loop {
        let res = listener.accept().await;
        match res {
            Ok((stream, addr)) => {
                // Cloning the ProofCoordinatorState structure to use the handle_connection() fn
                // in every spawned task.
                // The important fields are `Store` and `EthClient`
                // Both fields are wrapped with an Arc, making it possible to clone
                // the entire structure.
                let _ = ConnectionHandler::spawn(state.clone(), stream, addr).await;
            }
            Err(e) => {}
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ConnectionHandlerError {}

#[derive(Debug, Clone)]
pub enum ConnectionHandlerInMessage {
    Connection {
        stream: Arc<tokio::net::TcpStream>,
        addr: tokio::net::unix::SocketAddr,
    },
}

#[derive(Debug, Clone)]
pub enum ConnectionHandlerOutMessage {
    Done,
}

pub struct ConnectionHandler;

impl ConnectionHandler {
    pub async fn spawn(
        state: DiscoveryServerState,
        stream: tokio::net::TcpStream,
        addr: std::net::SocketAddr,
    ) -> Result<(), ConnectionHandlerError> {
        Ok(())
    }
}

impl GenServer for ConnectionHandler {
    type CallMsg = Unused;
    type CastMsg = ConnectionHandlerInMessage;
    type OutMsg = ConnectionHandlerOutMessage;
    type State = DiscoveryServerState;
    type Error = ConnectionHandlerError;

    fn new() -> Self {
        Self {}
    }

    async fn handle_cast(
        &mut self,
        message: Self::CastMsg,
        handle: &spawned_concurrency::tasks::GenServerHandle<Self>,
        state: Self::State,
    ) -> CastResponse<Self> {
        match message {
            Self::CastMsg::Connection { stream, addr } => {
                handle_connection(&state, stream, addr).await;
            }
        }
        CastResponse::NoReply(state)
    }
}

async fn handle_connection(
    state: &DiscoveryServerState,
    stream: Arc<tokio::net::TcpStream>,
    addr: tokio::net::unix::SocketAddr,
) {
    // Figure out if the node that is talking to add is suitable enough for
    // being added to the Kademlia routing table.
}
