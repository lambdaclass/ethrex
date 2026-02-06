//! Dev-mode block builder for `ethrex --dev`.
//!
//! Implemented as a GenServer that receives transactions via cast messages and
//! builds blocks using `ethrex-blockchain`.
//!
//! - **On-demand mode** (default): builds a block immediately per transaction
//! - **Interval mode** (`--dev.block-time`): collects transactions and builds at intervals

pub mod banner;
pub mod builder;
pub mod error;

pub use banner::display_banner;
pub use builder::{BlockBuilder, BlockBuilderConfig, CastMsg};
pub use error::BlockBuilderError;
