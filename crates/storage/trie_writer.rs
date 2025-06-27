use tokio::{
    sync::mpsc,
    task::JoinHandle,
};

use crate::{Store, TrieUpdates};

const WRITER_CHANNEL_SIZE: usize = 1000;

#[derive(Debug)]
pub struct TrieWriter {
    sender: mpsc::Sender<TrieUpdates>,
    handle: JoinHandle<()>,
    store: Store,
}

impl TrieWriter {
    pub fn new(store: Store) -> Self {
        let (sender, receiver) = mpsc::channel(WRITER_CHANNEL_SIZE);
        let handle = tokio::spawn(Self::writer_loop(store.clone(), receiver));
        Self { sender, handle, store }
    }

    pub async fn writer_loop(store: Store, mut receiver: mpsc::Receiver<TrieUpdates>) {
        while let Some(update) = receiver.recv().await {
            store.store_trie_updates(update).await.unwrap();
        }
    }

    /// Send a message to the TrieWriter task to persist the [`TrieUpdates`].
    pub async fn write(&self, update: TrieUpdates) {
        // before sending the updates to the task, we need to update the dirty nodes
        self.store.update_cache(&update);
        // then we can send the updates to the task
        self.sender.send(update).await.unwrap();
    }

    pub fn task_handle(&self) -> &JoinHandle<()> {
        &self.handle
    }
}

