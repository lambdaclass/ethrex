use ethrex_storage::Store;
use tokio::sync::mpsc::Receiver;

use crate::peer_handler::PeerHandler;

use super::{MAX_CHANNEL_READS, MAX_PARALLEL_FETCHES};

/// Runs the queue process by reading incoming messages from the receiver, adding the requests to the queue, and then spawning parallel fetch tasks for all queued items
/// This process will only end when an end signal in the form of an empty vector is read from the receiver
pub(crate) async fn run_queue<T, F, Fut>(receiver: &mut Receiver<Vec<T>>, queue: &mut Vec<T>, fetch_batch: &F,
    peers: PeerHandler,
    store: Store,
    batch_size: usize)
    where
    T: Send + 'static,
    F: Fn(Vec<T>, PeerHandler, Store) -> Fut + Sync + Send,
    Fut: std::future::Future<Output = (Vec<T>, bool)> + Send + 'static {
        // The pivot may become stale while the fetcher is active, we will still keep the process
        // alive until the end signal so we don't lose incoming messages
        let mut incoming = true;
        let mut stale = false;
        while incoming {
            // Read incoming messages and add them to the queue
            incoming = read_incoming_requests(receiver, queue).await;
            // If the pivot isn't stale, spawn fetch tasks for the queued elements
            if !stale {
                stale = spawn_fetch_tasks(queue, incoming, fetch_batch, peers.clone(), store.clone(), batch_size).await
            }
        }

    }

/// Reads incoming requests from the receiver, adds them to the queue, and returns the requests' incoming status
/// Will only wait out for incoming requests if the queue is currenlty empty
async fn read_incoming_requests<T>(
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

/// Spawns fetch tasks for the queued items, adds the remaining ones back to the queue and returns the pivot's stale status
/// Will only fetch full batches (according to `batch_size`) unless `full_batches` is set to false
async fn spawn_fetch_tasks<T, F, Fut>(
    queue: &mut Vec<T>,
    full_batches: bool,
    fetch_batch: &F,
    peers: PeerHandler,
    store: Store,
    batch_size: usize,
) -> bool
where
    T: Send + 'static,
    F: Fn(Vec<T>, PeerHandler, Store) -> Fut + Sync + Send,
    Fut: std::future::Future<Output = (Vec<T>, bool)> + Send + 'static,
{
    let mut stale = false;
    if queue.len() > batch_size || (!full_batches && !queue.is_empty()) {
        // Spawn fetch tasks
        let mut tasks = tokio::task::JoinSet::new();
        for _ in 0..MAX_PARALLEL_FETCHES {
            let next_batch = queue
                .drain(..batch_size.min(queue.len()))
                .collect::<Vec<_>>();
            tasks.spawn(fetch_batch(next_batch, peers.clone(), store.clone()));
            // End loop if we don't have enough elements to fill up a batch
            if queue.is_empty() || (full_batches && queue.len() < batch_size) {
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
