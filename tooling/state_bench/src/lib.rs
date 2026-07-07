//! Library surface of the cold-state benchmark harness.
//!
//! Only the pieces that other crates (notably the `ethrex-test` integration
//! suite) or the run orchestration need to reuse are exposed here. The
//! subcommand implementations live in the binary crate (`main.rs`).

pub mod recording_backend;
