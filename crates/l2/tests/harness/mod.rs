use bytes::Bytes;
use color_eyre::eyre;
use ethrex_common::{Address, H160, H256, U256, types::BlockNumber};
use ethrex_l2::{
    monitor::widget::{L2ToL1MessagesTable, l2_to_l1_messages::L2ToL1MessageRow},
    sequencer::l1_watcher::PrivilegedTransactionData,
};
use ethrex_l2_common::calldata::Value;
use ethrex_l2_rpc::{
    clients::{deploy, send_eip1559_transaction},
    signer::{LocalSigner, Signer},
};
use ethrex_l2_sdk::calldata::encode_calldata;
use ethrex_l2_sdk::{
    L1ToL2TransactionData, bridge_address, get_address_from_secret_key, git_clone,
    wait_for_transaction_receipt,
};
use ethrex_rpc::clients::eth::{L1MessageProof, eth_sender::Overrides, from_hex_string_to_u256};
use ethrex_rpc::{
    EthClient,
    types::block_identifier::{BlockIdentifier, BlockTag},
    types::receipt::RpcReceipt,
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
    time::Duration,
};

pub mod contracts;
pub mod erc20;
pub mod eth;

pub const L2_GAS_COST_MAX_DELTA: U256 = U256([100_000_000_000_000, 0, 0, 0]);
pub const PRIVATE_KEYS_FILE_PATH: &str = "../../fixtures/keys/private_keys_l1.txt";

pub const L1_RPC: &str = "http://localhost:8545";
pub const L2_RPC: &str = "http://localhost:1729";
// 0x0007a881CD95B1484fca47615B64803dad620C8d
const DEFAULT_PROPOSER_COINBASE_ADDRESS: Address = H160([
    0x00, 0x07, 0xa8, 0x81, 0xcd, 0x95, 0xb1, 0x48, 0x4f, 0xca, 0x47, 0x61, 0x5b, 0x64, 0x80, 0x3d,
    0xad, 0x62, 0x0c, 0x8d,
]);

pub async fn deposit(
    l1_client: &EthClient,
    l2_client: &EthClient,
    depositor_pk: SecretKey,
    value: U256,
) -> eyre::Result<()> {
    println!("fetching initial balances on L1 and L2");
    let depositor_address = ethrex_l2_sdk::get_address_from_secret_key(&depositor_pk)?;
    let depositor_l1_initial_balance = l1_client
        .get_balance(depositor_address, BlockIdentifier::Tag(BlockTag::Latest))
        .await?;
    let depositor_l2_initial_balance = l2_client
        .get_balance(depositor_address, BlockIdentifier::Tag(BlockTag::Latest))
        .await?;

    assert!(
        depositor_l1_initial_balance >= value,
        "L1 depositor doesn't have enough balance to deposit"
    );

    let bridge_initial_balance = l1_client
        .get_balance(bridge_address()?, BlockIdentifier::Tag(BlockTag::Latest))
        .await?;

    let fee_vault_balance_before_deposit = l2_client
        .get_balance(fees_vault(), BlockIdentifier::Tag(BlockTag::Latest))
        .await?;

    println!("depositing funds from L1 to L2");

    let deposit_tx_hash =
        ethrex_l2_sdk::deposit_through_transfer(value, depositor_address, &depositor_pk, l1_client)
            .await?;

    println!("waiting for L1 deposit transaction receipt");

    let deposit_tx_receipt =
        ethrex_l2_sdk::wait_for_transaction_receipt(deposit_tx_hash, l1_client, 5).await?;

    assert!(
        deposit_tx_receipt.receipt.status,
        "Deposit transaction failed"
    );

    println!("waiting for L2 deposit transaction receipt");
    let l2_deposit_receipt = wait_for_l2_deposit_receipt(
        deposit_tx_receipt.block_info.block_number,
        l1_client,
        l2_client,
    )
    .await?;
    assert!(
        l2_deposit_receipt.receipt.status,
        "L2 Deposit transaction failed"
    );

    let depositor_l1_balance_after_deposit = l1_client
        .get_balance(depositor_address, BlockIdentifier::default())
        .await?;
    let depositor_l2_balance_after_deposit = l2_client
        .get_balance(depositor_address, BlockIdentifier::Tag(BlockTag::Latest))
        .await?;
    let bridge_balance_after_deposit = l1_client
        .get_balance(bridge_address()?, BlockIdentifier::default())
        .await?;
    let fee_vault_balance_after_deposit = l2_client
        .get_balance(fees_vault(), BlockIdentifier::default())
        .await?;

    assert_eq!(
        depositor_l1_balance_after_deposit,
        depositor_l1_initial_balance
            - value
            - deposit_tx_receipt.tx_info.gas_used * deposit_tx_receipt.tx_info.effective_gas_price,
        "Depositor L1 balance didn't decrease as expected after deposit"
    );
    assert_eq!(
        bridge_balance_after_deposit,
        bridge_initial_balance + value,
        "Bridge balance didn't increase as expected after deposit"
    );
    assert_eq!(
        depositor_l2_balance_after_deposit,
        depositor_l2_initial_balance + value,
        "Deposit recipient L2 balance didn't increase as expected after deposit"
    );
    assert_eq!(
        fee_vault_balance_after_deposit, fee_vault_balance_before_deposit,
        "Fee vault balance should not change after deposit"
    );

    Ok(())
}

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

pub async fn wait_for_l2_deposit_receipt(
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
    let reader = BufReader::new(File::open(env_file_path).expect("Failed to open .env file"));

    for line in reader.lines() {
        let line = line.expect("Failed to read line");
        if line.starts_with("#") {
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

pub fn transfer_value() -> U256 {
    std::env::var("INTEGRATION_TEST_TRANSFER_VALUE")
        .map(|value| U256::from_dec_str(&value).expect("Invalid transfer value"))
        .unwrap_or(U256::from(10_000_000_000u128))
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
        tokio::time::sleep(Duration::from_secs(2)).await;
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

pub async fn test_balance_of(client: &EthClient, token: Address, user: Address) -> U256 {
    let res = client
        .call(
            token,
            encode_calldata("balanceOf(address)", &[Value::Address(user)])
                .unwrap()
                .into(),
            Default::default(),
        )
        .await
        .unwrap();
    from_hex_string_to_u256(&res).unwrap()
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

pub async fn test_send(
    client: &EthClient,
    private_key: &SecretKey,
    to: Address,
    signature: &str,
    data: &[Value],
) -> RpcReceipt {
    let signer: Signer = LocalSigner::new(*private_key).into();
    let mut tx = client
        .build_eip1559_transaction(
            to,
            signer.address(),
            encode_calldata(signature, data).unwrap().into(),
            Default::default(),
        )
        .await
        .unwrap();
    tx.gas_limit *= 2; // tx reverts in some cases otherwise
    let tx_hash = send_eip1559_transaction(client, &tx, &signer)
        .await
        .unwrap();
    ethrex_l2_sdk::wait_for_transaction_receipt(tx_hash, client, 10)
        .await
        .unwrap()
}

pub async fn test_deploy(
    l2_client: &EthClient,
    init_code: &[u8],
    deployer_private_key: &SecretKey,
) -> Result<Address, Box<dyn std::error::Error>> {
    println!("Deploying contract on L2");

    let deployer: Signer = LocalSigner::new(*deployer_private_key).into();

    let deployer_balance_before_deploy = l2_client
        .get_balance(deployer.address(), BlockIdentifier::Tag(BlockTag::Latest))
        .await?;

    let fee_vault_balance_before_deploy = l2_client
        .get_balance(fees_vault(), BlockIdentifier::Tag(BlockTag::Latest))
        .await?;

    let (deploy_tx_hash, contract_address) = deploy(
        l2_client,
        &deployer,
        init_code.to_vec().into(),
        Overrides::default(),
    )
    .await?;

    let deploy_tx_receipt =
        ethrex_l2_sdk::wait_for_transaction_receipt(deploy_tx_hash, l2_client, 5).await?;

    let deploy_fees = get_fees_details_l2(deploy_tx_receipt, l2_client).await;

    let deployer_balance_after_deploy = l2_client
        .get_balance(deployer.address(), BlockIdentifier::Tag(BlockTag::Latest))
        .await?;

    assert_eq!(
        deployer_balance_after_deploy,
        deployer_balance_before_deploy - deploy_fees.total_fees,
        "Deployer L2 balance didn't decrease as expected after deploy"
    );

    let fee_vault_balance_after_deploy = l2_client
        .get_balance(fees_vault(), BlockIdentifier::Tag(BlockTag::Latest))
        .await?;

    assert_eq!(
        fee_vault_balance_after_deploy,
        fee_vault_balance_before_deploy + deploy_fees.recoverable_fees,
        "Fee vault balance didn't increase as expected after deploy"
    );

    let deployed_contract_balance = l2_client
        .get_balance(contract_address, BlockIdentifier::Tag(BlockTag::Latest))
        .await?;

    assert!(
        deployed_contract_balance.is_zero(),
        "Deployed contract balance should be zero after deploy"
    );

    Ok(contract_address)
}

pub async fn test_deploy_l1(
    client: &EthClient,
    init_code: &[u8],
    private_key: &SecretKey,
) -> Result<Address, Box<dyn std::error::Error>> {
    println!("Deploying contract on L1");

    let deployer_signer: Signer = LocalSigner::new(*private_key).into();

    let (deploy_tx_hash, contract_address) = deploy(
        client,
        &deployer_signer,
        init_code.to_vec().into(),
        Overrides::default(),
    )
    .await?;

    ethrex_l2_sdk::wait_for_transaction_receipt(deploy_tx_hash, client, 5).await?;

    Ok(contract_address)
}

pub async fn perform_transfer(
    l2_client: &EthClient,
    transferer_private_key: &SecretKey,
    transfer_recipient_address: Address,
    transfer_value: U256,
) -> Result<(), Box<dyn std::error::Error>> {
    let transferer_address = ethrex_l2_sdk::get_address_from_secret_key(transferer_private_key)?;

    let transferer_initial_l2_balance = l2_client
        .get_balance(transferer_address, BlockIdentifier::Tag(BlockTag::Latest))
        .await?;

    assert!(
        transferer_initial_l2_balance >= transfer_value,
        "L2 transferer doesn't have enough balance to transfer"
    );

    let transfer_recipient_initial_balance = l2_client
        .get_balance(
            transfer_recipient_address,
            BlockIdentifier::Tag(BlockTag::Latest),
        )
        .await?;

    let fee_vault_balance_before_transfer = l2_client
        .get_balance(fees_vault(), BlockIdentifier::Tag(BlockTag::Latest))
        .await?;

    let transfer_tx = ethrex_l2_sdk::transfer(
        transfer_value,
        transferer_address,
        transfer_recipient_address,
        transferer_private_key,
        l2_client,
    )
    .await?;

    let transfer_tx_receipt =
        ethrex_l2_sdk::wait_for_transaction_receipt(transfer_tx, l2_client, 1000).await?;

    assert!(
        transfer_tx_receipt.receipt.status,
        "Transfer transaction failed"
    );

    let recoverable_fees_vault_balance = l2_client
        .get_balance(fees_vault(), BlockIdentifier::Tag(BlockTag::Latest))
        .await?;

    println!("Recoverable Fees Balance: {recoverable_fees_vault_balance}",);

    println!("Checking balances on L2 after transfer");

    let transferer_l2_balance_after_transfer = l2_client
        .get_balance(transferer_address, BlockIdentifier::Tag(BlockTag::Latest))
        .await?;

    assert!(
        (transferer_initial_l2_balance - transfer_value)
            .abs_diff(transferer_l2_balance_after_transfer)
            < L2_GAS_COST_MAX_DELTA,
        "L2 transferer balance didn't decrease as expected after transfer. Gas costs were {}/{L2_GAS_COST_MAX_DELTA}",
        (transferer_initial_l2_balance - transfer_value)
            .abs_diff(transferer_l2_balance_after_transfer)
    );

    let transfer_recipient_l2_balance_after_transfer = l2_client
        .get_balance(
            transfer_recipient_address,
            BlockIdentifier::Tag(BlockTag::Latest),
        )
        .await?;

    assert_eq!(
        transfer_recipient_l2_balance_after_transfer,
        transfer_recipient_initial_balance + transfer_value,
        "L2 transfer recipient balance didn't increase as expected after transfer"
    );

    let fee_vault_balance_after_transfer = l2_client
        .get_balance(fees_vault(), BlockIdentifier::Tag(BlockTag::Latest))
        .await?;

    let transfer_fees = get_fees_details_l2(transfer_tx_receipt, l2_client).await;

    assert_eq!(
        fee_vault_balance_after_transfer,
        fee_vault_balance_before_transfer + transfer_fees.recoverable_fees,
        "Fee vault balance didn't increase as expected after transfer"
    );

    Ok(())
}

pub async fn test_call_to_contract_with_deposit(
    l1_client: &EthClient,
    l2_client: &EthClient,
    deployed_contract_address: Address,
    calldata_to_contract: Bytes,
    caller_private_key: &SecretKey,
    value: U256,
    should_revert: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let caller_address = ethrex_l2_sdk::get_address_from_secret_key(caller_private_key)
        .expect("Failed to get address");

    println!("Checking balances before call");

    let caller_l1_balance_before_call = l1_client
        .get_balance(caller_address, BlockIdentifier::Tag(BlockTag::Latest))
        .await?;

    let deployed_contract_balance_before_call = l2_client
        .get_balance(
            deployed_contract_address,
            BlockIdentifier::Tag(BlockTag::Latest),
        )
        .await?;

    let fee_vault_balance_before_call = l2_client
        .get_balance(fees_vault(), BlockIdentifier::Tag(BlockTag::Latest))
        .await?;

    println!("Calling contract on L2 with deposit");

    let l1_to_l2_tx_hash = ethrex_l2_sdk::send_l1_to_l2_tx(
        caller_address,
        Some(0),
        None,
        L1ToL2TransactionData::new(
            deployed_contract_address,
            21000 * 5,
            value,
            calldata_to_contract.clone(),
        ),
        caller_private_key,
        bridge_address()?,
        l1_client,
    )
    .await?;

    println!("Waiting for L1 to L2 transaction receipt on L1");

    let l1_to_l2_tx_receipt = wait_for_transaction_receipt(l1_to_l2_tx_hash, l1_client, 5).await?;

    assert!(l1_to_l2_tx_receipt.receipt.status);

    println!("Waiting for L1 to L2 transaction receipt on L2");

    let _ = wait_for_l2_deposit_receipt(
        l1_to_l2_tx_receipt.block_info.block_number,
        l1_client,
        l2_client,
    )
    .await?;

    println!("Checking balances after call");

    let caller_l1_balance_after_call = l1_client
        .get_balance(caller_address, BlockIdentifier::Tag(BlockTag::Latest))
        .await?;

    assert_eq!(
        caller_l1_balance_after_call,
        caller_l1_balance_before_call
            - l1_to_l2_tx_receipt.tx_info.gas_used
                * l1_to_l2_tx_receipt.tx_info.effective_gas_price,
        "Caller L1 balance didn't decrease as expected after call"
    );

    let fee_vault_balance_after_call = l2_client
        .get_balance(fees_vault(), BlockIdentifier::Tag(BlockTag::Latest))
        .await?;

    assert_eq!(
        fee_vault_balance_after_call, fee_vault_balance_before_call,
        "Fee vault balance increased unexpectedly after call"
    );

    let deployed_contract_balance_after_call = l2_client
        .get_balance(
            deployed_contract_address,
            BlockIdentifier::Tag(BlockTag::Latest),
        )
        .await?;

    let value = if should_revert { U256::zero() } else { value };

    assert_eq!(
        deployed_contract_balance_before_call + value,
        deployed_contract_balance_after_call,
        "Deployed contract final balance was not expected"
    );

    Ok(())
}
