use bytes::Bytes;
use colored::Colorize;
use ethereum_rust_core::types::{TxKind, GAS_LIMIT_ADJUSTMENT_FACTOR, GAS_LIMIT_MINIMUM};
use ethereum_rust_l2::utils::{
    config::read_env_file,
    eth_client::{eth_sender::Overrides, EthClient},
};
use ethereum_types::{Address, H160, H256};
use keccak_hash::keccak;
use libsecp256k1::SecretKey;
use spinoff::{spinner, spinners, Color, Spinner};
use std::{process::Command, str::FromStr};

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
    let (deployer, deployer_private_key, eth_client) = setup();
    download_contract_deps();
    compile_contracts();
    let (on_chain_proposer, bridge_address) =
        deploy_contracts(deployer, deployer_private_key, &eth_client).await;
    initialize_contracts(
        deployer,
        deployer_private_key,
        on_chain_proposer,
        bridge_address,
        &eth_client,
    )
    .await;
}

fn setup() -> (Address, SecretKey, EthClient) {
    read_env_file().expect("Failed to read .env file");
    let eth_client = EthClient::new(&std::env::var("ETH_RPC_URL").expect("ETH_RPC_URL not set"));
    let deployer = std::env::var("DEPLOYER_ADDRESS")
        .expect("DEPLOYER_ADDRESS not set")
        .parse()
        .expect("Malformed DEPLOYER_ADDRESS");
    let deployer_private_key = SecretKey::parse(
        H256::from_str(
            std::env::var("DEPLOYER_PRIVATE_KEY")
                .expect("DEPLOYER_PRIVATE_KEY not set")
                .strip_prefix("0x")
                .expect("Malformed DEPLOYER_ADDRESS (strip_prefix(\"0x\"))"),
        )
        .expect("Malformed DEPLOYER_ADDRESS (H256::from_str)")
        .as_fixed_bytes(),
    )
    .expect("Malformed DEPLOYER_PRIVATE_KEY (SecretKey::parse)");

    // If not set, randomize the SALT
    let input = std::env::var("DEPLOYER_SALT_IS_ZERO").unwrap_or("false".to_owned());
    match input.trim().to_lowercase().as_str() {
        "true" | "1" => (),
        "false" | "0" => {
            let mut salt = SALT.lock().unwrap();
            *salt = H256::random();
            println!("SALT: {salt}");
        }
        _ => panic!("{}", format!("Invalid boolean string: {input}")),
    };

    (deployer, deployer_private_key, eth_client)
}

fn download_contract_deps() {
    std::fs::create_dir_all("contracts/lib").expect("Failed to create contracts/lib");
    Command::new("git")
        .arg("clone")
        .arg("https://github.com/OpenZeppelin/openzeppelin-contracts.git")
        .arg("contracts/lib/openzeppelin-contracts")
        .spawn()
        .expect("Failed to spawn git")
        .wait()
        .expect("Failed to wait for git");
}

fn compile_contracts() {
    // Both the contract path and the output path are relative to where the Makefile is.
    assert!(
        Command::new("solc")
            .arg("--bin")
            .arg("./contracts/src/l1/OnChainProposer.sol")
            .arg("-o")
            .arg("contracts/solc_out")
            .arg("--overwrite")
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
            .arg("./contracts/src/l1/CommonBridge.sol")
            .arg("-o")
            .arg("contracts/solc_out")
            .arg("--overwrite")
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
) -> (Address, Address) {
    let overrides = Overrides {
        gas_limit: Some(GAS_LIMIT_MINIMUM * GAS_LIMIT_ADJUSTMENT_FACTOR),
        gas_price: Some(1_000_000_000),
        ..Default::default()
    };

    let deploy_frames = spinner!(["📭❱❱", "❱📬❱", "❱❱📫"], 220);

    let mut spinner = Spinner::new(
        deploy_frames.clone(),
        "Deploying OnChainProposer",
        Color::Cyan,
    );

    let (on_chain_proposer_deployment_tx_hash, on_chain_proposer_address) =
        deploy_on_chain_proposer(
            deployer,
            deployer_private_key,
            overrides.clone(),
            eth_client,
        )
        .await;

    let msg = format!(
        "OnChainProposer:\n\tDeployed at address {} with tx hash {}",
        format!("{on_chain_proposer_address:#x}").bright_green(),
        format!("{on_chain_proposer_deployment_tx_hash:#x}").bright_cyan()
    );
    spinner.success(&msg);

    let mut spinner = Spinner::new(deploy_frames, "Deploying CommonBridge", Color::Cyan);
    let (bridge_deployment_tx_hash, bridge_address) =
        deploy_bridge(deployer, deployer_private_key, overrides, eth_client).await;

    let msg = format!(
        "CommonBridge:\n\tDeployed at address {} with tx hash {}",
        format!("{:#x}", bridge_address).bright_green(),
        format!("{:#x}", bridge_deployment_tx_hash).bright_cyan(),
    );
    spinner.success(&msg);

    (on_chain_proposer_address, bridge_address)
}

async fn deploy_on_chain_proposer(
    deployer: Address,
    deployer_private_key: SecretKey,
    overrides: Overrides,
    eth_client: &EthClient,
) -> (H256, Address) {
    let on_chain_proposer_init_code = hex::decode(
        std::fs::read_to_string("./contracts/solc_out/OnChainProposer.bin")
            .expect("Failed to read on_chain_proposer_init_code"),
    )
    .expect("Failed to decode on_chain_proposer_init_code")
    .into();

    let (deploy_tx_hash, on_chain_proposer) = create2_deploy(
        deployer,
        deployer_private_key,
        &on_chain_proposer_init_code,
        overrides,
        eth_client,
    )
    .await;

    (deploy_tx_hash, on_chain_proposer)
}

async fn deploy_bridge(
    deployer: Address,
    deployer_private_key: SecretKey,
    overrides: Overrides,
    eth_client: &EthClient,
) -> (H256, Address) {
    let mut bridge_init_code = hex::decode(
        std::fs::read_to_string("./contracts/solc_out/CommonBridge.bin")
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
        overrides,
        eth_client,
    )
    .await;

    (deploy_tx_hash, bridge_address)
}

async fn create2_deploy(
    deployer: Address,
    deployer_private_key: SecretKey,
    init_code: &Bytes,
    overrides: Overrides,
    eth_client: &EthClient,
) -> (H256, Address) {
    let calldata = [SALT.lock().unwrap().as_bytes(), init_code].concat();
    let deploy_tx_hash = eth_client
        .send(
            calldata.into(),
            deployer,
            TxKind::Call(DETERMINISTIC_CREATE2_ADDRESS),
            deployer_private_key,
            overrides,
        )
        .await
        .unwrap();

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

async fn initialize_contracts(
    deployer: Address,
    deployer_private_key: SecretKey,
    on_chain_proposer: Address,
    bridge: Address,
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
        deployer,
        deployer_private_key,
        eth_client,
    )
    .await;
    let msg = format!(
        "OnChainProposer:\n\tInitialized with tx hash {}",
        format!("{:#x}", initialize_tx_hash).bright_cyan()
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
        format!("{:#x}", initialize_tx_hash).bright_cyan()
    );
    spinner.success(&msg);
}

async fn initialize_on_chain_proposer(
    on_chain_proposer: Address,
    bridge: Address,
    deployer: Address,
    deployer_private_key: SecretKey,
    eth_client: &EthClient,
) -> H256 {
    let on_chain_proposer_initialize_selector = keccak(b"initialize(address)")
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

    let mut on_chain_proposer_initialization_calldata = Vec::new();
    on_chain_proposer_initialization_calldata
        .extend_from_slice(&on_chain_proposer_initialize_selector);
    on_chain_proposer_initialization_calldata.extend_from_slice(&encoded_bridge);

    let initialize_tx_hash = eth_client
        .send(
            on_chain_proposer_initialization_calldata.into(),
            deployer,
            TxKind::Call(on_chain_proposer),
            deployer_private_key,
            Overrides::default(),
        )
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

    let initialize_tx_hash = eth_client
        .send(
            bridge_initialization_calldata.into(),
            deployer,
            TxKind::Call(bridge),
            deployer_private_key,
            Overrides::default(),
        )
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
    use std::env;

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

        download_contract_deps();
        compile_contracts();

        std::fs::remove_dir_all(solc_out).unwrap();
        std::fs::remove_dir_all(lib).unwrap();
    }
}
