use crate::{config::EthereumRustL2Config, utils::config::confirm};
use clap::Subcommand;
use eyre::ContextCompat;
use secp256k1::SecretKey;
use std::path::{Path, PathBuf};

pub const CARGO_MANIFEST_DIR: &str = env!("CARGO_MANIFEST_DIR");

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
        #[clap(long, help = "Shuts down the local L1 node.", default_value_t = true)]
        l1: bool,
        #[clap(long, help = "Shuts down the L2 node.", default_value_t = true)]
        l2: bool,
        #[clap(short = 'y', long, help = "Forces the shutdown without confirmation.")]
        force: bool,
    },
    #[clap(about = "Starts the stack.")]
    Start {
        #[clap(long, help = "Starts a local L1 node.", required = false)]
        l1: bool,
        #[clap(long, help = "Starts the L2 node.", required = false)]
        l2: bool,
        #[clap(short = 'y', long, help = "Forces the start without confirmation.")]
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
        #[clap(short = 'y', long, help = "Forces the purge without confirmation.")]
        force: bool,
    },
    #[clap(
        about = "Re-initializes the stack. Prompts for confirmation.",
        long_about = "Re-initializing a stack means to shutdown, cleanup, and initialize the stack again. It uses the `shutdown` and `cleanup` commands under the hood."
    )]
    Restart {
        #[clap(short = 'y', long, help = "Forces the restart without confirmation.")]
        force: bool,
    },
}

impl Command {
    pub async fn run(self, cfg: EthereumRustL2Config) -> eyre::Result<()> {
        let root = std::path::Path::new(CARGO_MANIFEST_DIR)
            .parent()
            .map(std::path::Path::parent)
            .context("Failed to get parent")?
            .context("Failed to get grandparent")?;
        let ethereum_rust_dev_path = root.join("crates/blockchain/dev");
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
                    start_l1(&l2_crate_path, &ethereum_rust_dev_path).await?;
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
                    shutdown_l1(&ethereum_rust_dev_path)?;
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
                    start_l1(&l2_crate_path, &ethereum_rust_dev_path).await?;
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

fn shutdown_l1(ethereum_rust_dev_path: &Path) -> eyre::Result<()> {
    let local_l1_docker_compose_path = ethereum_rust_dev_path.join("docker-compose-dev.yaml");
    let cmd = std::process::Command::new("docker")
        .arg("compose")
        .arg("-f")
        .arg(local_l1_docker_compose_path)
        .arg("down")
        .current_dir(ethereum_rust_dev_path)
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
        .arg("ethereum_rust")
        .spawn()?
        .wait()?;
    Ok(())
}

async fn start_l1(l2_crate_path: &Path, ethereum_rust_dev_path: &Path) -> eyre::Result<()> {
    create_volumes(l2_crate_path)?;
    docker_compose_l2_up(ethereum_rust_dev_path)?;
    Ok(())
}

fn create_volumes(l2_crate_path: &Path) -> eyre::Result<()> {
    let volumes_path = l2_crate_path.join("volumes/reth/data");
    std::fs::create_dir_all(volumes_path)?;
    Ok(())
}

fn docker_compose_l2_up(ethereum_rust_dev_path: &Path) -> eyre::Result<()> {
    let local_l1_docker_compose_path = ethereum_rust_dev_path.join("docker-compose-dev.yaml");
    let cmd = std::process::Command::new("docker")
        .arg("compose")
        .arg("-f")
        .arg(local_l1_docker_compose_path)
        .arg("up")
        .arg("-d")
        .current_dir(ethereum_rust_dev_path)
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
            .arg("ethereum_rust")
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
                .arg("build_zkvm")
                .arg("--bin")
                .arg("ethereum_rust_prover")
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
