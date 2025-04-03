use ethrex_storage::Store;
use tokio::sync::mpsc::Receiver;

use crate::peer_handler::PeerHandler;

use super::{BATCH_SIZE, MAX_CHANNEL_READS, MAX_PARALLEL_FETCHES};

/// Reads incoming requests from the receiver, adds them to the queue, and returns the requests' incoming status
/// Will only wait out for incoming requests if the queue is currenlty empty
pub(crate) async fn read_incoming_requests<T>(
    receiver: &mut Receiver<Vec<T>>,
    queue: &mut Vec<T>,
) -> bool {
    if !receiver.is_empty() || queue.is_empty() {
        let mut msg_buffer = vec![];
        receiver.recv_many(&mut msg_buffer, MAX_CHANNEL_READS).await;
        let incoming = msg_buffer.is_empty() || msg_buffer.iter().any(|reqs| reqs.is_empty());
        queue.extend(msg_buffer.into_iter().flatten());
        incoming
    } else {
        true
    }
}

pub(crate) async fn spawn_fetch_tasks<T, F, Fut>(
    queue: &mut Vec<T>,
    full_batches: bool,
    fetch_batch: &F,
    peers: PeerHandler,
    store: Store,
) -> bool
where
    T: Send + 'static,
    F: Fn(Vec<T>, PeerHandler, Store) -> Fut + Sync + Send,
    Fut: std::future::Future<Output = (Vec<T>, bool)> + Send + 'static,
{
    let mut stale = false;
    if queue.len() > BATCH_SIZE || (!full_batches && !queue.is_empty()) {
        // Spawn fetch tasks
        let mut tasks = tokio::task::JoinSet::new();
        for _ in 0..MAX_PARALLEL_FETCHES {
            let next_batch = queue
                .drain(..BATCH_SIZE.min(queue.len()))
                .collect::<Vec<_>>();
            tasks.spawn(fetch_batch(next_batch, peers.clone(), store.clone()));
            // End loop if we don't have enough elements to fill up a batch
            if queue.is_empty() || (full_batches && queue.len() < BATCH_SIZE) {
                break;
            }
        }
        // Collect Results
        for res in tasks.join_all().await {
            let (remaining, is_stale) = res;
            queue.extend(remaining);
            stale |= is_stale;
        }
    }
    stale
}
