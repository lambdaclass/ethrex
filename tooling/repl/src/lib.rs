pub mod client;
pub mod commands;
pub mod completer;
mod ens;
pub mod formatter;
pub mod parser;
pub mod repl;

use client::RpcClient;
use repl::Repl;

/// Run the REPL with the given configuration.
///
/// If `execute` is `Some`, runs a single command and exits.
/// Otherwise, starts the interactive REPL loop.
pub async fn run(endpoint: String, history_file: String, execute: Option<String>) {
    let history_path = expand_tilde(&history_file);
    let client = RpcClient::new(endpoint);

    if let Some(command) = execute {
        let repl = Repl::new(client, history_path);
        let result = repl.execute_command(&command).await;
        if !result.is_empty() {
            println!("{result}");
        }
        return;
    }

    let mut repl = Repl::new(client, history_path);
    repl.run().await;
}

fn expand_tilde(path: &str) -> String {
    if path.starts_with('~')
        && let Ok(home) = std::env::var("HOME")
    {
        return path.replacen('~', &home, 1);
    }
    path.to_string()
}
