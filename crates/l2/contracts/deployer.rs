use bytes::Bytes;
use colored::Colorize;
use ethereum_types::{Address, H160, H256};
use ethrex_core::U256;
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
    let (
        deployer,
        deployer_private_key,
        committer_private_key,
        verifier_private_key,
        contract_verifier_address,
        eth_client,
        contracts_path,
    ) = setup();
    download_contract_deps(&contracts_path);
    compile_contracts(&contracts_path);
    let (on_chain_proposer, bridge_address) =
        deploy_contracts(deployer, deployer_private_key, &eth_client, &contracts_path).await;

    initialize_contracts(
        deployer,
        deployer_private_key,
        committer_private_key,
        verifier_private_key,
        on_chain_proposer,
        bridge_address,
        contract_verifier_address,
        &eth_client,
    )
    .await;

    let env_lines = read_env_as_lines().expect("Failed to read env file as lines.");

    let mut wr_lines: Vec<String> = Vec::new();
    for line in env_lines {
        let mut line = line.unwrap();
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
    write_env(wr_lines).expect("Failed to write changes to the .env file.");
}

fn setup() -> (
    Address,
    SecretKey,
    Address,
    Address,
    Address,
    EthClient,
    PathBuf,
) {
    if let Err(e) = read_env_file() {
        warn!("Failed to read .env file: {e}");
    }

    let eth_client = EthClient::new(&std::env::var("ETH_RPC_URL").expect("ETH_RPC_URL not set"));
    let deployer = std::env::var("DEPLOYER_ADDRESS")
        .expect("DEPLOYER_ADDRESS not set")
        .parse()
        .expect("Malformed DEPLOYER_ADDRESS");
    let deployer_private_key = SecretKey::from_slice(
        H256::from_str(
            std::env::var("DEPLOYER_PRIVATE_KEY")
                .expect("DEPLOYER_PRIVATE_KEY not set")
                .strip_prefix("0x")
                .expect("Malformed DEPLOYER PRIVATE KEY (strip_prefix(\"0x\"))"),
        )
        .expect("Malformed DEPLOYER_PRIVATE_KEY (H256::from_str)")
        .as_bytes(),
    )
    .expect("Malformed DEPLOYER_PRIVATE_KEY (SecretKey::parse)");

    let committer = std::env::var("COMMITTER_L1_ADDRESS")
        .expect("COMMITTER_L1_ADDRESS not set")
        .parse()
        .expect("Malformed COMMITTER_L1_ADDRESS");
    let verifier = std::env::var("PROVER_SERVER_VERIFIER_ADDRESS")
        .expect("PROVER_SERVER_VERIFIER_ADDRESS not set")
        .parse()
        .expect("Malformed PROVER_SERVER_VERIFIER_ADDRESS");

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
            let mut salt = SALT.lock().unwrap();
            *salt = H256::random();
        }
        _ => panic!("Invalid boolean string: {input}"),
    };
    let contract_verifier_address = std::env::var("DEPLOYER_CONTRACT_VERIFIER")
        .expect("DEPLOYER_CONTRACT_VERIFIER not set")
        .parse()
        .expect("Malformed DEPLOYER_CONTRACT_VERIFIER");
    (
        deployer,
        deployer_private_key,
        committer,
        verifier,
        contract_verifier_address,
        eth_client,
        contracts_path,
    )
}

fn download_contract_deps(contracts_path: &Path) {
    std::fs::create_dir_all(contracts_path.join("lib")).expect("Failed to create contracts/lib");
    Command::new("git")
        .arg("clone")
        .arg("https://github.com/OpenZeppelin/openzeppelin-contracts.git")
        .arg(
            contracts_path
                .join("lib/openzeppelin-contracts")
                .to_str()
                .unwrap(),
        )
        .spawn()
        .expect("Failed to spawn git")
        .wait()
        .expect("Failed to wait for git");
}

fn compile_contracts(contracts_path: &Path) {
    // Both the contract path and the output path are relative to where the Makefile is.
    assert!(
        Command::new("solc")
            .arg("--bin")
            .arg(
                contracts_path
                    .join("src/l1/OnChainProposer.sol")
                    .to_str()
                    .unwrap()
            )
            .arg("-o")
            .arg(contracts_path.join("solc_out").to_str().unwrap())
            .arg("--overwrite")
            .arg("--allow-paths")
            .arg(contracts_path.to_str().unwrap())
            .spawn()
            .expect("Failed to spawn solc")
            .wait()
            .expect("Failed to wait for solc")
            .success(),
        "Failed to compile OnChainProposer.sol"
    );

    assert!(
        Command::new("solc")
            .arg("--bin")
            .arg(
                contracts_path
                    .join("src/l1/CommonBridge.sol")
                    .to_str()
                    .unwrap()
            )
            .arg("-o")
            .arg(contracts_path.join("solc_out").to_str().unwrap())
            .arg("--overwrite")
            .arg("--allow-paths")
            .arg(contracts_path.to_str().unwrap())
            .spawn()
            .expect("Failed to spawn solc")
            .wait()
            .expect("Failed to wait for solc")
            .success(),
        "Failed to compile CommonBridge.sol"
    );
}

async fn deploy_contracts(
    deployer: Address,
    deployer_private_key: SecretKey,
    eth_client: &EthClient,
    contracts_path: &Path,
) -> (Address, Address) {
    let deploy_frames = spinner!(["📭❱❱", "❱📬❱", "❱❱📫"], 220);

    let mut spinner = Spinner::new(
        deploy_frames.clone(),
        "Deploying OnChainProposer",
        Color::Cyan,
    );

    let (on_chain_proposer_deployment_tx_hash, on_chain_proposer_address) =
        deploy_on_chain_proposer(deployer, deployer_private_key, eth_client, contracts_path).await;

    let msg = format!(
        "OnChainProposer:\n\tDeployed at address {} with tx hash {}",
        format!("{on_chain_proposer_address:#x}").bright_green(),
        format!("{on_chain_proposer_deployment_tx_hash:#x}").bright_cyan()
    );
    spinner.success(&msg);

    let mut spinner = Spinner::new(deploy_frames, "Deploying CommonBridge", Color::Cyan);
    let (bridge_deployment_tx_hash, bridge_address) =
        deploy_bridge(deployer, deployer_private_key, eth_client, contracts_path).await;

    let msg = format!(
        "CommonBridge:\n\tDeployed at address {} with tx hash {}",
        format!("{bridge_address:#x}").bright_green(),
        format!("{bridge_deployment_tx_hash:#x}").bright_cyan(),
    );
    spinner.success(&msg);

    (on_chain_proposer_address, bridge_address)
}

async fn deploy_on_chain_proposer(
    deployer: Address,
    deployer_private_key: SecretKey,
    eth_client: &EthClient,
    contracts_path: &Path,
) -> (H256, Address) {
    let on_chain_proposer_init_code = hex::decode(
        std::fs::read_to_string(contracts_path.join("solc_out/OnChainProposer.bin"))
            .expect("Failed to read on_chain_proposer_init_code"),
    )
    .expect("Failed to decode on_chain_proposer_init_code")
    .into();

    let (deploy_tx_hash, on_chain_proposer) = create2_deploy(
        deployer,
        deployer_private_key,
        &on_chain_proposer_init_code,
        eth_client,
    )
    .await;

    (deploy_tx_hash, on_chain_proposer)
}

async fn deploy_bridge(
    deployer: Address,
    deployer_private_key: SecretKey,
    eth_client: &EthClient,
    contracts_path: &Path,
) -> (H256, Address) {
    let mut bridge_init_code = hex::decode(
        std::fs::read_to_string(contracts_path.join("solc_out/CommonBridge.bin"))
            .expect("Failed to read bridge_init_code"),
    )
    .expect("Failed to decode bridge_init_code");

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
    .await;

    (deploy_tx_hash, bridge_address)
}

async fn create2_deploy(
    deployer: Address,
    deployer_private_key: SecretKey,
    init_code: &Bytes,
    eth_client: &EthClient,
) -> (H256, Address) {
    let calldata = [SALT.lock().unwrap().as_bytes(), init_code].concat();
    let deploy_tx = eth_client
        .build_eip1559_transaction(
            DETERMINISTIC_CREATE2_ADDRESS,
            deployer,
            calldata.into(),
            Overrides::default(),
            10,
        )
        .await
        .expect("Failed to build create2 deploy tx");

    let deploy_tx_hash = eth_client
        .send_eip1559_transaction(&deploy_tx, &deployer_private_key)
        .await
        .expect("Failed to send create2 deploy tx");

    wait_for_transaction_receipt(deploy_tx_hash, eth_client).await;

    let deployed_address = create2_address(keccak(init_code));

    (deploy_tx_hash, deployed_address)
}

fn create2_address(init_code_hash: H256) -> Address {
    Address::from_slice(
        keccak(
            [
                &[0xff],
                DETERMINISTIC_CREATE2_ADDRESS.as_bytes(),
                SALT.lock().unwrap().as_bytes(),
                init_code_hash.as_bytes(),
            ]
            .concat(),
        )
        .as_bytes()
        .get(12..)
        .expect("Failed to get create2 address"),
    )
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
) {
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
    .await;
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
    .await;
    let msg = format!(
        "CommonBridge:\n\tInitialized with tx hash {}",
        format!("{initialize_tx_hash:#x}").bright_cyan()
    );
    spinner.success(&msg);
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
) -> H256 {
    let on_chain_proposer_initialize_selector = keccak(b"initialize(address,address,address[])")
        .as_bytes()
        .get(..4)
        .expect("Failed to get initialize selector")
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
        .await
        .expect("Failed to build initialize transaction");
    let initialize_tx_hash = eth_client
        .send_eip1559_transaction(&initialize_tx, &deployer_private_key)
        .await
        .expect("Failed to send initialize transaction");

    wait_for_transaction_receipt(initialize_tx_hash, eth_client).await;

    initialize_tx_hash
}

async fn initialize_bridge(
    on_chain_proposer: Address,
    bridge: Address,
    deployer: Address,
    deployer_private_key: SecretKey,
    eth_client: &EthClient,
) -> H256 {
    let bridge_initialize_selector = keccak(b"initialize(address)")
        .as_bytes()
        .get(..4)
        .expect("Failed to get initialize selector")
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
        .expect("Failed to build initialize transaction");
    let initialize_tx_hash = eth_client
        .send_eip1559_transaction(&initialize_tx, &deployer_private_key)
        .await
        .expect("Failed to send initialize transaction");

    wait_for_transaction_receipt(initialize_tx_hash, eth_client).await;

    initialize_tx_hash
}

async fn wait_for_transaction_receipt(tx_hash: H256, eth_client: &EthClient) {
    while eth_client
        .get_transaction_receipt(tx_hash)
        .await
        .expect("Failed to get transaction receipt")
        .is_none()
    {
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    }
}

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
                panic!();
            }
        }
        if let Err(e) = std::fs::remove_dir_all(&lib) {
            if e.kind() != std::io::ErrorKind::NotFound {
                panic!();
            }
        }

        download_contract_deps(Path::new("contracts"));
        compile_contracts(Path::new("contracts"));

        std::fs::remove_dir_all(solc_out).unwrap();
        std::fs::remove_dir_all(lib).unwrap();
    }
}
