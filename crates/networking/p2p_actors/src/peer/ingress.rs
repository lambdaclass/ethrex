use ethrex_rlp::error::RLPDecodeError;
use tokio::sync::mpsc;

pub const MAX_UDP_PAYLOAD_SIZE: usize = 1280;
pub const DEFAULT_UDP_PAYLOAD_BUF: [u8; MAX_UDP_PAYLOAD_SIZE] = [0u8; MAX_UDP_PAYLOAD_SIZE];
pub const HASH_LENGTH_IN_BYTES: usize = 32;
pub const HEADER_LENGTH_IN_BYTES: usize = HASH_LENGTH_IN_BYTES + 65;
pub const PACKET_TYPE_LENGTH_IN_BYTES: usize = 1;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Failed to send message: {0:?}, reason: {1}")]
    FailedToSend(Message, String),
    #[error("RLP decode error: {0}")]
    FailedToRLPDecode(#[from] RLPDecodeError),
}

#[derive(Debug, Clone)]
pub enum Message {}

#[derive(Clone)]
pub struct Mailbox {
    sender: mpsc::Sender<Message>,
}

impl Mailbox {
    pub fn new(sender: mpsc::Sender<Message>) -> Self {
        Self { sender }
    }

    async fn send(&self, message: Message) -> Result<(), Error> {
        self.sender
            .send(message.clone())
            .await
            .map_err(|e| Error::FailedToSend(message, e.to_string()))
    }
}
