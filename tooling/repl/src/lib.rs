pub mod client;
pub mod commands;
pub mod completer;
mod ens;
pub mod formatter;
pub mod parser;
pub mod proof_callback;
pub mod repl;
pub mod variables;

use client::RpcClient;
use repl::Repl;

/// Run the REPL with the given configuration.
///
/// If `execute` is `Some`, runs a single command and exits.
/// Otherwise, starts the interactive REPL loop.
pub async fn run(
    endpoint: String,
    authrpc_endpoint: String,
    authrpc_jwtsecret: Option<String>,
    history_file: String,
    execute: Option<String>,
    proof_callback_port: u16,
    proof_callback_timeout: u64,
) {
    let history_path = expand_tilde(&history_file);
    let client = RpcClient::new(endpoint);

    let authrpc_client = authrpc_jwtsecret.map(|path| {
        let secret = read_jwtsecret_file(&path);
        RpcClient::new_with_jwt(authrpc_endpoint, secret)
    });

    if let Some(command) = execute {
        let repl = Repl::new(
            client,
            authrpc_client,
            history_path,
            proof_callback_port,
            proof_callback_timeout,
        );
        let result = repl.execute_command(&command).await;
        if !result.is_empty() {
            println!("{result}");
        }
        return;
    }

    let mut repl = Repl::new(
        client,
        authrpc_client,
        history_path,
        proof_callback_port,
        proof_callback_timeout,
    );
    repl.run().await;
}

fn read_jwtsecret_file(path: &str) -> Vec<u8> {
    let content = std::fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("Failed to read JWT secret from {path}: {e}"));
    let hex_str = content.trim().strip_prefix("0x").unwrap_or(content.trim());
    hex::decode(hex_str).unwrap_or_else(|e| panic!("Invalid hex in JWT secret file: {e}"))
}

fn expand_tilde(path: &str) -> String {
    if path.starts_with('~')
        && let Ok(home) = std::env::var("HOME")
    {
        return path.replacen('~', &home, 1);
    }
    path.to_string()
}
