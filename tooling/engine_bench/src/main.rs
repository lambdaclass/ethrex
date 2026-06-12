//! `engine_bench` — drive ethrex over JSON-RPC and REST/SSZ transports to
//! measure the engine API delta across every fork era. See README.md.
//!
//! Default mode self-hosts one devnet per fork (Paris → Amsterdam), produces
//! blocks through the engine API, and benches each. With --url it benches an
//! existing node instead, auto-detecting its fork.

mod cli;
mod devnet;
#[path = "../../../crates/networking/rpc/benches/fixtures.rs"]
mod fixtures;
mod jwt;
mod output;
mod setup;
mod stats;
mod transports;
mod workloads;

use clap::Parser;
use cli::{ForkArg, Workload};
use eyre::{Context, Result, eyre};
use rand::Rng;
use reqwest::Client;
use std::time::Duration;
use workloads::{IterationRecord, SYNTHETIC_PAYLOAD_ID, WorkloadContext};

/// Blobs endpoint versions exercised per fork.
const BLOBS_VERSIONS: [u8; 3] = [1, 2, 3];

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    let args = cli::Args::parse();
    let client = Client::builder().timeout(Duration::from_secs(30)).build()?;

    let blob_hashes = match &args.blob_hashes_file {
        Some(path) => {
            let hashes = setup::load_blob_hashes(path)?;
            println!(
                "loaded {} versioned hashes from {}",
                hashes.len(),
                path.display()
            );
            hashes
        }
        None => {
            if args.workloads.contains(&Workload::Blobs) {
                eprintln!(
                    "WARNING: no --blob-hashes-file; using random hashes — every blob entry will \
                     miss and the response carries no blob data (hits column will read 0)."
                );
            }
            fixtures::blob_versioned_hashes(
                fixtures::DEFAULT_SEED,
                fixtures::DEFAULT_BLOB_REQUEST_COUNT,
            )
        }
    };

    println!(
        "running {} iterations (+{} warmup) per (fork, workload, transport)",
        args.iterations, args.warmup
    );

    let mut all_records = Vec::new();
    match (&args.url, &args.jwt_path) {
        (Some(url), Some(jwt_path)) => {
            let secret = jwt::load_secret(jwt_path)?;
            let fork = setup::detect_fork(&client, url, &secret)
                .await
                .context("detecting the node's fork")?;
            println!("detected fork: {}", fork.path());
            run_suite(
                &client,
                url,
                &secret,
                fork,
                &args,
                &blob_hashes,
                &mut all_records,
            )
            .await?;
        }
        (Some(_), None) => return Err(eyre!("--url requires --jwt-path")),
        (None, _) => {
            let stamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_secs();
            let run_dir = args
                .datadir_base
                .clone()
                .unwrap_or_else(|| std::env::temp_dir().join("engine_bench_devnets"))
                .join(format!("run-{stamp}"));
            std::fs::create_dir_all(&run_dir)
                .with_context(|| format!("creating {}", run_dir.display()))?;
            let jwt_path = run_dir.join("jwt.hex");
            let mut secret = [0u8; 32];
            rand::thread_rng().fill(&mut secret);
            std::fs::write(&jwt_path, hex::encode(secret))?;
            let url = format!("http://127.0.0.1:{}", args.devnet_port);

            for fork in ForkArg::ALL {
                println!("── {} ──", fork.path());
                let fork_dir = run_dir.join(fork.path());
                std::fs::create_dir_all(&fork_dir)?;
                let genesis = devnet::write_genesis(fork, &fork_dir)?;
                let node = devnet::spawn(
                    &args.ethrex_bin,
                    &genesis,
                    &fork_dir.join("data"),
                    &jwt_path,
                    args.devnet_port,
                )
                .await?;
                // Bodies need the whole requested range on-chain.
                let target = args.bodies_from + args.bodies_count;
                let produce = devnet::produce_blocks(&client, &url, &secret, fork, target).await;
                let result = match produce {
                    Ok(()) => {
                        run_suite(
                            &client,
                            &url,
                            &secret,
                            fork,
                            &args,
                            &blob_hashes,
                            &mut all_records,
                        )
                        .await
                    }
                    Err(e) => Err(e),
                };
                node.stop().await?;
                result.with_context(|| {
                    format!(
                        "fork {} failed — node log: {}/data/node.log",
                        fork.path(),
                        fork_dir.display()
                    )
                })?;
            }

            if args.keep_devnets {
                println!("devnet data kept at {}", run_dir.display());
            } else {
                std::fs::remove_dir_all(&run_dir)
                    .with_context(|| format!("cleaning up {}", run_dir.display()))?;
            }
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

/// Run every requested (workload, transport) cell for one fork; the blobs
/// workload expands over all endpoint versions.
#[allow(clippy::too_many_arguments)]
async fn run_suite(
    client: &Client,
    url: &str,
    secret: &[u8],
    fork: ForkArg,
    args: &cli::Args,
    blob_hashes: &[ethrex_common::H256],
    all_records: &mut Vec<IterationRecord>,
) -> Result<()> {
    let ctx = resolve_context(client, url, secret, fork, args, blob_hashes).await?;

    for &workload in &args.workloads {
        for &transport in &args.transports {
            if workload == Workload::Blobs {
                for version in BLOBS_VERSIONS {
                    let mut vctx = ctx.clone();
                    vctx.blobs_version = version;
                    let records =
                        workloads::run_one(client, url, secret, &vctx, workload, transport).await?;
                    println!(
                        "  {} / {workload:?}(v{version}) / {transport:?}: {} records",
                        fork.path(),
                        records.len()
                    );
                    all_records.extend(records);
                }
            } else {
                let records =
                    workloads::run_one(client, url, secret, &ctx, workload, transport).await?;
                println!(
                    "  {} / {workload:?} / {transport:?}: {} records",
                    fork.path(),
                    records.len()
                );
                all_records.extend(records);
            }
        }
    }
    Ok(())
}

async fn resolve_context(
    client: &Client,
    url: &str,
    secret: &[u8],
    fork: ForkArg,
    args: &cli::Args,
    blob_hashes: &[ethrex_common::H256],
) -> Result<WorkloadContext> {
    let payload_id = if let Some(id) = &args.payload_id {
        id.clone()
    } else if args.workloads.contains(&Workload::GetPayload) {
        match setup::acquire_payload_id(client, url, secret, fork).await {
            Ok(id) => {
                println!("  {}: acquired payload id {id}", fork.path());
                id
            }
            Err(e) => {
                eprintln!(
                    "WARNING [{}]: could not acquire a real payload id ({e}); getPayload \
                     degrades to an error round-trip. Pass --payload-id to override.",
                    fork.path()
                );
                SYNTHETIC_PAYLOAD_ID.to_string()
            }
        }
    } else {
        SYNTHETIC_PAYLOAD_ID.to_string()
    };

    if args.bodies_count > 32 && args.workloads.contains(&Workload::Bodies) {
        eprintln!(
            "WARNING: --bodies-count {} exceeds the REST/SSZ MAX_BODIES_PER_REQUEST (32); the \
             SSZ side will error while JSON still answers, making the rows incomparable.",
            args.bodies_count
        );
    }

    Ok(WorkloadContext {
        fork,
        blobs_version: 3,
        payload_id,
        blob_hashes: blob_hashes.to_vec(),
        bodies_from: args.bodies_from,
        bodies_count: args.bodies_count,
        iterations: args.iterations,
        warmup: args.warmup,
    })
}
