//! `state-bench`: cold-state read+write benchmark harness for the BAL
//! parallel import path.
//!
//! Subcommands: `gen-state` builds a synthetic state fixture, `gen-workload`
//! builds real blocks + BALs on top of it, `run` times a cold import of that
//! workload (spawning hidden `_warmup`/`_measure`/`_reset` subprocesses), and
//! `compare` diffs two `run` metrics logs (e.g. branch A vs branch B).

mod compare;
mod gen_state;
mod gen_workload;
mod manifest;
mod run;

use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand};
use tracing::info;
use tracing_subscriber::{EnvFilter, fmt};

use ethrex_storage::STORE_SCHEMA_VERSION;

use gen_state::GenStateArgs;
use gen_workload::GenWorkloadArgs;
use run::{MeasureArgs, ResetArgs, RunArgs, WarmupArgs};

#[derive(Parser)]
#[command(
    name = "state-bench",
    about = "Cold-state read/write benchmark harness for the BAL parallel import path"
)]
struct Cli {
    /// Worker count for parallel steps. Defaults to the ambient CPU count.
    #[arg(long, global = true)]
    jobs: Option<usize>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Build a synthetic, deterministic state fixture on disk.
    GenState {
        /// Datadir to create and populate (must be empty / non-existent).
        #[arg(long)]
        datadir: PathBuf,
        /// Number of small storage-bearing accounts.
        #[arg(long, default_value_t = 1000)]
        num_small_accounts: u64,
        /// Storage slots per small account.
        #[arg(long, default_value_t = 8)]
        slots_per_account: u64,
        /// Target size of the mega account's storage (decimal GB).
        #[arg(long, default_value_t = 1.0)]
        mega_account_gb: f64,
        /// Deterministic seed for all derivations.
        #[arg(long)]
        seed: u64,
        /// Base genesis file; only its chain config is used.
        #[arg(long)]
        genesis: PathBuf,
        /// Enable RocksDB internal statistics and log them (plus per-CF
        /// compaction/write indicators for `STORAGE_TRIE_NODES`) at every
        /// mega storage trie progress checkpoint. Diagnostic-only; off by
        /// default so normal runs aren't spammed.
        #[arg(long, default_value_t = false)]
        rocksdb_stats: bool,
    },
    /// Produce a workload of real blocks + captured BALs (phase 3).
    GenWorkload {
        /// Datadir produced by `gen-state` (read-only; a throwaway copy is used).
        #[arg(long)]
        datadir: PathBuf,
        /// Output path for the RLP-concatenated blocks (`chain.rlp`).
        #[arg(long)]
        out_chain: PathBuf,
        /// Output path for the RLP-concatenated BALs (`bals.rlp`).
        #[arg(long)]
        out_bals: PathBuf,
        /// Number of workload blocks to build.
        #[arg(long, default_value_t = 1000)]
        num_blocks: u64,
        /// Cold storage reads (SLOAD of seeded slots) per block.
        #[arg(long, default_value_t = 8)]
        reads_per_block: u64,
        /// Cold storage writes (SSTORE of fresh slots) per block.
        #[arg(long, default_value_t = 4)]
        writes_per_block: u64,
        /// Fraction of touched slots that target the mega account (0.0..=1.0).
        #[arg(long, default_value_t = 0.5)]
        mega_fraction: f64,
        /// Base genesis file used at gen-state time; re-applies the chain config
        /// (must activate Amsterdam so blocks carry a BAL).
        #[arg(long)]
        genesis: PathBuf,
        /// After writing artifacts, re-import them onto a fresh copy of the
        /// datadir to validate every block + BAL end-to-end.
        #[arg(long, default_value_t = false)]
        verify_reimport: bool,
    },
    /// Run the timed cold import and record metrics.
    Run(RunArgs),
    /// Internal: warmup worker (records the undo log + pristine digest). Hidden.
    #[command(name = "_warmup", hide = true)]
    Warmup(WarmupArgs),
    /// Internal: measure worker (one cold timed import + metrics line). Hidden.
    #[command(name = "_measure", hide = true)]
    Measure(MeasureArgs),
    /// Internal: reset worker (replay undo log, assert pristine). Hidden.
    #[command(name = "_reset", hide = true)]
    Reset(ResetArgs),
    /// Compare two `run` metrics logs (e.g. branch A vs branch B).
    Compare {
        /// First out-log (typically the baseline branch).
        log_a: PathBuf,
        /// Second out-log (typically the candidate branch).
        log_b: PathBuf,
    },
}

fn resolve_jobs(jobs: Option<usize>) -> usize {
    jobs.unwrap_or_else(|| {
        std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1)
    })
}

#[tokio::main]
async fn main() -> Result<()> {
    fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .with_target(false)
        .init();

    let cli = Cli::parse();
    let jobs = resolve_jobs(cli.jobs);

    // Record the linked store schema version so a manifest can be checked
    // against the client it was built with.
    info!(
        schema_version = STORE_SCHEMA_VERSION,
        jobs, "state-bench starting"
    );

    match cli.command {
        Command::GenState {
            datadir,
            num_small_accounts,
            slots_per_account,
            mega_account_gb,
            seed,
            genesis,
            rocksdb_stats,
        } => {
            gen_state::run(GenStateArgs {
                datadir,
                num_small_accounts,
                slots_per_account,
                mega_account_gb,
                seed,
                genesis,
                jobs,
                rocksdb_stats,
            })
            .await
        }
        Command::GenWorkload {
            datadir,
            out_chain,
            out_bals,
            num_blocks,
            reads_per_block,
            writes_per_block,
            mega_fraction,
            genesis,
            verify_reimport,
        } => {
            gen_workload::run(GenWorkloadArgs {
                datadir,
                out_chain,
                out_bals,
                num_blocks,
                reads_per_block,
                writes_per_block,
                mega_fraction,
                genesis,
                verify_reimport,
                jobs,
            })
            .await
        }
        Command::Run(args) => run::run_parent(args, jobs).await,
        Command::Warmup(args) => run::warmup(args, jobs).await,
        Command::Measure(args) => run::measure(args, jobs).await,
        Command::Reset(args) => run::reset(args).await,
        Command::Compare { log_a, log_b } => compare::run(&log_a, &log_b),
    }
}
