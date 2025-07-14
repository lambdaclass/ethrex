// TODO: Handle this expects
#![expect(clippy::expect_used)]
#![expect(clippy::indexing_slicing)]
#[expect(clippy::result_large_err)]
pub(crate) mod app;
pub(crate) mod utils;
pub(crate) mod widget;

pub use app::EthrexMonitor;
use ethrex_storage::Store;
use ethrex_storage_rollup::StoreRollup;

use crate::SequencerConfig;
use crate::based::sequencer_state::SequencerState;
use crate::sequencer::errors::SequencerError;
