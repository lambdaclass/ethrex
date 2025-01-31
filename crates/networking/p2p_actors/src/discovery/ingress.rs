use std::net::SocketAddr;

use tokio::sync::mpsc;

use crate::{discovery::packet::Packet, types::NodeId};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Failed to send message {0:?}. Reason: {1}")]
    FailedToSend(Message, String),
}

#[derive(Debug, Clone)]
pub enum Message {
    Serve(Packet, SocketAddr),
    Lookup(NodeId),
    Revalidate,
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
        self.send(Message::Serve(packet, from)).await
    }

    pub async fn lookup(&self, target: NodeId) -> Result<(), Error> {
        self.send(Message::Lookup(target)).await
    }

    pub async fn revalidate(&self) -> Result<(), Error> {
        self.send(Message::Revalidate).await
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
