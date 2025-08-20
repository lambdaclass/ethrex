// TODO: Handle this expects
#[expect(clippy::result_large_err)]
pub mod app;
pub mod utils;
pub mod widget;

pub use app::EthrexMonitor;
