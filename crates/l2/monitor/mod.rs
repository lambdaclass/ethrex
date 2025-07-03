// TODO: Handle this expects
#![expect(clippy::expect_used)]
#![expect(clippy::panic)]
#![expect(clippy::indexing_slicing)]

pub(crate) mod app;
pub(crate) mod utils;
pub(crate) mod widget;

pub use app::EthrexMonitor;

use crate::SequencerConfig;
use crate::sequencer::errors::SequencerError;

pub async fn start_monitor(cfg: SequencerConfig) -> Result<(), SequencerError> {
    let app = EthrexMonitor::new(&cfg).await;
    app.start().await?;
    Ok(())
}
