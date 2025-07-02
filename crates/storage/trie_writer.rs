use std::sync::{Arc, atomic::AtomicU64};

use tokio::{sync::mpsc, task::JoinHandle};

use crate::{Store, TrieUpdates};

const WRITER_CHANNEL_SIZE: usize = 40000;

#[derive(Debug)]
pub struct TrieWriter {
    sender: mpsc::Sender<TrieUpdates>,
    handle: JoinHandle<()>,
    store: Store,
    queued: Arc<AtomicU64>,
}

impl TrieWriter {
    pub fn new(store: Store) -> Self {
        let queued = Arc::new(AtomicU64::new(0));
        let (sender, receiver) = mpsc::channel(WRITER_CHANNEL_SIZE);
        let handle = tokio::spawn(Self::writer_loop(store.clone(), receiver, queued.clone()));
        Self {
            sender,
            handle,
            store,
            queued,
        }
    }

    pub async fn writer_loop(
        store: Store,
        mut receiver: mpsc::Receiver<TrieUpdates>,
        queued: Arc<AtomicU64>,
    ) {
        while let Some(update) = receiver.recv().await {
            let queued = queued.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
            tracing::info!("Trie updates in queue: {queued}");
            store.store_trie_updates(update).await.unwrap();
        }
    }

    /// Send a message to the TrieWriter task to persist the [`TrieUpdates`].
    pub async fn write(&self, update: TrieUpdates) {
        if update.account_updates.is_empty() && update.storage_updates.is_empty() {
            return;
        }
        // before sending the updates to the task, we need to update the dirty nodes
        self.store.update_cache(&update);
        self.queued
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        // then we can send the updates to the task
        self.sender.send(update).await.unwrap();
    }

    pub fn task_handle(&self) -> &JoinHandle<()> {
        &self.handle
    }
}
