use ethrex_common::types::BlockNumber;
use ethrex_vm::backends::BlockExecutionResult;
use tokio::sync::broadcast::{self, Receiver, Sender};
use tracing::warn;

/// Proposer will push execution results into the cache so other components can retrieve them,
/// without having to re-execute. The cache is implemented as a mpmc (broadcast) channel.
pub struct ExecutionCache(Sender<(BlockNumber, BlockExecutionResult)>);

impl ExecutionCache {
    pub fn new(len: usize) -> Self {
        Self(broadcast::channel(len).0)
    }

    pub fn subscribe(&self) -> Receiver<(BlockNumber, BlockExecutionResult)> {
        self.0.subscribe()
    }

    pub fn push(&self, block_number: BlockNumber, execution_result: BlockExecutionResult) {
        if self.0.send((block_number, execution_result)).is_err() {
            warn!("Execution cache published new result but there are no receivers");
        }
    }

    pub async fn get(
        receiver: &mut Receiver<(BlockNumber, BlockExecutionResult)>,
        block_number: BlockNumber,
    ) -> Option<BlockExecutionResult> {
        match receiver.recv().await {
            Ok(result) if result.0 == block_number => Some(result.1),
            _ => None,
        }
    }
}
