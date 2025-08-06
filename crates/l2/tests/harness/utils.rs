#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
#![allow(clippy::indexing_slicing)]

use bytes::Bytes;
use color_eyre::eyre;
use ethrex_common::{Address, H160, H256, U256, types::BlockNumber};
use ethrex_l2::{
    monitor::widget::{L2ToL1MessagesTable, l2_to_l1_messages::L2ToL1MessageRow},
    sequencer::l1_watcher::PrivilegedTransactionData,
};
use ethrex_l2_sdk::{bridge_address, get_address_from_secret_key, git_clone};
use ethrex_rpc::{
    EthClient,
    clients::eth::L1MessageProof,
    types::{
        block_identifier::{BlockIdentifier, BlockTag},
        receipt::RpcReceipt,
    },
};
use hex::FromHexError;
use keccak_hash::keccak;
use secp256k1::SecretKey;
use std::{
    fs::{File, read_to_string},
    io::{BufRead, BufReader},
    ops::Mul,
    path::{Path, PathBuf},
    str::FromStr,
};

pub const L2_GAS_COST_MAX_DELTA: U256 = U256([100_000_000_000_000, 0, 0, 0]);
pub const PRIVATE_KEYS_FILE_PATH: &str = "../../fixtures/keys/private_keys_l1.txt";

pub const L1_RPC: &str = "http://localhost:8545";
pub const L2_RPC: &str = "http://localhost:1729";
// 0x0007a881CD95B1484fca47615B64803dad620C8d
const DEFAULT_PROPOSER_COINBASE_ADDRESS: Address = H160([
    0x00, 0x07, 0xa8, 0x81, 0xcd, 0x95, 0xb1, 0x48, 0x4f, 0xca, 0x47, 0x61, 0x5b, 0x64, 0x80, 0x3d,
    0xad, 0x62, 0x0c, 0x8d,
]);

pub fn l1_client() -> EthClient {
    EthClient::new(&std::env::var("INTEGRATION_TEST_L1_RPC").unwrap_or(L1_RPC.to_string())).unwrap()
}

pub fn l2_client() -> EthClient {
    EthClient::new(&std::env::var("INTEGRATION_TEST_L2_RPC").unwrap_or(L2_RPC.to_string())).unwrap()
}

pub fn fees_vault() -> Address {
    std::env::var("INTEGRATION_TEST_PROPOSER_COINBASE_ADDRESS")
        .map(|address| address.parse().expect("Invalid proposer coinbase address"))
        .unwrap_or(DEFAULT_PROPOSER_COINBASE_ADDRESS)
}

pub fn rich_wallet() -> Vec<SecretKey> {
    std::fs::read_to_string(private_keys_file_path())
        .unwrap()
        .lines()
        .map(|line| line.trim().to_string())
        .map(|hex| hex.trim_start_matches("0x").to_string())
        .map(|trimmed| hex::decode(trimmed).unwrap())
        .map(|decoded| SecretKey::from_slice(&decoded).unwrap())
        .collect()
}

pub fn rich_pk_1() -> SecretKey {
    rich_wallet()[0]
}

pub fn rich_pk_2() -> SecretKey {
    rich_wallet()[1]
}

pub async fn wait_for_l2_ptx_receipt(
    l1_receipt_block_number: BlockNumber,
    l1_client: &EthClient,
    l2_client: &EthClient,
) -> eyre::Result<RpcReceipt> {
    let topic = keccak(b"PrivilegedTxSent(address,address,address,uint256,uint256,uint256,bytes)");
    let logs = l1_client
        .get_logs(
            U256::from(l1_receipt_block_number),
            U256::from(l1_receipt_block_number),
            bridge_address()?,
            vec![topic],
        )
        .await?;
    let data = PrivilegedTransactionData::from_log(logs.first().unwrap().log.clone())?;

    let l2_deposit_tx_hash = data
        .into_tx(
            l1_client,
            l2_client.get_chain_id().await?.try_into().unwrap(),
            0,
        )
        .await
        .unwrap()
        .get_privileged_hash()
        .unwrap();

    Ok(ethrex_l2_sdk::wait_for_transaction_receipt(l2_deposit_tx_hash, l2_client, 1000).await?)
}

pub fn read_env_file_by_config() {
    let env_file_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(".env");
    dbg!(&env_file_path);
    let reader = BufReader::new(File::open(env_file_path).expect("Failed to open .env file"));

    for line in reader.lines() {
        let line = line.expect("Failed to read line");
        if line.starts_with('#') {
            // Skip comments
            continue;
        };
        match line.split_once('=') {
            Some((key, value)) => {
                if std::env::vars().any(|(k, _)| k == key) {
                    continue;
                }
                unsafe { std::env::set_var(key, value) }
            }
            None => continue,
        };
    }
}

pub async fn get_rich_accounts_balance(
    l2_client: &EthClient,
) -> Result<U256, Box<dyn std::error::Error>> {
    let mut total_balance = U256::zero();
    let private_keys_file_path = private_keys_file_path();

    let pks = read_to_string(private_keys_file_path)?;
    let private_keys: Vec<String> = pks
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| line.trim().to_string())
        .collect();

    for pk in private_keys.iter() {
        let secret_key = parse_private_key(pk)?;
        let address = get_address_from_secret_key(&secret_key)?;
        let get_balance = l2_client
            .get_balance(address, BlockIdentifier::Tag(BlockTag::Latest))
            .await?;
        total_balance += get_balance;
    }
    Ok(total_balance)
}

pub fn private_keys_file_path() -> PathBuf {
    match std::env::var("ETHREX_DEPLOYER_PRIVATE_KEYS_FILE_PATH") {
        Ok(path) => PathBuf::from(path),
        Err(_) => {
            println!(
                "ETHREX_DEPLOYER_PRIVATE_KEYS_FILE_PATH not set, using default: {PRIVATE_KEYS_FILE_PATH}",
            );
            PathBuf::from(PRIVATE_KEYS_FILE_PATH)
        }
    }
}

pub fn parse_private_key(s: &str) -> Result<SecretKey, Box<dyn std::error::Error>> {
    Ok(SecretKey::from_slice(&parse_hex(s)?)?)
}

pub fn parse_hex(s: &str) -> Result<Bytes, FromHexError> {
    match s.strip_prefix("0x") {
        Some(s) => hex::decode(s).map(Into::into),
        None => hex::decode(s).map(Into::into),
    }
}

pub fn get_contract_dependencies(contracts_path: &Path) {
    std::fs::create_dir_all(contracts_path.join("lib")).expect("Failed to create contracts/lib");
    git_clone(
        "https://github.com/OpenZeppelin/openzeppelin-contracts-upgradeable.git",
        contracts_path
            .join("lib/openzeppelin-contracts-upgradeable")
            .to_str()
            .expect("Failed to convert path to str"),
        None,
        true,
    )
    .unwrap();
}

// Removes the contracts/lib and contracts/solc_out directories
// generated by the tests.
pub fn clean_contracts_dir() {
    let lib_path = Path::new("contracts/lib");
    let solc_path = Path::new("contracts/solc_out");

    let _ = std::fs::remove_dir_all(lib_path).inspect_err(|e| {
        println!("Failed to remove {}: {}", lib_path.display(), e);
    });
    let _ = std::fs::remove_dir_all(solc_path).inspect_err(|e| {
        println!("Failed to remove {}: {}", solc_path.display(), e);
    });

    println!(
        "Cleaned up {} and {}",
        lib_path.display(),
        solc_path.display()
    );
}

pub fn on_chain_proposer_address() -> Address {
    Address::from_str(
        &std::env::var("ETHREX_COMMITTER_ON_CHAIN_PROPOSER_ADDRESS")
            .expect("ETHREX_COMMITTER_ON_CHAIN_PROPOSER_ADDRESS env var not set"),
    )
    .unwrap()
}

/// Waits until the batch containing L2->L1 message is verified on L1, and returns the proof for that message
pub async fn wait_for_verified_proof(
    l1_client: &EthClient,
    l2_client: &EthClient,
    tx: H256,
) -> L1MessageProof {
    let proof = l2_client.wait_for_message_proof(tx, 1000).await;
    let proof = proof.unwrap().into_iter().next().expect("proof not found");

    while l1_client
        .get_last_verified_batch(on_chain_proposer_address())
        .await
        .unwrap()
        < proof.batch_number
    {
        println!("Withdrawal is not verified on L1 yet");
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    }
    proof
}

#[derive(Debug)]
pub struct FeesDetails {
    pub total_fees: U256,
    pub recoverable_fees: U256,
}

pub async fn get_fees_details_l2(tx_receipt: RpcReceipt, l2_client: &EthClient) -> FeesDetails {
    let total_fees: U256 =
        (tx_receipt.tx_info.gas_used * tx_receipt.tx_info.effective_gas_price).into();

    let effective_gas_price = tx_receipt.tx_info.effective_gas_price;
    let base_fee_per_gas = l2_client
        .get_block_by_number(BlockIdentifier::Number(tx_receipt.block_info.block_number))
        .await
        .unwrap()
        .header
        .base_fee_per_gas
        .unwrap();

    let max_priority_fee_per_gas_transfer: U256 = (effective_gas_price - base_fee_per_gas).into();

    let recoverable_fees = max_priority_fee_per_gas_transfer.mul(tx_receipt.tx_info.gas_used);

    FeesDetails {
        total_fees,
        recoverable_fees,
    }
}

pub async fn find_withdrawal_with_widget(
    bridge_address: Address,
    l2tx: H256,
    l2_client: &EthClient,
    l1_client: &EthClient,
) -> Option<L2ToL1MessageRow> {
    let mut widget = L2ToL1MessagesTable::new(bridge_address);
    widget.on_tick(l1_client, l2_client).await.unwrap();
    widget
        .items
        .iter()
        .find(|row| row.l2_tx_hash == l2tx)
        .cloned()
}
