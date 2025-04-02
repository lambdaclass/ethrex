use crate::{config::EthrexL2Config, utils::config::confirm};
use clap::Subcommand;
use ethrex_common::{
    types::{bytes_from_blob, BlockHeader, BYTES_PER_BLOB},
    Address, U256,
};
use ethrex_l2::sequencer::state_diff::StateDiff;
use ethrex_rpc::{
    clients::{beacon::BeaconClient, eth::BlockByNumber},
    EthClient,
};
use ethrex_storage::{EngineType, Store};
use eyre::{ContextCompat, OptionExt};
use itertools::Itertools;
use keccak_hash::keccak;
use reqwest::Url;
use secp256k1::SecretKey;
use std::{
    fs::{create_dir_all, read_dir},
    path::{Path, PathBuf},
    time::Duration,
};

pub const CARGO_MANIFEST_DIR: &str = env!("CARGO_MANIFEST_DIR");

#[allow(clippy::large_enum_variant)]
#[derive(Subcommand)]
pub(crate) enum Command {
    #[clap(
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
    #[clap(about = "Shutdown the stack.")]
    Shutdown {
        #[arg(long, help = "Shuts down the local L1 node.", default_value_t = true)]
        l1: bool,
        #[arg(long, help = "Shuts down the L2 node.", default_value_t = true)]
        l2: bool,
        #[arg(short = 'y', long, help = "Forces the shutdown without confirmation.")]
        force: bool,
    },
    #[clap(about = "Starts the stack.")]
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
    #[clap(about = "Cleans up the stack. Prompts for confirmation.")]
    Purge {
        #[arg(short = 'y', long, help = "Forces the purge without confirmation.")]
        force: bool,
    },
    #[clap(
        about = "Re-initializes the stack. Prompts for confirmation.",
        long_about = "Re-initializing a stack means to shutdown, cleanup, and initialize the stack again. It uses the `shutdown` and `cleanup` commands under the hood."
    )]
    Restart {
        #[arg(short = 'y', long, help = "Forces the restart without confirmation.")]
        force: bool,
    },
    #[clap(about = "Launch a server that listens for Blobs submissions and saves them offline.")]
    BlobsSaver {
        #[clap(
            short = 'c',
            long = "contract",
            help = "The contract address to listen to."
        )]
        contract_address: Address,
        #[arg(short = 'd', long, help = "The directory to save the blobs.")]
        data_dir: PathBuf,
        #[arg(short = 'e', long)]
        l1_eth_rpc: Url,
        #[arg(short = 'b', long)]
        l1_beacon_rpc: Url,
    },
    #[clap(about = "Reconstructs the L2 state from L1 blobs.")]
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
            Command::BlobsSaver {
                l1_eth_rpc,
                l1_beacon_rpc,
                contract_address,
                data_dir,
            } => {
                create_dir_all(data_dir.clone())?;

                let eth_client = EthClient::new(l1_eth_rpc.as_str());
                let beacon_client = BeaconClient::new(l1_beacon_rpc);

                // Keep delay for finality
                let mut current_block = U256::zero();
                while current_block < U256::from(64) {
                    current_block = eth_client.get_block_number().await?;
                    tokio::time::sleep(Duration::from_secs(12)).await;
                }
                current_block = current_block
                    .checked_sub(U256::from(64))
                    .ok_or_eyre("Cannot get finalized block")?;

                let event_signature = keccak("BlockCommitted(bytes32)");

                loop {
                    // Wait for a block
                    tokio::time::sleep(Duration::from_secs(12)).await;

                    let logs = eth_client
                        .get_logs(
                            current_block,
                            current_block,
                            contract_address,
                            event_signature,
                        )
                        .await?;

                    if !logs.is_empty() {
                        // Get parent beacon block root hash from block
                        let block = eth_client
                            .get_block_by_number(BlockByNumber::Number(current_block.as_u64()))
                            .await?;
                        let parent_beacon_hash = block
                            .header
                            .parent_beacon_block_root
                            .ok_or_eyre("Unknown parent beacon root")?;

                        // Get block slot from parent beacon block
                        let parent_beacon_block =
                            beacon_client.get_block_by_hash(parent_beacon_hash).await?;
                        let target_slot = parent_beacon_block.message.slot + 1;

                        // Get versioned hashes from transactions
                        let mut l2_blob_hashes = vec![];
                        for log in logs {
                            let tx = eth_client
                                .get_transaction_by_hash(log.transaction_hash)
                                .await?
                                .ok_or_eyre(format!(
                                    "Transaction {:#x} not found",
                                    log.transaction_hash
                                ))?;
                            l2_blob_hashes.extend(tx.blob_versioned_hashes.ok_or_eyre(format!(
                                "Blobs not found in transaction {:#x}",
                                log.transaction_hash
                            ))?);
                        }

                        // Get blobs from block's slot and only keep L2 commitment's blobs
                        for blob in beacon_client
                            .get_blobs_by_slot(target_slot)
                            .await?
                            .into_iter()
                            .filter(|blob| l2_blob_hashes.contains(&blob.versioned_hash()))
                        {
                            let blob_path =
                                data_dir.join(format!("{}-{}.blob", target_slot, blob.index));
                            std::fs::write(blob_path, blob.blob)?;
                        }

                        println!("Saved blobs for slot {}", target_slot);
                    }

                    current_block += U256::one();
                }
            }
            Command::Reconstruct {
                genesis,
                blobs_dir,
                store_path,
                coinbase,
            } => {
                let store = Store::new_from_genesis(
                    store_path.to_str().expect("Invalid store path"),
                    EngineType::Libmdbx,
                    genesis.to_str().expect("Invalid genesis path"),
                )?;

                let genesis_header = store.get_block_header(0)?.expect("Genesis block not found");
                let genesis_block_hash = genesis_header.compute_block_hash();

                let mut new_trie = store
                    .state_trie(genesis_block_hash)?
                    .expect("Cannot open state trie");
                let mut last_number = 0;
                let mut last_hash = genesis_block_hash;

                let files: Vec<std::fs::DirEntry> = read_dir(blobs_dir)?.try_collect()?;
                for file in files.into_iter().sorted_by_key(|f| f.file_name()) {
                    let blob = std::fs::read(file.path())?;

                    if blob.len() != BYTES_PER_BLOB {
                        panic!("Invalid blob size");
                    }

                    let blob = bytes_from_blob(blob.into());
                    let state_diff = StateDiff::decode(&blob)?;
                    let account_updates = state_diff.to_account_updates(&new_trie)?;

                    new_trie = store
                        .apply_account_updates_from_trie(new_trie, &account_updates)
                        .expect("Error applying account updates");

                    let new_block = BlockHeader {
                        coinbase,
                        number: last_number + 1,
                        parent_hash: last_hash,
                        state_root: new_trie.hash().expect("Error committing state"),
                        ..state_diff.header
                    };
                    let new_block_hash = new_block.compute_block_hash();

                    store.add_block_header(new_block_hash, new_block)?;
                    store.add_block_number(new_block_hash, last_number + 1)?;
                    store.set_canonical_block(last_number + 1, new_block_hash)?;

                    last_number += 1;
                    last_hash = new_block_hash;
                }

                store.update_latest_block_number(last_number)?;
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
