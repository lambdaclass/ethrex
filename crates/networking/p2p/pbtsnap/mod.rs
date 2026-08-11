//! `pbtsnap/1` — server side.
//!
//! The wire types live in `rlpx::pbtsnap`; this module is the half that reads
//! the node's own binary trie to answer them. There is no client here yet — the
//! sync driver is a later slice.
//!
//! - `server`: request processing
//! - `error`: the serving error type, and what a client must read into it

pub mod error;
mod server;

pub use error::PbtSnapError;
pub use server::process_pbt_leaf_range_request;
