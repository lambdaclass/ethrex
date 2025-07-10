// TODO: Handle this expects
#![expect(clippy::expect_used)]
#![expect(clippy::panic)]
#![expect(clippy::indexing_slicing)]

pub(crate) mod app;
pub(crate) mod utils;
pub(crate) mod widget;

pub use app::EthrexMonitor;
use ethrex_storage::Store;
use ethrex_storage_rollup::StoreRollup;

use crate::SequencerConfig;
use crate::based::sequencer_state::SequencerState;
use crate::sequencer::errors::SequencerError;

pub async fn start_monitor(
    sequencer_state: SequencerState,
    store: Store,
    rollup_store: StoreRollup,
    cfg: SequencerConfig,
) -> Result<(), SequencerError> {
    let app = EthrexMonitor::new(sequencer_state, store, rollup_store, &cfg).await;
    app.start().await?;
    Ok(())
}
