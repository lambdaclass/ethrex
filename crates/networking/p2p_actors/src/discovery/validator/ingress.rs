use tokio::sync::mpsc;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Failed to send message {0:?}. Reason: {1}")]
    FailedToSend(Message, String),
}

#[derive(Debug, Clone)]
pub enum Message {
    Validate,
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

    pub async fn validate(&self) -> Result<(), Error> {
        self.send(Message::Validate).await
    }

    pub async fn terminate(&self) -> Result<(), Error> {
        self.send(Message::Terminate).await
    }

    pub async fn send(&self, message: Message) -> Result<(), Error> {
        self.sender
            .send(message.clone())
            .await
            .map_err(|err| Error::FailedToSend(message, err.to_string()))
    }
}
