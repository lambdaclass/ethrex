//! snap/2 state synchronization (EIP-8189).
//!
//! Where snap/1 reconciles a range download by fetching individual trie nodes,
//! snap/2 removes `GetTrieNodes` outright and reconciles by applying block
//! access lists to the downloaded flat state, then rebuilding the tries from
//! it. The algorithm is specified in devp2p `caps/snap.md`,
//! "Synchronization algorithm".

pub mod apply;
pub mod catchup;
pub mod cursor;
pub mod download;
pub mod flat;
pub mod generate;
pub mod worker;

pub use apply::{FlatApplyStats, apply_bal_flat};
pub use catchup::{MAX_CATCH_UP_BLOCKS, catch_up, catch_up_exceeds_retention, gap_headers};
pub use cursor::{DownloadCursor, HashRange};
pub use download::download_state;
pub use flat::FlatState;
pub use generate::reconstruct_and_verify;
