use clap::{Parser, ValueEnum};
use ethereum_types::{Address, H160, H256, U256};
use ethrex_blockchain::constants::TX_GAS_COST;
use ethrex_l2_common::calldata::Value;
use ethrex_l2_rpc::clients::{deploy, send_eip1559_transaction};
use ethrex_l2_rpc::signer::{LocalSigner, Signer};
use ethrex_l2_sdk::calldata::{self};
use ethrex_rpc::clients::{EthClient, EthClientError, Overrides};
use ethrex_rpc::types::block_identifier::{BlockIdentifier, BlockTag};
use ethrex_rpc::types::receipt::RpcReceipt;
use futures::StreamExt;
use futures::stream::FuturesUnordered;
use hex::ToHex;
use secp256k1::SecretKey;
use std::fs;
use std::path::Path;
use tokio::task::JoinSet;

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

// Contract with operations that are asymmetrically costly for a prover based on their gas cost
// See `fixtures/contracts/zkarnage/ZKarnage.sol` for the code.
const ZKARNAGE: &str =
    include_str!("../../../fixtures/contracts/zkarnage/ZKarnage.bin").trim_ascii();

#[derive(Parser)]
#[command(name = "load_test")]
#[command(about = "A CLI tool with a single test flag", long_about = None)]
struct Cli {
    #[arg(
        long,
        short = 'n',
        default_value = "http://localhost:8545",
        env = "RPC_ENDPOINT",
        help = "URL of the node being tested."
    )]
    node: String,

    #[arg(long, short = 'k', help = "Path to the file containing private keys.")]
    pkeys: String,

    #[arg(
        long,
        short='t',
        value_enum,
        default_value_t=TestType::EthTransfers,
        env = "TEST_TYPE",
        help="Type of test to run. Can be eth_transfers or erc20."
    )]
    test_type: TestType,

    #[arg(
        short = 'N',
        long,
        env = "TX_AMOUNT",
        default_value_t = 2000,
        help = "Number of transactions to send."
    )]
    tx_amount: usize,

    // Amount of minutes to wait before exiting. If the value is 0, the program will wait indefinitely. If not present, the program will not wait for transactions to be included in blocks.
    #[arg(
        long,
        short = 'w',
        default_value_t = 0,
        help = "Timeout in minutes. If the node doesn't provide updates in this time, it's considered stuck and the load test fails. If 0 is specified, the load test will wait indefinitely."
    )]
    wait: u64,
    #[arg(
        long,
        default_value_t = true,
        env = "WAIT_INCLUDED",
        help = "Wait until all transactions were included"
    )]
    wait_included: bool,
}

#[derive(ValueEnum, Clone, Debug)] // Derive ValueEnum for TestType
pub enum TestType {
    EthTransfers,
    Erc20Deploy,
    Erc20DeployClaim,
    Erc20Transfers,
    Fibonacci,
    IOHeavy,
    ZKarnage,
}

const RETRIES: u64 = 1000;
const ETH_TRANSFER_VALUE: u64 = 1000;

// Private key for the rich account after making the initial deposits on the L2.
const RICH_ACCOUNT: &str = "0xbcdf20249abf0ed6d944c0288fad489e33f66b3960d9e6229c1cd214ed3bbe31";
// ERC20 contract address in case RICH_ACCOUNT deploying it for the first time
const ERC20_ADDRESS: &str = "0x17435cce3d1b4fa2e5f8a08ed921d57c6762a180";

async fn deploy_contract(
    client: EthClient,
    deployer: &Signer,
    contract: Vec<u8>,
) -> eyre::Result<Address> {
    let (_, contract_address) =
        deploy(&client, deployer, contract.into(), Overrides::default()).await?;

    eyre::Ok(contract_address)
}

async fn erc20_deploy(client: EthClient, deployer: &Signer) -> eyre::Result<Address> {
    let erc20_bytecode = hex::decode(ERC20).expect("Failed to decode ERC20 bytecode");
    deploy_contract(client, deployer, erc20_bytecode).await
}

async fn deploy_fibo(client: EthClient, deployer: &Signer) -> eyre::Result<Address> {
    let fibo_bytecode = hex::decode(FIBO_CODE).expect("Failed to decode Fibo bytecode");
    deploy_contract(client, deployer, fibo_bytecode).await
}

async fn deploy_io_heavy(client: EthClient, deployer: &Signer) -> eyre::Result<Address> {
    let io_heavy_bytecode = hex::decode(IO_HEAVY_CODE).expect("Failed to decode IO Heavy bytecode");
    deploy_contract(client, deployer, io_heavy_bytecode).await
}

async fn deploy_zkarnage(client: EthClient, deployer: &Signer) -> eyre::Result<Address> {
    let zkarnage_bytecode = hex::decode(ZKARNAGE).expect("Failed to decode ZKarnage bytecode");
    deploy_contract(client, deployer, zkarnage_bytecode).await
}

// Given an account vector and the erc20 contract address, claim balance for all accounts.
async fn claim_erc20_balances(
    contract_address: Address,
    client: EthClient,
    accounts: Vec<Signer>,
) -> eyre::Result<()> {
    let mut tasks = JoinSet::new();

    for account in accounts {
        let contract = contract_address;
        let client = client.clone();

        tasks.spawn(async move {
            let claim_balance_calldata = calldata::encode_calldata("freeMint()", &[]).unwrap();

            let claim_tx = client
                .build_eip1559_transaction(
                    contract,
                    account.address(),
                    claim_balance_calldata.into(),
                    Default::default(),
                )
                .await
                .unwrap();
            let tx_hash = send_eip1559_transaction(&client, &claim_tx, &account)
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
    Deploy(Vec<u8>),
    Noop,
    ZKarnage(Address),
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
            TxBuilder::Deploy(code) => (None, code.clone(), Address::zero()),
            TxBuilder::ZKarnage(contract_address) => {
                let zkarnage_calldata =
                    calldata::encode_calldata("executeJumpdestAttack()", &[Value::Uint(10.into())])
                        .unwrap();
                (None, zkarnage_calldata, *contract_address)
            }
            TxBuilder::Noop => unreachable!(),
        }
    }
}

async fn load_test(
    tx_amount: usize,
    accounts: Vec<Signer>,
    client: EthClient,
    chain_id: u64,
    tx_builder: TxBuilder,
) -> eyre::Result<Vec<H256>> {
    let mut tx_hashes = Vec::with_capacity(tx_amount);
    let tx_amount_per_account = tx_amount.div_ceil(accounts.len()).max(1);
    for account in accounts.iter() {
        let mut tasks = FuturesUnordered::new();
        let nonce = client
            .get_nonce(account.address(), BlockIdentifier::Tag(BlockTag::Latest))
            .await
            .unwrap();
        for i in 0..tx_amount_per_account.min(tx_amount - tx_hashes.len()) {
            let tx_builder = tx_builder.clone();
            let client = client.clone();
            tasks.push(async move {
                let (value, calldata, dst) = tx_builder.build_tx();
                let tx = client
                    .build_eip1559_transaction(
                        dst,
                        account.address(),
                        calldata.into(),
                        Overrides {
                            chain_id: Some(chain_id),
                            value,
                            nonce: Some(nonce + i as u64),
                            max_fee_per_gas: Some(u64::MAX),
                            max_priority_fee_per_gas: Some(10_u64),
                            gas_limit: Some(TX_GAS_COST * 100),
                            ..Default::default()
                        },
                    )
                    .await?;
                let client = client.clone();
                let result = send_eip1559_transaction(&client, &tx, &account).await;
                let tx_hash = result?;
                Ok::<_, EthClientError>(tx_hash)
            });
        }
        while let Some(result) = tasks.next().await {
            tx_hashes.push(result?);
        }
        println!("Sent {}/{tx_amount} transactions", tx_hashes.len());
    }

    Ok(tx_hashes)
}

// Waits until the nonce of each account has reached the tx_amount.
async fn wait_until_all_included(
    client: EthClient,
    accounts: Vec<Signer>,
    tx_hashes: &[H256],
) -> Result<(), String> {
    for account in accounts {
        let client = client.clone();
        let src = account.address();
        let encoded_src: String = src.encode_hex();

        let mut tasks = FuturesUnordered::new();
        for tx_hash in tx_hashes {
            tasks.push(client.wait_for_transaction_receipt(*tx_hash, 50));
        }

        let mut completed_tasks = 0;
        while let Some(result) = tasks.next().await {
            let result = result.map_err(|err| err.to_string())?;
            assert!(result.receipt.status);
            completed_tasks += 1;
            println!(
                "{:.2}% of transactions from {encoded_src} were executed",
                (completed_tasks as f64 / tasks.len() as f64) * 100.0
            );
        }

        println!("All transactions sent from {encoded_src} have been included in blocks",);
    }

    Ok(())
}

fn parse_pk_file(path: &Path) -> eyre::Result<Vec<Signer>> {
    let pkeys_content = fs::read_to_string(path).expect("Unable to read private keys file");
    let accounts: Vec<Signer> = pkeys_content
        .lines()
        .map(parse_private_key_into_local_signer)
        .collect();

    Ok(accounts)
}

fn parse_private_key_into_local_signer(pkey: &str) -> Signer {
    let key = pkey
        .parse::<H256>()
        .unwrap_or_else(|_| panic!("Private key is not a valid hex representation {pkey}"));
    let secret_key = SecretKey::from_slice(key.as_bytes())
        .unwrap_or_else(|_| panic!("Invalid private key {}", pkey));
    LocalSigner::new(secret_key).into()
}

fn parse_address(addr: &str) -> Address {
    addr.parse()
        .unwrap_or_else(|_| panic!("Address is not a valid hex representation {addr}"))
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

    let deployer = parse_private_key_into_local_signer(RICH_ACCOUNT);

    let tx_builder = match cli.test_type {
        TestType::Erc20Deploy => {
            println!("ERC20 deploy load test starting");
            TxBuilder::Deploy(hex::decode(ERC20).expect("Failed to decode ERC20 bytecode"))
        }
        TestType::Erc20DeployClaim => {
            println!("ERC20 deployment & claim starting (executes once, ignores --tx-amount)");
            println!("Deploying ERC20 contract...");
            let contract_address = erc20_deploy(client.clone(), &deployer)
                .await
                .expect("Failed to deploy ERC20 contract");
            println!("deployed at {contract_address:?}");
            claim_erc20_balances(contract_address, client.clone(), accounts.clone())
                .await
                .expect("Failed to claim ERC20 balances");
            TxBuilder::Noop
        }
        TestType::Erc20Transfers => {
            println!("ERC20 transfer load test starting");
            let contract_address = parse_address(ERC20_ADDRESS);
            TxBuilder::Erc20(contract_address)
        }
        TestType::EthTransfers => {
            println!("Eth transfer load test starting");
            TxBuilder::EthTransfer
        }
        TestType::Fibonacci => {
            println!("Fibonacci load test starting");
            println!("Deploying Fibonacci contract...");
            let contract_address = deploy_fibo(client.clone(), &deployer)
                .await
                .expect("Failed to deploy Fibonacci contract");
            TxBuilder::Fibonacci(contract_address)
        }
        TestType::IOHeavy => {
            println!("IO Heavy load test starting");
            println!("Deploying IO Heavy contract...");
            let contract_address = deploy_io_heavy(client.clone(), &deployer)
                .await
                .expect("Failed to deploy IO Heavy contract");
            TxBuilder::IOHeavy(contract_address)
        }
        TestType::ZKarnage => {
            println!("ZKarnage starting");
            println!("Deploying ZKarnage contract...");
            let contract_address = deploy_zkarnage(client.clone(), &deployer)
                .await
                .expect("Failed to deploy ZKarnage contract");
            println!("deployed at {contract_address:?}");
            TxBuilder::ZKarnage(contract_address)
        }
    };

    let time_now = tokio::time::Instant::now();

    if !matches!(tx_builder, TxBuilder::Noop) {
        println!("Starting load test with {} transactions...", cli.tx_amount);
        let tx_hashes = load_test(
            cli.tx_amount,
            accounts.clone(),
            client.clone(),
            chain_id,
            tx_builder,
        )
        .await
        .expect("Failed to load test");

        if cli.wait_included {
            println!("Waiting for all transactions to be included in blocks...");
            wait_until_all_included(client, accounts, &tx_hashes)
                .await
                .unwrap();
        }
    }
    let elapsed_time = time_now.elapsed();

    println!(
        "Load test finished. Elapsed time: {} seconds",
        elapsed_time.as_secs()
    );
}
