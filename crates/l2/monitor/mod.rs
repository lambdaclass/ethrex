// TODO: Handle this expects
#![expect(clippy::expect_used)]
#![expect(clippy::indexing_slicing)]
#[expect(clippy::result_large_err)]
pub(crate) mod app;
pub(crate) mod utils;
pub(crate) mod widget;

pub use app::EthrexMonitor;
