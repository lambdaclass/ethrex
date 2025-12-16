mod cli;
mod utils;

use crate::cli::CLI;
use clap::Parser;

#[tokio::main]
async fn main() {
    let CLI { command } = CLI::parse();

    command.run().await;
}
