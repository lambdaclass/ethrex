use std::net::SocketAddr;
use tokio::sync::mpsc;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Failed to send message: {0:?}")]
    FailedToSend(Message, String),
}

#[derive(Debug, Clone)]
pub enum Message {
    SendViaUDP(SocketAddr, Vec<u8>),
    SendViaTCP(SocketAddr),
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

    pub async fn relay(&self, to: SocketAddr, content: Vec<u8>) -> Result<(), Error> {
        self.send(Message::SendViaUDP(to, content)).await
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
