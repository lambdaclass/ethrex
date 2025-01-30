use std::net::SocketAddr;

use tokio::sync::mpsc;

use crate::discovery::packet::{Packet, PacketData};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Failed to send message {0:?}. Reason: {1}")]
    FailedToSend(Message, String),
}

#[derive(Debug, Clone)]
pub enum Message {
    Ping(Packet),
    Pong(Packet),
    FindNode(Packet, SocketAddr),
    Neighbors(Packet),
    ENRRequest(Packet),
    ENRResponse(Packet),
    Terminate,
}

#[derive(Clone)]
pub struct Mailbox {
    sender: mpsc::Sender<Message>,
}

impl Mailbox {
    pub fn new(sender: mpsc::Sender<Message>) -> Self {
        Self { sender }
    }

    pub async fn serve(&self, packet: Packet, from: SocketAddr) -> Result<(), Error> {
        let message = match packet.data {
            PacketData::Ping { .. } => Message::Ping(packet),
            PacketData::Pong { .. } => Message::Pong(packet),
            PacketData::FindNode { .. } => Message::FindNode(packet, from),
            PacketData::Neighbors { .. } => Message::Neighbors(packet),
            PacketData::ENRRequest { .. } => Message::ENRRequest(packet),
            PacketData::ENRResponse { .. } => Message::ENRResponse(packet),
        };
        self.send(message).await
    }

    pub async fn terminate(&self) -> Result<(), Error> {
        self.send(Message::Terminate).await
    }

    async fn send(&self, message: Message) -> Result<(), Error> {
        self.sender
            .send(message.clone())
            .await
            .map_err(|err| Error::FailedToSend(message, err.to_string()))
    }
}
