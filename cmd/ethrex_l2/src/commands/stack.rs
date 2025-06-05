use crate::{config::EthrexL2Config, utils::config::confirm};
use clap::Subcommand;
use ethrex_common::H256;
use ethrex_common::{
    types::{batch::Batch, bytes_from_blob, BlobsBundle, BlockHeader, BYTES_PER_BLOB},
    Address,
};
use ethrex_l2_common::state_diff::StateDiff;
use ethrex_storage::{EngineType, Store};
use ethrex_storage_rollup::{EngineTypeRollup, StoreRollup};
use eyre::ContextCompat;
use itertools::Itertools;
use secp256k1::SecretKey;
use std::{
    fs::read_dir,
    path::{Path, PathBuf},
};

pub const CARGO_MANIFEST_DIR: &str = env!("CARGO_MANIFEST_DIR");

#[allow(clippy::large_enum_variant)]
#[derive(Subcommand)]
pub(crate) enum Command {
    #[command(
        about = "Initializes the L2 network in the provided L1.",
        long_about = "Initializing an L2 involves deploying and setting up the contracts in the L1 and running an L2 node.",
        visible_alias = "i"
    )]
    Init {
        #[arg(
            long = "skip-l1-deployment",
            help = "Skips L1 deployment. Beware that this will only work if the L1 is already set up. L1 contracts must be present in the config."
        )]
        skip_l1_deployment: bool,
        #[arg(
            long = "start-prover",
            help = "Start ZK Prover for the L2 if set.",
            short = 'p',
            default_value_t = false
        )]
        start_prover: bool,
    },
    #[command(about = "Shutdown the stack.")]
    Shutdown {
        #[arg(long, help = "Shuts down the local L1 node.", default_value_t = true)]
        l1: bool,
        #[arg(long, help = "Shuts down the L2 node.", default_value_t = true)]
        l2: bool,
        #[arg(short = 'y', long, help = "Forces the shutdown without confirmation.")]
        force: bool,
    },
    #[command(about = "Starts the stack.")]
    Start {
        #[arg(long, help = "Starts a local L1 node.", required = false)]
        l1: bool,
        #[arg(long, help = "Starts the L2 node.", required = false)]
        l2: bool,
        #[arg(short = 'y', long, help = "Forces the start without confirmation.")]
        force: bool,
        #[arg(
            long = "start-prover",
            help = "Start ZK Prover for the L2 if set.",
            short = 'p',
            default_value_t = false
        )]
        start_prover: bool,
    },
    #[command(about = "Cleans up the stack. Prompts for confirmation.")]
    Purge {
        #[arg(short = 'y', long, help = "Forces the purge without confirmation.")]
        force: bool,
    },
    #[command(
        about = "Re-initializes the stack. Prompts for confirmation.",
        long_about = "Re-initializing a stack means to shutdown, cleanup, and initialize the stack again. It uses the `shutdown` and `cleanup` commands under the hood."
    )]
    Restart {
        #[arg(short = 'y', long, help = "Forces the restart without confirmation.")]
        force: bool,
    },
    #[command(about = "Reconstructs the L2 state from L1 blobs.")]
    Reconstruct {
        #[arg(short = 'g', long, help = "The genesis file for the L2 network.")]
        genesis: PathBuf,
        #[arg(short = 'b', long, help = "The directory to read the blobs from.")]
        blobs_dir: PathBuf,
        #[arg(short = 's', long, help = "The path to the store.")]
        store_path: PathBuf,
        #[arg(short = 'c', long, help = "Address of the L2 proposer coinbase")]
        coinbase: Address,
    },
}

impl Command {
    pub async fn run(self, cfg: EthrexL2Config) -> eyre::Result<()> {
        let root = std::path::Path::new(CARGO_MANIFEST_DIR)
            .parent()
            .map(std::path::Path::parent)
            .context("Failed to get parent")?
            .context("Failed to get grandparent")?;
        let ethrex_dev_path = root.join("crates/blockchain/dev");
        let l2_crate_path = root.join("crates/l2");
        let contracts_path = l2_crate_path.join("contracts");

        let l1_rpc_url = cfg.network.l1_rpc_url.clone();
        let l2_rpc_url = cfg.network.l2_rpc_url.clone();

        match self {
            Command::Init {
                skip_l1_deployment,
                start_prover,
            } => {
                // Delegate the command whether to init in a local environment
                // or in a testnet. If the L1 RPC URL is localhost, then it is
                // a local environment and the local node needs to be started.
                if l1_rpc_url.contains("localhost") {
                    start_l1(&l2_crate_path, &ethrex_dev_path).await?;
                    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                }
                if !skip_l1_deployment {
                    deploy_l1(&l1_rpc_url, &cfg.wallet.private_key, &contracts_path)?;
                }
                start_l2(root.to_path_buf(), &l2_rpc_url, start_prover).await?;
            }
            Command::Shutdown { l1, l2, force } => {
                if force || (l1 && confirm("Are you sure you want to shutdown the local L1 node?")?)
                {
                    shutdown_l1(&ethrex_dev_path)?;
                }
                if force || (l2 && confirm("Are you sure you want to shutdown the L2 node?")?) {
                    shutdown_l2()?;
                }
            }
            Command::Start {
                l1,
                l2,
                force,
                start_prover,
            } => {
                if force || l1 {
                    start_l1(&l2_crate_path, &ethrex_dev_path).await?;
                }
                if force || l2 {
                    start_l2(root.to_path_buf(), &l2_rpc_url, start_prover).await?;
                }
            }
            Command::Purge { force } => {
                if force || confirm("Are you sure you want to purge the stack?")? {
                    match std::fs::remove_dir_all(root.join("volumes")) {
                        Ok(_) | Err(_) => (),
                    };
                    match std::fs::remove_dir_all(contracts_path.join("out")) {
                        Ok(_) | Err(_) => (),
                    };
                    match std::fs::remove_dir_all(contracts_path.join("lib")) {
                        Ok(_) | Err(_) => (),
                    };
                    match std::fs::remove_dir_all(contracts_path.join("cache")) {
                        Ok(_) | Err(_) => (),
                    };
                } else {
                    println!("Aborted.");
                }
            }
            Command::Restart { force } => {
                if force || confirm("Are you sure you want to restart the stack?")? {
                    Box::pin(async {
                        Self::Shutdown {
                            l1: true,
                            l2: true,
                            force,
                        }
                        .run(cfg.clone())
                        .await
                    })
                    .await?;
                    Box::pin(async { Self::Purge { force }.run(cfg.clone()).await }).await?;
                    Box::pin(async {
                        Self::Init {
                            skip_l1_deployment: false,
                            start_prover: false,
                        }
                        .run(cfg.clone())
                        .await
                    })
                    .await?;
                } else {
                    println!("Aborted.");
                }
            }
            Command::Reconstruct {
                genesis,
                blobs_dir,
                store_path,
                coinbase,
            } => {
                // Init stores
                let store = Store::new_from_genesis(
                    store_path.to_str().expect("Invalid store path"),
                    EngineType::Libmdbx,
                    genesis.to_str().expect("Invalid genesis path"),
                )
                .await?;
                let rollup_store = StoreRollup::new(
                    store_path
                        .join("./rollup_store")
                        .to_str()
                        .expect("Invalid store path"),
                    EngineTypeRollup::Libmdbx,
                )?;
                rollup_store
                    .init()
                    .await
                    .expect("Failed to init rollup store");

                // Get genesis
                let genesis_header = store.get_block_header(0)?.expect("Genesis block not found");
                let genesis_block_hash = genesis_header.hash();

                let mut new_trie = store
                    .state_trie(genesis_block_hash)?
                    .expect("Cannot open state trie");

                let mut last_block_number = 0;

                // Iterate over each blob
                let files: Vec<std::fs::DirEntry> = read_dir(blobs_dir)?.try_collect()?;
                for (file_number, file) in files
                    .into_iter()
                    .sorted_by_key(|f| f.file_name())
                    .enumerate()
                {
                    let batch_number = file_number as u64 + 1;
                    let blob = std::fs::read(file.path())?;

                    if blob.len() != BYTES_PER_BLOB {
                        panic!("Invalid blob size");
                    }

                    // Decode state diff from blob
                    let blob = bytes_from_blob(blob.into());
                    let state_diff = StateDiff::decode(&blob)?;

                    // Apply all account updates to trie
                    let account_updates = state_diff.to_account_updates(&new_trie)?;
                    new_trie = store
                        .apply_account_updates_from_trie(new_trie, account_updates.values())
                        .await
                        .expect("Error applying account updates");

                    // Get withdrawal hashes
                    let withdrawal_hashes = state_diff
                        .withdrawal_logs
                        .iter()
                        .map(|w| {
                            keccak_hash::keccak(
                                [
                                    w.address.as_bytes(),
                                    &w.amount.to_big_endian(),
                                    w.tx_hash.as_bytes(),
                                ]
                                .concat(),
                            )
                        })
                        .collect();

                    // Get the first block of the batch
                    let first_block_number = last_block_number + 1;

                    // Build the header of the last block.
                    // Note that its state_root is the root of new_trie.
                    let new_block = BlockHeader {
                        coinbase,
                        state_root: new_trie.hash().expect("Error committing state"),
                        ..state_diff.last_header
                    };

                    // Store last block.
                    let new_block_hash = new_block.hash();
                    store
                        .add_block_header(new_block_hash, new_block.clone())
                        .await?;
                    store
                        .add_block_number(new_block_hash, state_diff.last_header.number)
                        .await?;
                    store
                        .set_canonical_block(state_diff.last_header.number, new_block_hash)
                        .await?;
                    println!(
                        "Stored last block of blob. Block {}. State root {}",
                        new_block.number, new_block.state_root
                    );

                    last_block_number = new_block.number;

                    let batch = Batch {
                        number: batch_number,
                        first_block: first_block_number,
                        last_block: new_block.number,
                        state_root: new_block.state_root,
                        deposit_logs_hash: H256::zero(),
                        withdrawal_hashes,
                        blobs_bundle: BlobsBundle::empty(),
                    };

                    // Store batch info in L2 storage
                    rollup_store
                        .store_batch(batch)
                        .await
                        .expect("Error storing batch");
                }
                store.update_latest_block_number(last_block_number).await?;
            }
        }
        Ok(())
    }
}

fn deploy_l1(
    l1_rpc_url: &str,
    deployer_private_key: &SecretKey,
    contracts_path: &PathBuf,
) -> eyre::Result<()> {
    // Run 'which solc' to get the path of the solc binary
    let solc_path_output = std::process::Command::new("which").arg("solc").output()?;

    let solc_path = String::from_utf8_lossy(&solc_path_output.stdout)
        .trim()
        .to_string();

    let cmd = std::process::Command::new("forge")
        .current_dir(contracts_path)
        .arg("script")
        .arg("script/DeployL1.s.sol:DeployL1Script")
        .arg("--rpc-url")
        .arg(l1_rpc_url)
        .arg("--private-key")
        .arg(hex::encode(deployer_private_key.secret_bytes())) // TODO: In the future this must be the proposer's private key.
        .arg("--broadcast")
        .arg("--use")
        .arg(solc_path)
        .spawn()?
        .wait()?;
    if !cmd.success() {
        eyre::bail!("Failed to run L1 deployer script");
    }
    Ok(())
}

fn shutdown_l1(ethrex_dev_path: &Path) -> eyre::Result<()> {
    let local_l1_docker_compose_path = ethrex_dev_path.join("docker-compose-dev.yaml");
    let cmd = std::process::Command::new("docker")
        .arg("compose")
        .arg("-f")
        .arg(local_l1_docker_compose_path)
        .arg("down")
        .current_dir(ethrex_dev_path)
        .spawn()?
        .wait()?;
    if !cmd.success() {
        eyre::bail!("Failed to shutdown L1");
    }
    Ok(())
}

fn shutdown_l2() -> eyre::Result<()> {
    std::process::Command::new("pkill")
        .arg("-f")
        .arg("ethrex")
        .spawn()?
        .wait()?;
    Ok(())
}

async fn start_l1(l2_crate_path: &Path, ethrex_dev_path: &Path) -> eyre::Result<()> {
    create_volumes(l2_crate_path)?;
    docker_compose_l2_up(ethrex_dev_path)?;
    Ok(())
}

fn create_volumes(l2_crate_path: &Path) -> eyre::Result<()> {
    let volumes_path = l2_crate_path.join("volumes/reth/data");
    std::fs::create_dir_all(volumes_path)?;
    Ok(())
}

fn docker_compose_l2_up(ethrex_dev_path: &Path) -> eyre::Result<()> {
    let local_l1_docker_compose_path = ethrex_dev_path.join("docker-compose-dev.yaml");
    let cmd = std::process::Command::new("docker")
        .arg("compose")
        .arg("-f")
        .arg(local_l1_docker_compose_path)
        .arg("up")
        .arg("-d")
        .current_dir(ethrex_dev_path)
        .spawn()?
        .wait()?;
    if !cmd.success() {
        eyre::bail!("Failed to run local L1");
    }
    Ok(())
}

// The cli is not displaying tracing logs.
async fn start_l2(root: PathBuf, l2_rpc_url: &str, start_prover: bool) -> eyre::Result<()> {
    let l2_genesis_file_path = root.join("test_data/genesis-l2.json");
    let l2_rpc_url_owned = l2_rpc_url.to_owned();
    let root_clone = root.clone();
    let l2_start_cmd = std::thread::spawn(move || {
        let status = std::process::Command::new("cargo")
            .arg("run")
            .arg("--release")
            .arg("--bin")
            .arg("ethrex")
            .arg("--features")
            .arg("l2")
            .arg("--")
            .arg("--network")
            .arg(l2_genesis_file_path)
            .arg("--http.port")
            .arg(l2_rpc_url_owned.split(':').last().unwrap())
            .current_dir(root)
            .status();

        match status {
            Ok(s) if s.success() => Ok(()),
            Ok(_) => Err(eyre::eyre!("Failed to run L2 node")),
            Err(e) => Err(eyre::eyre!(e)),
        }
    });

    let l2_result = l2_start_cmd.join().expect("L2 thread panicked");
    l2_result?;

    if start_prover {
        let prover_start_cmd = std::thread::spawn(|| {
            let status = std::process::Command::new("cargo")
                .arg("run")
                .arg("--release")
                .arg("--features")
                .arg("build_risc0")
                .arg("--bin")
                .arg("ethrex_prover")
                .current_dir(root_clone)
                .status();

            match status {
                Ok(s) if s.success() => Ok(()),
                Ok(_) => Err(eyre::eyre!("Failed to Initialize Prover")),
                Err(e) => Err(eyre::eyre!(e)),
            }
        });
        let prover_result = prover_start_cmd.join().expect("Prover thread panicked");
        prover_result?;
    }

    Ok(())
}
