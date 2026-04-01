//! Minimal L1 prover binary for EIP-8025 demo.
//!
//! Connects to the L1 ProofCoordinator, pulls ProgramInput,
//! executes via ExecBackend, and submits the proof back.
//!
//! Usage:
//!   cargo run --features eip-8025 -p ethrex-prover --bin l1_prover -- \
//!     --coordinator http://localhost:9100

use clap::Parser;
use ethrex_guest_program::input::ProgramInput;
use ethrex_prover::BackendType;
use ethrex_prover::prover::{ProverPullConfig, start_prover};
use url::Url;

#[derive(Parser)]
#[command(name = "l1-prover", about = "EIP-8025 L1 execution prover")]
struct Cli {
    /// Coordinator TCP endpoint
    #[arg(long, default_value = "http://localhost:9100")]
    coordinator: String,

    /// Polling interval in milliseconds
    #[arg(long, default_value_t = 2000)]
    poll_interval_ms: u64,
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

    let cli = Cli::parse();
    let endpoint: Url = cli.coordinator.parse().expect("Invalid coordinator URL");

    println!("L1 Prover starting (ExecBackend)");
    println!("  Coordinator: {endpoint}");
    println!("  Poll interval: {}ms", cli.poll_interval_ms);

    let config = ProverPullConfig {
        proof_coordinator_endpoints: vec![endpoint],
        proving_time_ms: cli.poll_interval_ms,
        timed: false,
        commit_hash: String::new(),
    };

    tokio::select! {
        _ = start_prover::<ProgramInput>(BackendType::Exec, config) => {}
        _ = tokio::signal::ctrl_c() => {
            println!("\nShutting down...");
        }
    }
}
