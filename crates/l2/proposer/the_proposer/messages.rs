#[derive(Debug, Clone)]
pub struct CommitterToCommitter;

#[derive(Debug, Clone)]
pub enum L1WatcherToL1Watcher {
    WatchL1ToL2Tx,
}

#[derive(Debug, Clone)]
pub struct BlockProducerToBlockProducer;
