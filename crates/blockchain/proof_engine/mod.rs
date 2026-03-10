//! EIP-8025: Execution Layer Triggerable Proofs — proof engine module.
//!
//! This module contains the ProofEngine configuration, types, core engine,
//! and L1 ProofCoordinator used by the Engine API proof endpoints.

pub mod config;
pub mod coordinator;
pub mod engine;
pub mod types;
