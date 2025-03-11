use super::{
    messages::{BlockProducerToBlockProducer, CommitterToCommitter, L1WatcherToL1Watcher},
    traits::connections::{Connections, Receiver, Sender},
};

pub struct Spine {
    sender_committer_to_committer: Sender<CommitterToCommitter>,
    receiver_committer_to_committer: Receiver<CommitterToCommitter>,

    sender_l1_watcher_to_l1_watcher: Sender<L1WatcherToL1Watcher>,
    receiver_l1_watcher_to_l1_watcher: Receiver<L1WatcherToL1Watcher>,

    sender_block_producer_to_block_producer: Sender<BlockProducerToBlockProducer>,
    receiver_block_producer_to_block_producer: Receiver<BlockProducerToBlockProducer>,
}

impl Default for Spine {
    fn default() -> Self {
        let (sender_send_commit_after, receiver_send_commit_after) = crossbeam_channel::unbounded();
        let (sender_send_watch_l1_to_l2_tx_after, receiver_send_watch_l1_to_l2_tx_after) =
            crossbeam_channel::unbounded();
        let (sender_block_producer_to_block_producer, receiver_block_producer_to_block_producer) =
            crossbeam_channel::unbounded();

        Self {
            sender_committer_to_committer: sender_send_commit_after,
            receiver_committer_to_committer: receiver_send_commit_after,
            sender_l1_watcher_to_l1_watcher: sender_send_watch_l1_to_l2_tx_after,
            receiver_l1_watcher_to_l1_watcher: receiver_send_watch_l1_to_l2_tx_after,
            sender_block_producer_to_block_producer,
            receiver_block_producer_to_block_producer,
        }
    }
}

impl Spine {
    pub fn to_connections(&self) -> SpineConnections {
        SpineConnections::new(self.into(), self.into())
    }
}

pub struct SendersSpine {
    sender_send_commit_after: Sender<CommitterToCommitter>,
    sender_send_watch_l1_to_l2_tx_after: Sender<L1WatcherToL1Watcher>,
    sender_block_producer_to_block_producer: Sender<BlockProducerToBlockProducer>,
}

impl From<&Spine> for SendersSpine {
    fn from(spine: &Spine) -> Self {
        Self {
            sender_send_commit_after: spine.sender_committer_to_committer.clone(),
            sender_send_watch_l1_to_l2_tx_after: spine.sender_l1_watcher_to_l1_watcher.clone(),
            sender_block_producer_to_block_producer: spine
                .sender_block_producer_to_block_producer
                .clone(),
        }
    }
}

impl AsRef<Sender<CommitterToCommitter>> for SendersSpine {
    fn as_ref(&self) -> &Sender<CommitterToCommitter> {
        &self.sender_send_commit_after
    }
}

impl AsRef<Sender<L1WatcherToL1Watcher>> for SendersSpine {
    fn as_ref(&self) -> &Sender<L1WatcherToL1Watcher> {
        &self.sender_send_watch_l1_to_l2_tx_after
    }
}

impl AsRef<Sender<BlockProducerToBlockProducer>> for SendersSpine {
    fn as_ref(&self) -> &Sender<BlockProducerToBlockProducer> {
        &self.sender_block_producer_to_block_producer
    }
}

pub struct ReceiversSpine {
    receiver_send_commit_after: Receiver<CommitterToCommitter>,
    receiver_send_watch_l1_to_l2_tx_after: Receiver<L1WatcherToL1Watcher>,
    receiver_block_producer_to_block_producer: Receiver<BlockProducerToBlockProducer>,
}

impl From<&Spine> for ReceiversSpine {
    fn from(spine: &Spine) -> Self {
        Self {
            receiver_send_commit_after: spine.receiver_committer_to_committer.clone(),
            receiver_send_watch_l1_to_l2_tx_after: spine.receiver_l1_watcher_to_l1_watcher.clone(),
            receiver_block_producer_to_block_producer: spine
                .receiver_block_producer_to_block_producer
                .clone(),
        }
    }
}

impl AsMut<Receiver<CommitterToCommitter>> for ReceiversSpine {
    fn as_mut(&mut self) -> &mut Receiver<CommitterToCommitter> {
        &mut self.receiver_send_commit_after
    }
}

impl AsMut<Receiver<L1WatcherToL1Watcher>> for ReceiversSpine {
    fn as_mut(&mut self) -> &mut Receiver<L1WatcherToL1Watcher> {
        &mut self.receiver_send_watch_l1_to_l2_tx_after
    }
}

impl AsMut<Receiver<BlockProducerToBlockProducer>> for ReceiversSpine {
    fn as_mut(&mut self) -> &mut Receiver<BlockProducerToBlockProducer> {
        &mut self.receiver_block_producer_to_block_producer
    }
}

pub struct SpineConnections {
    senders: SendersSpine,
    receivers: ReceiversSpine,
}

impl SpineConnections {
    pub fn new(senders: SendersSpine, receivers: ReceiversSpine) -> Self {
        Self { senders, receivers }
    }
}

impl Connections for SpineConnections {
    type Senders = SendersSpine;

    type Receivers = ReceiversSpine;

    fn receiver<T>(&mut self) -> &mut Receiver<T>
    where
        Self::Receivers: AsMut<Receiver<T>>,
    {
        self.receivers.as_mut()
    }

    fn senders(&self) -> &Self::Senders {
        &self.senders
    }

    fn sender<T>(&mut self) -> &Sender<T>
    where
        Self::Senders: AsRef<Sender<T>>,
    {
        self.senders.as_ref()
    }
}
