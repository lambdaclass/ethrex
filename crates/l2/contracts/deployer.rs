use bytes::Bytes;
use colored::Colorize;
use ethereum_types::{Address, H160, H256};
use ethrex_core::U256;
use ethrex_l2::utils::eth_client::errors::EthClientError;
use ethrex_l2::utils::{
    config::{read_env_as_lines, read_env_file, write_env},
    eth_client::{eth_sender::Overrides, EthClient},
};
use keccak_hash::keccak;
use secp256k1::SecretKey;
use spinoff::{spinner, spinners, Color, Spinner};
use std::{
    path::{Path, PathBuf},
    process::Command,
    str::FromStr,
};
use tracing::warn;

struct SetupResult {
    deployer_address: Address,
    deployer_private_key: SecretKey,
    committer_address: Address,
    verifier_address: Address,
    contract_verifier_address: Address,
    eth_client: EthClient,
    contracts_path: PathBuf,
}

#[derive(Debug, thiserror::Error)]
pub enum DeployError {
    #[error("Failed to lock SALT: {0}")]
    FailedToLockSALT(String),
    #[error("The path is not a valid utf-8 string")]
    FailedToGetStringFromPath,
    #[error("Deployer setup error: {0} not set")]
    ConfigValueNotSet(String),
    #[error("Deployer setup parse error: {0}")]
    ParseError(String),
    #[error("Deployer dependency error: {0}")]
    DependencyError(String),
    #[error("Deployer compilation error: {0}")]
    CompilationError(String),
    #[error("Deployer EthClient error: {0}")]
    EthClientError(#[from] EthClientError),
    #[error("Deployer decoding error: {0}")]
    DecodingError(String),
}

// 0x4e59b44847b379578588920cA78FbF26c0B4956C
const DETERMINISTIC_CREATE2_ADDRESS: Address = H160([
    0x4e, 0x59, 0xb4, 0x48, 0x47, 0xb3, 0x79, 0x57, 0x85, 0x88, 0x92, 0x0c, 0xa7, 0x8f, 0xbf, 0x26,
    0xc0, 0xb4, 0x95, 0x6c,
]);

lazy_static::lazy_static! {
    static ref SALT: std::sync::Mutex<H256> = std::sync::Mutex::new(H256::zero());
}

#[tokio::main]
async fn main() {
    let Ok(setup_result) = setup() else {
        panic!("Failed on setup");
    };
    if let Err(e) = download_contract_deps(&setup_result.contracts_path) {
        panic!("Failed to download contracts {e}");
    };
    if let Err(e) = compile_contracts(&setup_result.contracts_path) {
        panic!("Failed to compile contracts {e}");
    };

    let on_chain_proposer;
    let bridge_address;
    match deploy_contracts(
        setup_result.deployer_address,
        setup_result.deployer_private_key,
        &setup_result.eth_client,
        &setup_result.contracts_path,
    )
    .await
    {
        Ok((ocp, ba)) => {
            on_chain_proposer = ocp;
            bridge_address = ba;
        }
        Err(e) => panic!("Failed to deploy contracts {e}"),
    };

    if let Err(err) = initialize_contracts(
        setup_result.deployer_address,
        setup_result.deployer_private_key,
        setup_result.committer_address,
        setup_result.verifier_address,
        on_chain_proposer,
        bridge_address,
        setup_result.contract_verifier_address,
        &setup_result.eth_client,
    )
    .await
    {
        panic!("Failed to initialize contracts: {err}");
    }

    let Ok(env_lines) = read_env_as_lines() else {
        panic!("Failed to read env file as lines.");
    };

    let mut wr_lines: Vec<String> = Vec::new();
    let mut env_lines_iter = env_lines.into_iter();
    while let Some(Ok(mut line)) = env_lines_iter.next() {
        if let Some(eq) = line.find('=') {
            let (envar, _) = line.split_at(eq);
            line = match envar {
                "COMMITTER_ON_CHAIN_PROPOSER_ADDRESS" => {
                    format!("{envar}={on_chain_proposer:#x}")
                }
                "L1_WATCHER_BRIDGE_ADDRESS" => {
                    format!("{envar}={bridge_address:#x}")
                }
                _ => line,
            };
        }
        wr_lines.push(line);
    }
    if let Err(err) = write_env(wr_lines) {
        panic!(
            "{}",
            format!("Failed to write changes to the .env file: {err}")
        );
    }
}

fn setup() -> Result<SetupResult, DeployError> {
    if let Err(e) = read_env_file() {
        warn!("Failed to read .env file: {e}");
    }

    let eth_client = EthClient::new(&read_env_var("ETH_RPC_URL")?);

    let deployer_address = parse_env_var("DEPLOYER_ADDRESS")?;
    let deployer_private_key = SecretKey::from_slice(
        H256::from_str(
            read_env_var("DEPLOYER_PRIVATE_KEY")?
                .strip_prefix("0x")
                .ok_or(DeployError::ParseError(
                    "Malformed DEPLOYER PRIVATE KEY (strip_prefix(\"0x\"))".to_owned(),
                ))?,
        )
        .map_err(|err| {
            DeployError::ParseError(format!(
                "Malformed DEPLOYER PRIVATE KEY (H256::from_str): {err}"
            ))
        })?
        .as_bytes(),
    )
    .map_err(|err| {
        DeployError::ParseError(format!(
            "Malformed DEPLOYER_PRIVATE_KEY (SecretKey::parse): {err}"
        ))
    })?;

    let committer_address = parse_env_var("COMMITTER_L1_ADDRESS")?;

    let verifier_address = parse_env_var("PROVER_SERVER_VERIFIER_ADDRESS")?;

    let contracts_path = Path::new(
        std::env::var("DEPLOYER_CONTRACTS_PATH")
            .unwrap_or(".".to_string())
            .as_str(),
    )
    .to_path_buf();

    // If not set, randomize the SALT
    let input = std::env::var("DEPLOYER_SALT_IS_ZERO").unwrap_or("false".to_owned());
    match input.trim().to_lowercase().as_str() {
        "true" | "1" => (),
        "false" | "0" => {
            let mut salt = SALT
                .lock()
                .map_err(|err| DeployError::FailedToLockSALT(err.to_string()))?;
            *salt = H256::random();
        }
        _ => {
            return Err(DeployError::ParseError(format!(
                "Invalid boolean string: {input}"
            )));
        }
    };
    let contract_verifier_address = parse_env_var("DEPLOYER_CONTRACT_VERIFIER")?;
    Ok(SetupResult {
        deployer_address,
        deployer_private_key,
        committer_address,
        verifier_address,
        contract_verifier_address,
        eth_client,
        contracts_path,
    })
}

fn read_env_var(key: &str) -> Result<String, DeployError> {
    std::env::var(key).map_err(|_| DeployError::ConfigValueNotSet(key.to_owned()))
}

fn parse_env_var(key: &str) -> Result<Address, DeployError> {
    read_env_var(key)?
        .parse()
        .map_err(|err| DeployError::ParseError(format!("Malformed {key}: {err}")))
}

fn download_contract_deps(contracts_path: &Path) -> Result<(), DeployError> {
    std::fs::create_dir_all(contracts_path.join("lib")).map_err(|err| {
        DeployError::DependencyError(format!("Failed to create contracts/lib: {err}"))
    })?;
    Command::new("git")
        .arg("clone")
        .arg("https://github.com/OpenZeppelin/openzeppelin-contracts.git")
        .arg(
            contracts_path
                .join("lib/openzeppelin-contracts")
                .to_str()
                .ok_or(DeployError::FailedToGetStringFromPath)?,
        )
        .spawn()
        .map_err(|err| DeployError::DependencyError(format!("Failed to spawn git: {err}")))?
        .wait()
        .map_err(|err| DeployError::DependencyError(format!("Failed to wait for git: {err}")))?;
    Ok(())
}

fn compile_contracts(contracts_path: &Path) -> Result<(), DeployError> {
    // Both the contract path and the output path are relative to where the Makefile is.
    if !Command::new("solc")
        .arg("--bin")
        .arg(
            contracts_path
                .join("src/l1/OnChainProposer.sol")
                .to_str()
                .ok_or(DeployError::FailedToGetStringFromPath)?,
        )
        .arg("-o")
        .arg(
            contracts_path
                .join("solc_out")
                .to_str()
                .ok_or(DeployError::FailedToGetStringFromPath)?,
        )
        .arg("--overwrite")
        .arg("--allow-paths")
        .arg(
            contracts_path
                .to_str()
                .ok_or(DeployError::FailedToGetStringFromPath)?,
        )
        .spawn()
        .map_err(|err| DeployError::CompilationError(format!("Failed to spawn solc: {err}")))?
        .wait()
        .map_err(|err| DeployError::CompilationError(format!("Failed to wait for solc: {err}")))?
        .success()
    {
        return Err(DeployError::CompilationError(
            "Failed to compile OnChainProposer.sol".to_owned(),
        ));
    }

    if !Command::new("solc")
        .arg("--bin")
        .arg(
            contracts_path
                .join("src/l1/CommonBridge.sol")
                .to_str()
                .ok_or(DeployError::FailedToGetStringFromPath)?,
        )
        .arg("-o")
        .arg(
            contracts_path
                .join("solc_out")
                .to_str()
                .ok_or(DeployError::FailedToGetStringFromPath)?,
        )
        .arg("--overwrite")
        .arg("--allow-paths")
        .arg(
            contracts_path
                .to_str()
                .ok_or(DeployError::FailedToGetStringFromPath)?,
        )
        .spawn()
        .map_err(|err| DeployError::CompilationError(format!("Failed to spawn solc: {err}")))?
        .wait()
        .map_err(|err| DeployError::CompilationError(format!("Failed to wait for solc: {err}")))?
        .success()
    {
        return Err(DeployError::CompilationError(
            "Failed to compile CommonBridge.sol".to_owned(),
        ));
    }
    Ok(())
}

async fn deploy_contracts(
    deployer: Address,
    deployer_private_key: SecretKey,
    eth_client: &EthClient,
    contracts_path: &Path,
) -> Result<(Address, Address), DeployError> {
    let deploy_frames = spinner!(["📭❱❱", "❱📬❱", "❱❱📫"], 220);

    let mut spinner = Spinner::new(
        deploy_frames.clone(),
        "Deploying OnChainProposer",
        Color::Cyan,
    );

    let (on_chain_proposer_deployment_tx_hash, on_chain_proposer_address) =
        deploy_on_chain_proposer(deployer, deployer_private_key, eth_client, contracts_path)
            .await?;

    let msg = format!(
        "OnChainProposer:\n\tDeployed at address {} with tx hash {}",
        format!("{on_chain_proposer_address:#x}").bright_green(),
        format!("{on_chain_proposer_deployment_tx_hash:#x}").bright_cyan()
    );
    spinner.success(&msg);

    let mut spinner = Spinner::new(deploy_frames, "Deploying CommonBridge", Color::Cyan);
    let (bridge_deployment_tx_hash, bridge_address) =
        deploy_bridge(deployer, deployer_private_key, eth_client, contracts_path).await?;

    let msg = format!(
        "CommonBridge:\n\tDeployed at address {} with tx hash {}",
        format!("{bridge_address:#x}").bright_green(),
        format!("{bridge_deployment_tx_hash:#x}").bright_cyan(),
    );
    spinner.success(&msg);

    Ok((on_chain_proposer_address, bridge_address))
}

async fn deploy_on_chain_proposer(
    deployer: Address,
    deployer_private_key: SecretKey,
    eth_client: &EthClient,
    contracts_path: &Path,
) -> Result<(H256, Address), DeployError> {
    let on_chain_proposer_init_code = hex::decode(
        std::fs::read_to_string(contracts_path.join("solc_out/OnChainProposer.bin")).map_err(
            |err| {
                DeployError::DecodingError(format!(
                    "Failed to read on_chain_proposer_init_code: {err}"
                ))
            },
        )?,
    )
    .map_err(|err| {
        DeployError::DecodingError(format!(
            "Failed to decode on_chain_proposer_init_code: {err}"
        ))
    })?
    .into();

    let (deploy_tx_hash, on_chain_proposer) = create2_deploy(
        deployer,
        deployer_private_key,
        &on_chain_proposer_init_code,
        eth_client,
    )
    .await
    .map_err(DeployError::from)?;

    Ok((deploy_tx_hash, on_chain_proposer))
}

async fn deploy_bridge(
    deployer: Address,
    deployer_private_key: SecretKey,
    eth_client: &EthClient,
    contracts_path: &Path,
) -> Result<(H256, Address), DeployError> {
    let mut bridge_init_code = hex::decode(
        std::fs::read_to_string(contracts_path.join("solc_out/CommonBridge.bin")).map_err(
            |err| DeployError::DecodingError(format!("Failed to read bridge_init_code: {err}")),
        )?,
    )
    .map_err(|err| {
        DeployError::DecodingError(format!("Failed to decode bridge_init_code: {err}"))
    })?;

    let encoded_owner = {
        let offset = 32 - deployer.as_bytes().len() % 32;
        let mut encoded_owner = vec![0; offset];
        encoded_owner.extend_from_slice(deployer.as_bytes());
        encoded_owner
    };

    bridge_init_code.extend_from_slice(&encoded_owner);

    let (deploy_tx_hash, bridge_address) = create2_deploy(
        deployer,
        deployer_private_key,
        &bridge_init_code.into(),
        eth_client,
    )
    .await?;

    Ok((deploy_tx_hash, bridge_address))
}

async fn create2_deploy(
    deployer: Address,
    deployer_private_key: SecretKey,
    init_code: &Bytes,
    eth_client: &EthClient,
) -> Result<(H256, Address), DeployError> {
    let calldata = [
        SALT.lock()
            .map_err(|err| DeployError::FailedToLockSALT(err.to_string()))?
            .as_bytes(),
        init_code,
    ]
    .concat();
    let deploy_tx = eth_client
        .build_eip1559_transaction(
            DETERMINISTIC_CREATE2_ADDRESS,
            deployer,
            calldata.into(),
            Overrides::default(),
            10,
        )
        .await?;

    let deploy_tx_hash = eth_client
        .send_eip1559_transaction(&deploy_tx, &deployer_private_key)
        .await?;

    wait_for_transaction_receipt(deploy_tx_hash, eth_client)
        .await
        .map_err(DeployError::from)?;

    let deployed_address = create2_address(keccak(init_code))?;

    Ok((deploy_tx_hash, deployed_address))
}

fn create2_address(init_code_hash: H256) -> Result<Address, DeployError> {
    let addr = Address::from_slice(
        keccak(
            [
                &[0xff],
                DETERMINISTIC_CREATE2_ADDRESS.as_bytes(),
                SALT.lock()
                    .map_err(|err| DeployError::FailedToLockSALT(err.to_string()))?
                    .as_bytes(),
                init_code_hash.as_bytes(),
            ]
            .concat(),
        )
        .as_bytes()
        .get(12..)
        .ok_or(DeployError::DecodingError(
            "Failed to get create2 address".to_owned(),
        ))?,
    );
    Ok(addr)
}

#[allow(clippy::too_many_arguments)]
async fn initialize_contracts(
    deployer: Address,
    deployer_private_key: SecretKey,
    committer: Address,
    verifier: Address,
    on_chain_proposer: Address,
    bridge: Address,
    contract_verifier_address: Address,
    eth_client: &EthClient,
) -> Result<(), DeployError> {
    let initialize_frames = spinner!(["🪄❱❱", "❱🪄❱", "❱❱🪄"], 200);

    let mut spinner = Spinner::new(
        initialize_frames.clone(),
        "Initilazing OnChainProposer",
        Color::Cyan,
    );

    let initialize_tx_hash = initialize_on_chain_proposer(
        on_chain_proposer,
        bridge,
        contract_verifier_address,
        deployer,
        deployer_private_key,
        committer,
        verifier,
        eth_client,
    )
    .await
    .map_err(DeployError::from)?;
    let msg = format!(
        "OnChainProposer:\n\tInitialized with tx hash {}",
        format!("{initialize_tx_hash:#x}").bright_cyan()
    );
    spinner.success(&msg);

    let mut spinner = Spinner::new(
        initialize_frames.clone(),
        "Initilazing CommonBridge",
        Color::Cyan,
    );
    let initialize_tx_hash = initialize_bridge(
        on_chain_proposer,
        bridge,
        deployer,
        deployer_private_key,
        eth_client,
    )
    .await
    .map_err(DeployError::from)?;
    let msg = format!(
        "CommonBridge:\n\tInitialized with tx hash {}",
        format!("{initialize_tx_hash:#x}").bright_cyan()
    );
    spinner.success(&msg);
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn initialize_on_chain_proposer(
    on_chain_proposer: Address,
    bridge: Address,
    contract_verifier_address: Address,
    deployer: Address,
    deployer_private_key: SecretKey,
    committer: Address,
    verifier: Address,
    eth_client: &EthClient,
) -> Result<H256, DeployError> {
    let on_chain_proposer_initialize_selector = keccak(b"initialize(address,address,address[])")
        .as_bytes()
        .get(..4)
        .ok_or(DeployError::DecodingError(
            "Failed to get initialize selector".to_owned(),
        ))?
        .to_vec();
    let encoded_bridge = {
        let offset = 32 - bridge.as_bytes().len() % 32;
        let mut encoded_bridge = vec![0; offset];
        encoded_bridge.extend_from_slice(bridge.as_bytes());
        encoded_bridge
    };

    let encoded_contract_verifier = {
        let offset = 32 - contract_verifier_address.as_bytes().len() % 32;
        let mut encoded_contract_verifier = vec![0; offset];
        encoded_contract_verifier.extend_from_slice(contract_verifier_address.as_bytes());
        encoded_contract_verifier
    };

    let mut on_chain_proposer_initialization_calldata = Vec::new();
    on_chain_proposer_initialization_calldata
        .extend_from_slice(&on_chain_proposer_initialize_selector);
    on_chain_proposer_initialization_calldata.extend_from_slice(&encoded_bridge);
    on_chain_proposer_initialization_calldata.extend_from_slice(&encoded_contract_verifier);

    let mut encoded_offset = [0; 32];
    U256::from(32 * 3).to_big_endian(&mut encoded_offset);
    on_chain_proposer_initialization_calldata.extend_from_slice(&encoded_offset);
    let mut allowed_addresses = [0; 32];
    U256::from(2).to_big_endian(&mut allowed_addresses);
    on_chain_proposer_initialization_calldata.extend_from_slice(&allowed_addresses);

    let committer_h256: H256 = committer.into();
    let verifier_h256: H256 = verifier.into();
    on_chain_proposer_initialization_calldata.extend_from_slice(committer_h256.as_fixed_bytes());
    on_chain_proposer_initialization_calldata.extend_from_slice(verifier_h256.as_fixed_bytes());

    let initialize_tx = eth_client
        .build_eip1559_transaction(
            on_chain_proposer,
            deployer,
            on_chain_proposer_initialization_calldata.into(),
            Overrides::default(),
            10,
        )
        .await?;
    let initialize_tx_hash = eth_client
        .send_eip1559_transaction(&initialize_tx, &deployer_private_key)
        .await?;

    wait_for_transaction_receipt(initialize_tx_hash, eth_client).await?;

    Ok(initialize_tx_hash)
}

async fn initialize_bridge(
    on_chain_proposer: Address,
    bridge: Address,
    deployer: Address,
    deployer_private_key: SecretKey,
    eth_client: &EthClient,
) -> Result<H256, DeployError> {
    let bridge_initialize_selector = keccak(b"initialize(address)")
        .as_bytes()
        .get(..4)
        .ok_or(DeployError::DecodingError(
            "Failed to get initialize selector".to_owned(),
        ))?
        .to_vec();
    let encoded_on_chain_proposer = {
        let offset = 32 - on_chain_proposer.as_bytes().len() % 32;
        let mut encoded_owner = vec![0; offset];
        encoded_owner.extend_from_slice(on_chain_proposer.as_bytes());
        encoded_owner
    };

    let mut bridge_initialization_calldata = Vec::new();
    bridge_initialization_calldata.extend_from_slice(&bridge_initialize_selector);
    bridge_initialization_calldata.extend_from_slice(&encoded_on_chain_proposer);

    let initialize_tx = eth_client
        .build_eip1559_transaction(
            bridge,
            deployer,
            bridge_initialization_calldata.into(),
            Overrides::default(),
            10,
        )
        .await
        .map_err(DeployError::from)?;
    let initialize_tx_hash = eth_client
        .send_eip1559_transaction(&initialize_tx, &deployer_private_key)
        .await
        .map_err(DeployError::from)?;

    wait_for_transaction_receipt(initialize_tx_hash, eth_client)
        .await
        .map_err(DeployError::from)?;

    Ok(initialize_tx_hash)
}

async fn wait_for_transaction_receipt(
    tx_hash: H256,
    eth_client: &EthClient,
) -> Result<(), EthClientError> {
    while eth_client.get_transaction_receipt(tx_hash).await?.is_none() {
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    }
    Ok(())
}

#[allow(clippy::unwrap_used)]
#[allow(clippy::expect_used)]
#[cfg(test)]
mod test {
    use crate::{compile_contracts, download_contract_deps};
    use std::{env, path::Path};

    #[test]
    fn test_contract_compilation() {
        let binding = env::current_dir().unwrap();
        let parent_dir = binding.parent().unwrap();

        env::set_current_dir(parent_dir).expect("Failed to change directory");

        let solc_out = parent_dir.join("contracts/solc_out");
        let lib = parent_dir.join("contracts/lib");

        if let Err(e) = std::fs::remove_dir_all(&solc_out) {
            if e.kind() != std::io::ErrorKind::NotFound {
                panic!("Failed to remove directory solc_out");
            }
        }
        if let Err(e) = std::fs::remove_dir_all(&lib) {
            if e.kind() != std::io::ErrorKind::NotFound {
                panic!("failed to remove directory lib");
            }
        }

        if download_contract_deps(Path::new("contracts")).is_err() {
            panic!("failed to download contract deps");
        };
        if compile_contracts(Path::new("contracts")).is_err() {
            panic!("failed to compile contracts");
        };

        std::fs::remove_dir_all(solc_out).unwrap();
        std::fs::remove_dir_all(lib).unwrap();
    }
}
