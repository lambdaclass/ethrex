//! `pbtsnap/1` — the halves of the protocol that touch this node's own state.
//!
//! The wire types live in `rlpx::pbtsnap`; this module answers them from the
//! node's binary trie and asks them of peers. The *driver* that decides what to
//! ask for lives in `sync::pbt_snap`, one layer up.
//!
//! - `server`: request processing
//! - `client`: the transport seam the driver runs over, and its real impl
//! - `error`: the serving error type, and what a client must read into it

pub mod client;
pub mod error;
mod server;

/// The protocol over a real connection between two in-process nodes. Separate
/// from `server`'s unit tests because it tests the seams *between* the pieces
/// those cover — negotiation, dispatch and response routing — which no
/// function-level test reaches.
#[cfg(test)]
mod live_tests;

pub use client::{PbtProviderError, PbtSnapProvider, PeerPbtSnapProvider};
pub use error::PbtSnapError;
pub use server::process_pbt_leaf_range_request;
