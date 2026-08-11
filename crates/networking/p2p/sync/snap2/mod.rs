//! snap/2 state synchronization (EIP-8189).
//!
//! Where snap/1 reconciles a range download by fetching individual trie nodes,
//! snap/2 removes `GetTrieNodes` outright and reconciles by applying block
//! access lists to the downloaded flat state, then rebuilding the tries from
//! it. The algorithm is specified in devp2p `caps/snap.md`,
//! "Synchronization algorithm".

pub mod cursor;
pub mod flat;

pub use cursor::{DownloadCursor, HashRange};
pub use flat::FlatState;
