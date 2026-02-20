use clap::{Parser, Subcommand, ValueEnum};
use node_replay::{
    checkpoint, commands, planner, runner,
    types::{
        CommandResponse, ErrorResponse, Finality as TypesFinality, ReplayMode as TypesReplayMode,
        RunManifest,
    },
    workspace::Workspace,
};
use std::process;
use tracing_subscriber::EnvFilter;

#[derive(Parser)]
#[command(
    name = "node-replay",
    about = "Agent-friendly replay system for ethrex block processing"
)]
struct Cli {
    /// Workspace directory for all artifacts
    #[arg(long)]
    workspace: std::path::PathBuf,

    /// Disable colored output
    #[arg(long, global = true)]
    no_color: bool,

    /// Request ID for correlation
    #[arg(long, global = true)]
    request_id: Option<String>,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Manage checkpoints
    Checkpoint {
        #[command(subcommand)]
        command: CheckpointCommands,
    },
    /// Plan a replay run
    Plan {
        /// Checkpoint ID to use
        #[arg(long)]
        checkpoint: String,
        /// Number of blocks to replay
        #[arg(long)]
        blocks: u64,
        /// Finality level for block selection
        #[arg(long, default_value = "head")]
        finality: Finality,
        /// Path to the live node datadir (used to read canonical block hashes)
        #[arg(long)]
        datadir: std::path::PathBuf,
    },
    /// Execute a planned replay
    Run {
        /// Path to run manifest
        #[arg(long)]
        manifest: std::path::PathBuf,
        /// Replay mode
        #[arg(long, default_value = "isolated")]
        mode: ReplayMode,
    },
    /// Check run status
    Status {
        /// Run ID
        #[arg(long)]
        run: String,
    },
    /// Resume a paused/failed run
    Resume {
        /// Run ID
        #[arg(long)]
        run: String,
    },
    /// Cancel a run
    Cancel {
        /// Run ID
        #[arg(long)]
        run: String,
    },
    /// Verify run results
    Verify {
        /// Run ID
        #[arg(long)]
        run: String,
    },
    /// Generate run report
    Report {
        /// Run ID
        #[arg(long)]
        run: String,
    },
}

#[derive(Subcommand)]
enum CheckpointCommands {
    /// Create a new checkpoint from a live datadir
    Create {
        /// Path to the live node datadir
        #[arg(long)]
        datadir: std::path::PathBuf,
        /// Human-readable label
        #[arg(long)]
        label: String,
    },
    /// List all checkpoints
    List,
}

#[derive(Clone, ValueEnum)]
enum Finality {
    Safe,
    Finalized,
    Head,
}

impl From<Finality> for TypesFinality {
    fn from(f: Finality) -> Self {
        match f {
            Finality::Safe => TypesFinality::Safe,
            Finality::Finalized => TypesFinality::Finalized,
            Finality::Head => TypesFinality::Head,
        }
    }
}

impl From<ReplayMode> for TypesReplayMode {
    fn from(m: ReplayMode) -> Self {
        match m {
            ReplayMode::Isolated => TypesReplayMode::Isolated,
            ReplayMode::StopLiveNode => TypesReplayMode::StopLiveNode,
        }
    }
}

#[derive(Clone, ValueEnum)]
enum ReplayMode {
    Isolated,
    StopLiveNode,
}

/// Print a JSON response to stdout.
fn print_json<T: serde::Serialize>(value: &T) {
    println!("{}", serde_json::to_string_pretty(value).unwrap());
}

/// Print a JSON error response and exit with the given code.
fn exit_error(code: &str, message: &str, request_id: Option<&str>, exit_code: i32) -> ! {
    let response: CommandResponse<serde_json::Value> = CommandResponse {
        success: false,
        data: None,
        error: Some(ErrorResponse {
            code: code.to_string(),
            message: message.to_string(),
        }),
        request_id: request_id.map(String::from),
    };
    eprintln!("{}", serde_json::to_string_pretty(&response).unwrap());
    process::exit(exit_code);
}

#[cfg(unix)]
fn raise_nofile_soft_limit_best_effort() {
    use libc::{RLIMIT_NOFILE, getrlimit, rlimit, setrlimit};

    let mut current = rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    // Best effort: ignore failures and continue with current process limits.
    unsafe {
        if getrlimit(RLIMIT_NOFILE, &mut current) == 0 && current.rlim_cur < current.rlim_max {
            let target = rlimit {
                rlim_cur: current.rlim_max,
                rlim_max: current.rlim_max,
            };
            let _ = setrlimit(RLIMIT_NOFILE, &target);
        }
    }
}

#[cfg(not(unix))]
fn raise_nofile_soft_limit_best_effort() {}

#[tokio::main]
async fn main() {
    raise_nofile_soft_limit_best_effort();

    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let cli = Cli::parse();
    let request_id = cli.request_id.as_deref();

    match run(cli.command, &cli.workspace, request_id).await {
        Ok(()) => {}
        Err(e) => {
            exit_error(e.error_code(), &e.to_string(), request_id, e.exit_code());
        }
    }
}

async fn run(
    command: Commands,
    workspace_path: &std::path::Path,
    request_id: Option<&str>,
) -> Result<(), node_replay::errors::ReplayError> {
    match command {
        Commands::Checkpoint { command } => match command {
            CheckpointCommands::Create { datadir, label } => {
                let workspace = Workspace::init(workspace_path)?;
                let meta = checkpoint::create_checkpoint(&workspace, &datadir, &label).await?;
                let response = CommandResponse::success(Some(meta), request_id.map(String::from));
                print_json(&response);
            }
            CheckpointCommands::List => {
                let workspace = Workspace::init(workspace_path)?;
                let checkpoints = checkpoint::list_checkpoints(&workspace)?;
                let response =
                    CommandResponse::success(Some(checkpoints), request_id.map(String::from));
                print_json(&response);
            }
        },

        Commands::Plan {
            checkpoint,
            blocks,
            finality,
            datadir,
        } => {
            let workspace = Workspace::init(workspace_path)?;
            let types_finality: TypesFinality = finality.into();
            let manifest =
                planner::plan_run(&workspace, &checkpoint, blocks, &types_finality, &datadir)
                    .await?;
            let response = CommandResponse::success(Some(manifest), request_id.map(String::from));
            print_json(&response);
        }

        Commands::Status { run } => {
            let workspace = Workspace::open(workspace_path)?;
            let status = commands::get_status(&workspace, &run)?;
            let response = CommandResponse::success(Some(status), request_id.map(String::from));
            print_json(&response);
        }

        Commands::Run {
            manifest: manifest_path,
            mode,
        } => {
            let workspace = Workspace::open(workspace_path)?;
            let manifest_data = std::fs::read_to_string(&manifest_path).map_err(|e| {
                node_replay::errors::ReplayError::InvalidArgument(format!(
                    "failed to read manifest: {e}"
                ))
            })?;
            let manifest: RunManifest = serde_json::from_str(&manifest_data)?;
            let types_mode: TypesReplayMode = mode.into();
            let summary = runner::execute_run(&workspace, &manifest, &types_mode, false).await?;
            let response = CommandResponse::success(Some(summary), request_id.map(String::from));
            print_json(&response);
        }
        Commands::Resume { run } => {
            let workspace = Workspace::open(workspace_path)?;
            // resume_run validates state (Paused/Failed), acquires lock, clears
            // error, and transitions to Running. execute_run is then called with
            // lock_held=true so it skips lock acquisition and the RunAlreadyRunning
            // guard.
            let status = commands::resume_run(&workspace, &run)?;
            if status.state == node_replay::types::RunState::Completed {
                // Already completed — idempotent, just return the summary.
                let summary = workspace.read_run_summary(&run)?;
                let response =
                    CommandResponse::success(Some(summary), request_id.map(String::from));
                print_json(&response);
            } else {
                let manifest = workspace.read_run_manifest(&run)?;
                let mode = manifest.mode.clone();
                let summary = runner::execute_run(&workspace, &manifest, &mode, true).await?;
                let response =
                    CommandResponse::success(Some(summary), request_id.map(String::from));
                print_json(&response);
            }
        }
        Commands::Cancel { run } => {
            let workspace = Workspace::open(workspace_path)?;
            let status = commands::cancel_run(&workspace, &run)?;
            let response = CommandResponse::success(Some(status), request_id.map(String::from));
            print_json(&response);
        }
        Commands::Verify { run } => {
            let workspace = Workspace::open(workspace_path)?;
            let report = commands::verify_run(&workspace, &run)?;
            let response = CommandResponse::success(Some(report), request_id.map(String::from));
            print_json(&response);
        }
        Commands::Report { run } => {
            let workspace = Workspace::open(workspace_path)?;
            let report = commands::get_report(&workspace, &run)?;
            let response = CommandResponse::success(Some(report), request_id.map(String::from));
            print_json(&response);
        }
    }

    Ok(())
}
