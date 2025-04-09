use clap::{Parser, ValueEnum};
use ethereum_types::{Address, H160, H256, U256};
use ethrex_blockchain::constants::TX_GAS_COST;
use ethrex_l2_sdk::calldata::{self, Value};
use ethrex_rpc::clients::eth::BlockByNumber;
use ethrex_rpc::clients::{EthClient, EthClientError, Overrides};
use ethrex_rpc::types::receipt::RpcReceipt;
use futures::stream::FuturesUnordered;
use futures::StreamExt;
use hex::ToHex;
use keccak_hash::keccak;
use secp256k1::{PublicKey, SecretKey};
use std::fs;
use std::path::Path;
use std::time::Duration;
use tokio::{task::JoinSet, time::sleep};

// ERC20 compiled artifact generated from this tutorial:
// https://medium.com/@kaishinaw/erc20-using-hardhat-a-comprehensive-guide-3211efba98d4
// If you want to modify the behaviour of the contract, edit the ERC20.sol file,
// and compile it with solc.
const ERC20: &str = include_str!("../../../test_data/ERC20/ERC20.bin/TestToken.bin").trim_ascii();
type Account = (PublicKey, SecretKey);

#[derive(Parser)]
#[command(name = "load_test")]
#[command(about = "A CLI tool with a single test flag", long_about = None)]
struct Cli {
    #[arg(
        long,
        short = 'n',
        default_value = "http://localhost:8545",
        help = "URL of the node being tested."
    )]
    node: String,

    #[arg(long, short = 'k', help = "Path to the file containing private keys.")]
    pkeys: String,

    #[arg(long, short='t', value_enum, default_value_t=TestType::Erc20, help="Type of test to run. Can be eth_transfers or erc20.")]
    test_type: TestType,

    #[arg(
        short = 'N',
        long,
        default_value_t = 1000,
        help = "Number of transactions to send for each account."
    )]
    tx_amount: u64,
}

#[derive(ValueEnum, Clone, Debug)] // Derive ValueEnum for TestType
pub enum TestType {
    EthTransfers,
    Erc20,
}

const RETRIES: u64 = 1000;
const ETH_TRANSFER_VALUE: u64 = 1000;

// Private key for the rich account present in the gesesis_l2.json file.
const RICH_ACCOUNT: &str = "0x385c546456b6a603a1cfcaa9ec9494ba4832da08dd6bcf4de9a71e4a01b74924";

// TODO: this should be in common utils.
fn address_from_pub_key(public_key: PublicKey) -> H160 {
    let bytes = public_key.serialize_uncompressed();
    let hash = keccak(&bytes[1..]);
    let address_bytes: [u8; 20] = hash.as_ref().get(12..32).unwrap().try_into().unwrap();

    Address::from(address_bytes)
}

async fn deploy_contract(
    client: EthClient,
    deployer: (PublicKey, SecretKey),
    contract: Vec<u8>,
) -> eyre::Result<Address> {
    let (_, contract_address) = client
        .deploy(
            address_from_pub_key(deployer.0),
            deployer.1,
            contract.into(),
            Overrides::default(),
        )
        .await?;

    eyre::Ok(contract_address)
}

async fn erc20_deploy(
    client: EthClient,
    deployer: (PublicKey, SecretKey),
) -> eyre::Result<Address> {
    let erc20_bytecode = hex::decode(ERC20).expect("Failed to decode ERC20 bytecode");
    deploy_contract(client, deployer, erc20_bytecode).await
}

// Given an account vector and the erc20 contract address, claim balance for all accounts.
async fn claim_erc20_balances(
    contract_address: Address,
    client: EthClient,
    accounts: &[Account],
) -> eyre::Result<()> {
    let mut tasks = JoinSet::new();

    for (pk, sk) in accounts {
        let contract = contract_address;
        let client = client.clone();
        let pk = pk.clone();
        let sk = sk.clone();

        tasks.spawn(async move {
            let claim_balance_calldata = calldata::encode_calldata("freeMint()", &[]).unwrap();
            let claim_tx = client
                .build_eip1559_transaction(
                    contract,
                    address_from_pub_key(pk.clone()),
                    claim_balance_calldata.into(),
                    Default::default(),
                )
                .await
                .unwrap();
            let tx_hash = client
                .send_eip1559_transaction(&claim_tx, &sk)
                .await
                .unwrap();
            client.wait_for_transaction_receipt(tx_hash, RETRIES).await
        });
    }
    for response in tasks.join_all().await {
        match response {
            Ok(RpcReceipt { receipt, .. }) if !receipt.status => {
                return Err(eyre::eyre!(
                    "Failed to assign balance to an account, tx failed with receipt: {receipt:?}"
                ))
            }
            Err(err) => {
                return Err(eyre::eyre!(
                    "Failed to assign balance to an account, tx failed: {err}"
                ))
            }
            Ok(_) => {
                continue;
            }
        }
    }
    Ok(())
}

#[derive(Clone)]
enum TxBuilder {
    Erc20(Address),
    EthTransfer,
}

impl TxBuilder {
    // Returns the destination address, the calldata and the contract address.
    fn build_tx(&self) -> (Option<U256>, Vec<u8>, H160) {
        let dst = H160::random();
        match self {
            TxBuilder::Erc20(contract_address) => {
                let send_calldata = calldata::encode_calldata(
                    "transfer(address,uint256)",
                    &[Value::Address(dst), Value::Uint(U256::one())],
                )
                .unwrap();
                (None, send_calldata, *contract_address)
            }
            TxBuilder::EthTransfer => (Some(U256::from(ETH_TRANSFER_VALUE)), [].into(), dst),
        }
    }
}

async fn load_test(
    tx_amount: u64,
    accounts: &[Account],
    client: EthClient,
    chain_id: u64,
    tx_builder: TxBuilder,
) -> eyre::Result<()> {
    let mut tasks = FuturesUnordered::new();
    for (pk, sk) in accounts {
        let pk = pk.clone();
        let sk = sk.clone();
        let client = client.clone();
        let tx_builder = tx_builder.clone();
        tasks.push(async move {
            let nonce = client
                .get_nonce(address_from_pub_key(pk), BlockByNumber::Latest)
                .await
                .unwrap();
            let src = address_from_pub_key(pk);
            let encoded_src: String = src.encode_hex();

            for i in 0..tx_amount {
                let (value, calldata, dst) = tx_builder.build_tx();
                let tx = client
                    .build_eip1559_transaction(
                        dst,
                        src,
                        calldata.into(),
                        Overrides {
                            chain_id: Some(chain_id),
                            value: value,
                            nonce: Some(nonce + i),
                            max_fee_per_gas: Some(3121115334),
                            max_priority_fee_per_gas: Some(3000000000),
                            gas_limit: Some(TX_GAS_COST * 100),
                            ..Default::default()
                        },
                    )
                    .await?;
                let client = client.clone();
                sleep(Duration::from_micros(800)).await;
                let _sent = client.send_eip1559_transaction(&tx, &sk).await?;
                println!(
                    "Tx number {} sent! From: {}. To: {}",
                    nonce + i + 1,
                    encoded_src,
                    dst.encode_hex::<String>()
                );
            }
            Ok::<(), EthClientError>(())
        });
    }

    while let Some(result) = tasks.next().await {
        result?; // Propagate errors from tasks
    }
    Ok(())
}

fn parse_pk_file(path: &Path) -> eyre::Result<Vec<Account>> {
    let pkeys_content = fs::read_to_string(path).expect("Unable to read private keys file");
    let accounts: Vec<Account> = pkeys_content
        .lines()
        .map(parse_private_key_into_account)
        .collect();

    Ok(accounts)
}

fn parse_private_key_into_account(pkey: &str) -> Account {
    let key = pkey
        .parse::<H256>()
        .expect(format!("Private key is not a valid hex representation {}", pkey).as_str());
    let secret_key = SecretKey::from_slice(key.as_bytes())
        .expect(format!("Invalid private key {}", pkey).as_str());
    let public_key = secret_key.public_key(secp256k1::SECP256K1).clone();
    (public_key, secret_key)
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    let pkeys_path = Path::new(&cli.pkeys);
    let accounts = parse_pk_file(pkeys_path)
        .expect(format!("Failed to parse private keys file {}", pkeys_path.display()).as_str());
    let client = EthClient::new(&cli.node);

    // We ask the client for the chain id.
    let chain_id = client
        .get_chain_id()
        .await
        .expect("Failed to get chain id")
        .as_u64();

    let deployer = parse_private_key_into_account(RICH_ACCOUNT);

    let tx_builder = match cli.test_type {
        TestType::Erc20 => {
            println!("ERC20 Load test starting");
            println!("Deploying ERC20 contract...");
            let contract_address = erc20_deploy(client.clone(), deployer)
                .await
                .expect("Failed to deploy ERC20 contract");
            claim_erc20_balances(contract_address, client.clone(), &accounts)
                .await
                .expect("Failed to claim ERC20 balances");
            TxBuilder::Erc20(contract_address)
        }
        TestType::EthTransfers => {
            println!("Eth transfer load test starting");
            TxBuilder::EthTransfer
        }
    };

    load_test(
        cli.tx_amount,
        &accounts,
        client.clone(),
        chain_id,
        tx_builder,
    )
    .await
    .expect("Failed to load test");
}
