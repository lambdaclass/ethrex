use clap::Parser;

#[derive(Parser)]
#[command(name = "ethrex-repl", about = "Interactive REPL for Ethereum JSON-RPC")]
struct Cli {
    /// JSON-RPC endpoint URL
    #[arg(short = 'e', long, default_value = "http://localhost:8545")]
    endpoint: String,

    /// Path to command history file
    #[arg(long, default_value = "~/.ethrex/history")]
    history_file: String,

    /// Execute a single command and exit
    #[arg(short = 'x', long)]
    execute: Option<String>,
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    ethrex_repl::run(cli.endpoint, cli.history_file, cli.execute).await;
}
