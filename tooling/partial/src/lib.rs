//! Partial stateful node mode for ethrex.
//!
//! A partial stateful node holds only the accounts and storage its users read, proven
//! against headers a light client verified, and keeps them current with EIP-7928 block
//! access lists instead of executing blocks. EIP-8440 execution chain proofs, when
//! available, establish that the chain it follows executed validly since its checkpoint.
//!
//! [`verify`] is the trust boundary: untrusted peer bytes become verified values there
//! and nowhere else. [`state`] holds partial state built only from verified values,
//! bounded by the keys [`working`] keeps.

pub mod chain_proofs;
pub mod exec;
pub mod fetch;
pub mod follow;
pub mod light_client;
pub mod node;
pub mod p2p_adapter;
pub mod peers;
pub mod rpc;
pub mod state;
#[cfg(test)]
pub(crate) mod testing;
pub mod txbook;
pub mod verify;
pub mod working;
