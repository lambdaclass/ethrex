//! Measurement harness and checks for the comparison of proposals that
//! authorize transactions with a post-quantum signature scheme.
//!
//! `lifecycle.rs` and `checks.rs` are the same on every branch under
//! comparison. `adapter.rs` holds everything that belongs to the proposal this
//! branch implements.

mod adapter;
mod checks;
mod lifecycle;
