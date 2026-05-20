//! `engine_bench` — drive ethrex over JSON-RPC and REST/SSZ transports
//! to measure the engine API delta. See README.md.

mod cli;
mod fixtures;
mod jwt;
mod output;
mod stats;
mod transports;
mod workloads;

use clap::Parser;
use eyre::Result;
use reqwest::Client;
use std::time::Duration;

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    let args = cli::Args::parse();
    let secret = jwt::load_secret(&args.jwt_path)?;

    let client = Client::builder().timeout(Duration::from_secs(30)).build()?;

    println!(
        "running {} iterations per (transport, workload)",
        args.iterations
    );

    let mut all_records = Vec::new();
    for &workload in &args.workloads {
        for &transport in &args.transports {
            let records = workloads::run_one(
                &client,
                &args.url,
                &secret,
                workload,
                transport,
                args.iterations,
            )
            .await?;
            println!("  {workload:?} / {transport:?}: {} records", records.len());
            all_records.extend(records);
        }
    }

    let summaries = output::aggregate(&all_records);
    output::print_markdown(&summaries);

    if let Some(csv_path) = &args.csv_out {
        output::write_csv(&all_records, csv_path)?;
        println!("wrote per-iteration CSV to {}", csv_path.display());
    }

    Ok(())
}
