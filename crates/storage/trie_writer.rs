use tokio::{
    sync::mpsc,
    task::JoinHandle,
};

use crate::{Store, TrieUpdates};

// TODO: Revisit this constant, since this can lead to bugs.
const WRITER_CHANNEL_SIZE: usize = 100;

#[derive(Debug)]
pub struct TrieWriter {
    sender: mpsc::Sender<TrieUpdates>,
    handle: JoinHandle<()>,
}

impl TrieWriter {
    pub fn new(store: Store) -> Self {
        let (sender, receiver) = mpsc::channel(WRITER_CHANNEL_SIZE);
        let handle = tokio::spawn(Self::writer_loop(store.clone(), receiver));
        Self { sender, handle }
    }

    pub async fn writer_loop(store: Store, mut receiver: mpsc::Receiver<TrieUpdates>) {
        while let Some(update) = receiver.recv().await {
            store.store_trie_updates(update).await.unwrap();
        }
    }

    /// Send a message to the TrieWriter task to persist the [`TrieUpdates`].
    pub async fn write(&self, update: TrieUpdates) {
        self.sender.send(update).await.unwrap();
    }

    pub fn task_handle(&self) -> &JoinHandle<()> {
        &self.handle
    }
}

