// TODO: Handle this expects
#[expect(clippy::result_large_err)]
pub mod app;
pub mod config;
pub mod error;
pub mod utils;
pub mod widget;

pub use app::EthrexMonitor;
pub use config::MonitorConfig;
pub use error::MonitorError;
