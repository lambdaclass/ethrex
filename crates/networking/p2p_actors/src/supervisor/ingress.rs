use crate::supervisor::types::ChildSpec;
use tokio::sync::mpsc;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Failed to send message {0:?}. Reason: {1}")]
    FailedToSend(Message, String),
}

#[derive(Debug, Clone)]
pub enum Message {
    Supervise,
    StartChild(ChildSpec),
    TerminateChild(&'static str),
    DeleteChild(&'static str),
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

    pub fn is_closed(&self) -> bool {
        self.sender.is_closed()
    }

    pub async fn supervise(&self) -> Result<(), Error> {
        self.send(Message::Supervise).await
    }

    pub async fn start_child(&self, spec: ChildSpec) -> Result<(), Error> {
        self.send(Message::StartChild(spec)).await
    }

    pub async fn terminate_child(&self, child_id: &'static str) -> Result<(), Error> {
        self.send(Message::TerminateChild(child_id)).await
    }

    pub async fn delete_child(&self, child_id: &'static str) -> Result<(), Error> {
        self.send(Message::DeleteChild(child_id)).await
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
