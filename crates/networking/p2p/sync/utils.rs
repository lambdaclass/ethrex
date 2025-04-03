use tokio::sync::mpsc::Receiver;

use super::MAX_CHANNEL_READS;

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
