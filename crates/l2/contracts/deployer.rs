use bytes::Bytes;
use ethereum_rust_core::types::{TxKind, GAS_LIMIT_ADJUSTMENT_FACTOR, GAS_LIMIT_MINIMUM};
use ethereum_rust_l2::utils::{
    config::read_env_file,
    eth_client::{eth_sender::Overrides, EthClient},
};
use ethereum_types::{Address, H160, H256};
use keccak_hash::keccak;
use libsecp256k1::SecretKey;
use std::{process::Command, str::FromStr};

// 0x4e59b44847b379578588920cA78FbF26c0B4956C
const DETERMINISTIC_CREATE2_ADDRESS: Address = H160([
    0x4e, 0x59, 0xb4, 0x48, 0x47, 0xb3, 0x79, 0x57, 0x85, 0x88, 0x92, 0x0c, 0xa7, 0x8f, 0xbf, 0x26,
    0xc0, 0xb4, 0x95, 0x6c,
]);
const SALT: H256 = H256::zero();

#[tokio::main]
async fn main() {
    read_env_file().unwrap();

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

    if std::fs::exists("contracts/solc_out").expect("Could not determine if solc_out exists") {
        std::fs::remove_dir_all("contracts/solc_out").expect("Failed to remove solc_out");
    }

    let overrides = Overrides {
        gas_limit: Some(GAS_LIMIT_MINIMUM * GAS_LIMIT_ADJUSTMENT_FACTOR),
        gas_price: Some(1_000_000_000),
        ..Default::default()
    };

    let (on_chain_proposer_deployment_tx_hash, on_chain_proposer_address) =
        deploy_on_chain_proposer(
            deployer,
            deployer_private_key,
            overrides.clone(),
            &eth_client,
        )
        .await;
    println!(
        "OnChainProposer deployed at address {:#x} with tx hash {:#x}",
        on_chain_proposer_address, on_chain_proposer_deployment_tx_hash
    );

    let (bridge_deployment_tx_hash, bridge_address) =
        deploy_bridge(deployer, deployer_private_key, overrides, &eth_client).await;
    println!(
        "Bridge deployed at address {:#x} with tx hash {:#x}",
        bridge_address, bridge_deployment_tx_hash
    );
}

async fn deploy_on_chain_proposer(
    deployer: Address,
    deployer_private_key: SecretKey,
    overrides: Overrides,
    eth_client: &EthClient,
) -> (H256, Address) {
    // Both the contract path and the output path are relative to where the Makefile is.
    assert!(
        Command::new("solc")
            .arg("--bin")
            .arg("./contracts/src/l1/OnChainProposer.sol")
            .arg("-o")
            .arg("contracts/solc_out")
            .spawn()
            .expect("Failed to spawn solc")
            .wait()
            .expect("Failed to wait for solc")
            .success(),
        "Failed to compile OnChainProposer.sol"
    );

    let on_chain_proposer_init_code = hex::decode(
        std::fs::read_to_string("./contracts/solc_out/OnChainProposer.bin")
            .expect("Failed to read on_chain_proposer_init_code"),
    )
    .expect("Failed to decode on_chain_proposer_init_code")
    .into();

    let (deploy_tx_hash, on_chain_proposer_address) = create2_deploy(
        deployer,
        deployer_private_key,
        &on_chain_proposer_init_code,
        overrides,
        eth_client,
    )
    .await;

    (deploy_tx_hash, on_chain_proposer_address)
}

async fn deploy_bridge(
    deployer: Address,
    deployer_private_key: SecretKey,
    overrides: Overrides,
    eth_client: &EthClient,
) -> (H256, Address) {
    assert!(
        Command::new("solc")
            .arg("--bin")
            .arg("./contracts/src/l1/CommonBridge.sol")
            .arg("-o")
            .arg("contracts/solc_out")
            .spawn()
            .expect("Failed to spawn solc")
            .wait()
            .expect("Failed to wait for solc")
            .success(),
        "Failed to compile CommonBridge.sol"
    );

    let bridge_init_code = hex::decode(
        std::fs::read_to_string("./contracts/solc_out/CommonBridge.bin")
            .expect("Failed to read bridge_init_code"),
    )
    .expect("Failed to decode bridge_init_code")
    .into();

    let (deploy_tx_hash, bridge_address) = create2_deploy(
        deployer,
        deployer_private_key,
        &bridge_init_code,
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
    let calldata = [SALT.as_bytes(), init_code].concat();
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

    while eth_client
        .get_transaction_receipt(deploy_tx_hash)
        .await
        .expect("Failed to get transaction receipt")
        .is_none()
    {
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    }

    let deployed_address = create2_address(keccak(init_code));

    (deploy_tx_hash, deployed_address)
}

fn create2_address(init_code_hash: H256) -> Address {
    Address::from_slice(
        keccak(
            [
                &[0xff],
                DETERMINISTIC_CREATE2_ADDRESS.as_bytes(),
                SALT.as_bytes(),
                init_code_hash.as_bytes(),
            ]
            .concat(),
        )
        .as_bytes()
        .get(12..)
        .expect("Failed to get create2 address"),
    )
}
