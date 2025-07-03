use clap::{Parser, ValueEnum};
use ethereum_types::{Address, H160, H256, U256};
use ethrex_blockchain::constants::TX_GAS_COST;
use ethrex_l2_common::calldata::Value;
use ethrex_l2_sdk::calldata::{self};
use ethrex_l2_sdk::get_address_from_secret_key;
use ethrex_rpc::clients::{EthClient, EthClientError, Overrides};
use ethrex_rpc::types::block_identifier::{BlockIdentifier, BlockTag};
use ethrex_rpc::types::receipt::RpcReceipt;
use futures::StreamExt;
use futures::stream::FuturesUnordered;
use hex::ToHex;
use secp256k1::{PublicKey, SecretKey};
use std::fs;
use std::path::Path;
use std::time::Duration;
use tokio::{task::JoinSet, time::sleep};

// ERC20 compiled artifact generated from this tutorial:
// https://medium.com/@kaishinaw/erc20-using-hardhat-a-comprehensive-guide-3211efba98d4
// If you want to modify the behaviour of the contract, edit the ERC20.sol file,
// and compile it with solc.
const ERC20: &str =
    include_str!("../../../fixtures/contracts/ERC20/ERC20.bin/TestToken.bin").trim_ascii();

// This is the bytecode for the contract with the following functions
// version() -> always returns 2
// function fibonacci(uint n) public pure returns (uint) -> returns the nth fib number
const FIBO_CODE: &str = "6080604052348015600e575f5ffd5b506103198061001c5f395ff3fe608060405234801561000f575f5ffd5b5060043610610034575f3560e01c806354fd4d501461003857806361047ff414610056575b5f5ffd5b610040610086565b60405161004d9190610152565b60405180910390f35b610070600480360381019061006b9190610199565b61008b565b60405161007d9190610152565b60405180910390f35b600281565b5f5f8210156100cf576040517f08c379a00000000000000000000000000000000000000000000000000000000081526004016100c69061021e565b60405180910390fd5b5f82036100de575f9050610135565b600182036100ef5760019050610135565b5f5f90505f600190505f600290505b84811161012e575f82905083836101159190610269565b92508093505080806101269061029c565b9150506100fe565b5080925050505b919050565b5f819050919050565b61014c8161013a565b82525050565b5f6020820190506101655f830184610143565b92915050565b5f5ffd5b6101788161013a565b8114610182575f5ffd5b50565b5f813590506101938161016f565b92915050565b5f602082840312156101ae576101ad61016b565b5b5f6101bb84828501610185565b91505092915050565b5f82825260208201905092915050565b7f496e707574206d757374206265206e6f6e2d6e656761746976650000000000005f82015250565b5f610208601a836101c4565b9150610213826101d4565b602082019050919050565b5f6020820190508181035f830152610235816101fc565b9050919050565b7f4e487b71000000000000000000000000000000000000000000000000000000005f52601160045260245ffd5b5f6102738261013a565b915061027e8361013a565b92508282019050808211156102965761029561023c565b5b92915050565b5f6102a68261013a565b91507fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff82036102d8576102d761023c565b5b60018201905091905056fea264697066735822122021e2c2b56b7e23b9555cc95390dfb2979a8526595038818d133d5bb772c01a6564736f6c634300081c0033";

// Contract with a function that touches 100 storage slots on every transaction.
// See `fixtures/contracts/load_test/IOHeavyContract.sol` for the code.
const IO_HEAVY_CODE: &str = "6080604052348015600e575f5ffd5b505f5f90505b6064811015603e57805f8260648110602d57602c6043565b5b018190555080806001019150506014565b506070565b7f4e487b71000000000000000000000000000000000000000000000000000000005f52603260045260245ffd5b6102728061007d5f395ff3fe608060405234801561000f575f5ffd5b506004361061003f575f3560e01c8063431aabc21461004357806358faa02f1461007357806362f8e72a1461007d575b5f5ffd5b61005d6004803603810190610058919061015c565b61009b565b60405161006a9190610196565b60405180910390f35b61007b6100b3565b005b61008561010a565b6040516100929190610196565b60405180910390f35b5f81606481106100a9575f80fd5b015f915090505481565b5f5f90505b60648110156101075760015f82606481106100d6576100d56101af565b5b01546100e29190610209565b5f82606481106100f5576100f46101af565b5b018190555080806001019150506100b8565b50565b5f5f5f6064811061011e5761011d6101af565b5b0154905090565b5f5ffd5b5f819050919050565b61013b81610129565b8114610145575f5ffd5b50565b5f8135905061015681610132565b92915050565b5f6020828403121561017157610170610125565b5b5f61017e84828501610148565b91505092915050565b61019081610129565b82525050565b5f6020820190506101a95f830184610187565b92915050565b7f4e487b71000000000000000000000000000000000000000000000000000000005f52603260045260245ffd5b7f4e487b71000000000000000000000000000000000000000000000000000000005f52601160045260245ffd5b5f61021382610129565b915061021e83610129565b9250828201905080821115610236576102356101dc565b5b9291505056fea264697066735822122055f6d7149afdb56c745a203d432710eaa25a8ccdb030503fb970bf1c964ac03264736f6c634300081b0033";

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

    // Amount of minutes to wait before exiting. If the value is 0, the program will wait indefinitely. If not present, the program will not wait for transactions to be included in blocks.
    #[arg(
        long,
        short = 'w',
        default_value_t = 0,
        help = "Timeout in minutes. If the node doesn't provide updates in this time, it's considered stuck and the load test fails. If 0 is specified, the load test will wait indefinitely."
    )]
    wait: u64,
}

#[derive(ValueEnum, Clone, Debug)] // Derive ValueEnum for TestType
pub enum TestType {
    EthTransfers,
    Erc20,
    Fibonacci,
    IOHeavy,
}

const RETRIES: u64 = 1000;
const ETH_TRANSFER_VALUE: u64 = 1000;

// Private key for the rich account after making the initial deposits on the L2.
const RICH_ACCOUNT: &str = "0xbcdf20249abf0ed6d944c0288fad489e33f66b3960d9e6229c1cd214ed3bbe31";

async fn deploy_contract(
    client: EthClient,
    deployer: (PublicKey, SecretKey),
    contract: Vec<u8>,
) -> eyre::Result<Address> {
    let address = get_address_from_secret_key(&deployer.1)
        .map_err(|e| eyre::eyre!("Failed to get address from secret key: {}", e))?;

    let (_, contract_address) = client
        .deploy(address, deployer.1, contract.into(), Overrides::default())
        .await?;

    eyre::Ok(contract_address)
}

async fn erc20_deploy(client: EthClient, deployer: Account) -> eyre::Result<Address> {
    let erc20_bytecode = hex::decode(ERC20).expect("Failed to decode ERC20 bytecode");
    deploy_contract(client, deployer, erc20_bytecode).await
}

async fn deploy_fibo(client: EthClient, deployer: Account) -> eyre::Result<Address> {
    let fibo_bytecode = hex::decode(FIBO_CODE).expect("Failed to decode Fibo bytecode");
    deploy_contract(client, deployer, fibo_bytecode).await
}

async fn deploy_io_heavy(client: EthClient, deployer: Account) -> eyre::Result<Address> {
    let io_heavy_bytecode = hex::decode(IO_HEAVY_CODE).expect("Failed to decode IO Heavy bytecode");
    deploy_contract(client, deployer, io_heavy_bytecode).await
}

// Given an account vector and the erc20 contract address, claim balance for all accounts.
async fn claim_erc20_balances(
    contract_address: Address,
    client: EthClient,
    accounts: &[Account],
) -> eyre::Result<()> {
    let mut tasks = JoinSet::new();

    for (_, sk) in accounts {
        let contract = contract_address;
        let client = client.clone();
        let sk = *sk;

        tasks.spawn(async move {
            let claim_balance_calldata = calldata::encode_calldata("freeMint()", &[]).unwrap();
            let address = get_address_from_secret_key(&sk)
                .map_err(|e| eyre::eyre!("Failed to get address from secret key: {}", e))
                .unwrap();

            let claim_tx = client
                .build_eip1559_transaction(
                    contract,
                    address,
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
                ));
            }
            Err(err) => {
                return Err(eyre::eyre!(
                    "Failed to assign balance to an account, tx failed: {err}"
                ));
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
    Fibonacci(Address),
    IOHeavy(Address),
}

impl TxBuilder {
    // Returns value, the calldata and the destination (contract or eoa).
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
            TxBuilder::Fibonacci(contract_address) => {
                let fibo_calldata = calldata::encode_calldata(
                    "fibonacci(uint256)",
                    &[Value::Uint(100000000000000_u64.into())],
                )
                .unwrap();
                (None, fibo_calldata, *contract_address)
            }
            TxBuilder::IOHeavy(contract_address) => {
                let io_heavy_calldata =
                    calldata::encode_calldata("incrementNumbers()", &[]).unwrap();
                (None, io_heavy_calldata, *contract_address)
            }
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
    for (_, sk) in accounts {
        let sk = *sk;
        let client = client.clone();
        let tx_builder = tx_builder.clone();
        tasks.push(async move {
            let address =
                get_address_from_secret_key(&sk).expect("Failed to get address from secret key");

            let nonce = client
                .get_nonce(address, BlockIdentifier::Tag(BlockTag::Latest))
                .await
                .unwrap();
            let src = address;
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
                            value,
                            nonce: Some(nonce + i),
                            max_fee_per_gas: Some(u64::MAX),
                            max_priority_fee_per_gas: Some(10_u64),
                            gas_limit: Some(TX_GAS_COST * 100),
                            ..Default::default()
                        },
                    )
                    .await?;
                let client = client.clone();
                sleep(Duration::from_micros(800)).await;
                let _sent = client.send_eip1559_transaction(&tx, &sk).await?;
            }
            println!("{tx_amount} transactions have been sent for {encoded_src}",);
            Ok::<(), EthClientError>(())
        });
    }

    while let Some(result) = tasks.next().await {
        result?; // Propagate errors from tasks
    }
    Ok(())
}

// Waits until the nonce of each account has reached the tx_amount.
async fn wait_until_all_included(
    client: EthClient,
    timeout: Option<Duration>,
    accounts: &[Account],
    tx_amount: u64,
) -> Result<(), String> {
    for (_, sk) in accounts {
        let client = client.clone();
        let src = get_address_from_secret_key(sk).expect("Failed to get address from secret key");
        let encoded_src: String = src.encode_hex();
        let mut last_updated = tokio::time::Instant::now();
        let mut last_nonce = 0;

        loop {
            let nonce = client
                .get_nonce(src, BlockIdentifier::Tag(BlockTag::Latest))
                .await
                .unwrap();
            if nonce >= tx_amount {
                println!(
                    "All transactions sent from {encoded_src} have been included in blocks. Nonce: {nonce}",
                );
                break;
            } else {
                println!(
                    "Waiting for transactions to be included from {encoded_src}. Nonce: {nonce}. Needs: {tx_amount}. Percentage: {:2}%.",
                    (nonce as f64 / tx_amount as f64) * 100.0
                );
            }

            if let Some(timeout) = timeout {
                if last_nonce == nonce {
                    let inactivity_time = last_updated.elapsed();
                    if inactivity_time > timeout {
                        return Err(format!(
                            "Node inactive for {} seconds. Timeout reached.",
                            inactivity_time.as_secs()
                        ));
                    }
                } else {
                    last_nonce = nonce;
                    last_updated = tokio::time::Instant::now();
                }
            }

            sleep(Duration::from_secs(5)).await;
        }
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
        .unwrap_or_else(|_| panic!("Private key is not a valid hex representation {pkey}"));
    let secret_key = SecretKey::from_slice(key.as_bytes())
        .unwrap_or_else(|_| panic!("Invalid private key {pkey}"));
    let public_key = secret_key.public_key(secp256k1::SECP256K1);
    (public_key, secret_key)
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    let pkeys_path = Path::new(&cli.pkeys);
    let accounts = parse_pk_file(pkeys_path)
        .unwrap_or_else(|_| panic!("Failed to parse private keys file {}", pkeys_path.display()));
    let client = EthClient::new(&cli.node).expect("Failed to create EthClient");

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
        TestType::Fibonacci => {
            println!("Fibonacci load test starting");
            println!("Deploying Fibonacci contract...");
            let contract_address = deploy_fibo(client.clone(), deployer)
                .await
                .expect("Failed to deploy Fibonacci contract");
            TxBuilder::Fibonacci(contract_address)
        }
        TestType::IOHeavy => {
            println!("IO Heavy load test starting");
            println!("Deploying IO Heavy contract...");
            let contract_address = deploy_io_heavy(client.clone(), deployer)
                .await
                .expect("Failed to deploy IO Heavy contract");
            TxBuilder::IOHeavy(contract_address)
        }
    };

    println!(
        "Starting load test with {} transactions per account...",
        cli.tx_amount
    );
    let time_now = tokio::time::Instant::now();

    load_test(
        cli.tx_amount,
        &accounts,
        client.clone(),
        chain_id,
        tx_builder,
    )
    .await
    .expect("Failed to load test");

    let wait_time = if cli.wait > 0 {
        Some(Duration::from_secs(cli.wait * 60))
    } else {
        None
    };

    println!("Waiting for all transactions to be included in blocks...");
    wait_until_all_included(client, wait_time, &accounts, cli.tx_amount)
        .await
        .unwrap();

    let elapsed_time = time_now.elapsed();

    println!(
        "Load test finished. Elapsed time: {} seconds",
        elapsed_time.as_secs()
    );
}
