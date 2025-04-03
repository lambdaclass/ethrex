use bytes::Bytes;
use clap::{Parser, ValueEnum};
use ethereum_types::{Address, H256, U256};
use ethrex_blockchain::constants::TX_GAS_COST;
use ethrex_common::H160;
use ethrex_l2_sdk::{
    calldata::{self, Value},
    get_address_from_secret_key,
};
use ethrex_rpc::{
    clients::{
        eth::{eth_sender::Overrides, BlockByNumber, EthClient},
        EthClientError,
    },
    types::receipt::RpcReceipt,
};
use eyre::bail;
use secp256k1::SecretKey;
use std::{
    fs::File,
    io::{self, BufRead},
    path::Path,
    str::FromStr,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::{task::JoinSet, time::sleep};

// ERC20 compiled artifact generated from this tutorial:
// https://medium.com/@kaishinaw/erc20-using-hardhat-a-comprehensive-guide-3211efba98d4
// If you want to modify the behaviour of the contract, edit the ERC20.sol file,
// and compile it with solc.
const ERC20: &str = include_str!("../../test_data/ERC20/ERC20.bin/TestToken.bin").trim_ascii();

#[derive(Debug, Clone, ValueEnum)]
pub enum TestType {
    PlainTransactions,
    Fibonacci,
    IoHeavy,
    Erc20,
}

#[derive(Parser)]
struct Command {
    #[arg(
        short = 'p',
        long = "path",
        help = "Path to the file containing private keys."
    )]
    path: String,
    #[arg(
        short = 't',
        long = "to",
        help = "Address to send the transactions. Defaults to random."
    )]
    to: Option<Address>,
    #[arg(
            short = 'a',
            long = "value",
            default_value = "1000",
            value_parser = U256::from_dec_str,
            help = "Value to send in each transaction."
        )]
    value: U256,
    #[arg(
        short = 'i',
        long = "iterations",
        default_value = "1000",
        help = "Number of transactions per private key."
    )]
    iterations: u64,
    #[arg(
        short = 'v',
        long = "verbose",
        default_value = "false",
        help = "Prints each transaction."
    )]
    verbose: bool,
    #[arg(
        long = "test_type",
        short = 'y',
        default_value = "plain-transactions",
        help = "Specify the type of test."
    )]
    test_type: TestType,
    #[arg(
        long = "pk",
        help = "Rich account's private_key.",
        value_parser = ethrex_l2_sdk::secret_key_parser,
        default_value = "0x385c546456b6a603a1cfcaa9ec9494ba4832da08dd6bcf4de9a71e4a01b74924"
    )]
    private_key: SecretKey,
    #[arg(
        long = "url",
        short = 'u',
        help = "ethrex's RPC URL.",
        default_value = "http://localhost:8545"
    )]
    ethrex_url: String,
}

#[tokio::main]
async fn main() -> eyre::Result<()> {
    let command = Command::parse();
    command.run().await?;
    Ok(())
}

fn read_lines<P>(filename: P) -> io::Result<io::Lines<io::BufReader<File>>>
where
    P: AsRef<Path>,
{
    let file = File::open(filename)?;
    Ok(io::BufReader::new(file).lines())
}

#[allow(clippy::too_many_arguments)]
async fn transfer_from(
    pk: SecretKey,
    to_address: Address,
    value: U256,
    iterations: u64,
    chain_id: u64,
    verbose: bool,
    calldata: Bytes,
    eth_client: Arc<EthClient>,
) -> eyre::Result<u64> {
    let address = get_address_from_secret_key(&pk)?;

    let nonce = eth_client
        .get_nonce(address, BlockByNumber::Latest)
        .await
        .unwrap();

    let mut retries = 0;

    for i in nonce..(nonce + iterations) {
        if verbose {
            println!("transfer {i:04} from address: {address:#x}");
        }

        let tx = eth_client
            .build_eip1559_transaction(
                to_address,
                address,
                calldata.clone(),
                Overrides {
                    chain_id: Some(chain_id),
                    value: if calldata.is_empty() {
                        Some(value)
                    } else {
                        None
                    },
                    gas_limit: Some(TX_GAS_COST * 100),
                    nonce: Some(i),
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        while let Err(e) = eth_client.send_eip1559_transaction(&tx, &pk).await {
            println!("Transaction failed (PK: {pk:?} - Nonce: {}): {e}", tx.nonce);
            retries += 1;
            sleep(std::time::Duration::from_secs(2)).await;
        }
        sleep(Duration::from_millis(30)).await;
    }

    eyre::Ok(retries)
}

async fn test_connection(eth_client: &EthClient) -> Result<(), EthClientError> {
    const RETRIES: usize = 5;

    let mut retry = 1;
    loop {
        match eth_client.get_chain_id().await {
            Ok(_) => break Ok(()),
            Err(err) if retry == RETRIES => {
                dbg!(retry);
                break Err(err);
            }
            Err(err) => {
                println!(
                    "Couldn't establish connection with client: {err}, retrying {retry}/{RETRIES}"
                );
                sleep(Duration::from_secs(1)).await;
                retry += 1
            }
        }
    }
}

async fn wait_receipt(
    tx_hash: H256,
    retries: Option<u64>,
    eth_client: &EthClient,
) -> eyre::Result<RpcReceipt> {
    let retries = retries.unwrap_or(10_u64);
    for _ in 0..retries {
        match eth_client.get_transaction_receipt(tx_hash).await {
            Err(_) | Ok(None) => {
                let _ = sleep(Duration::from_secs(1)).await;
            }
            Ok(Some(receipt)) => return Ok(receipt),
        };
    }
    Err(eyre::eyre!(
        "Failed to fetch receipt for tx with hash: {}",
        tx_hash
    ))
}

// Deploy the ERC20 from the raw bytecode.
async fn erc20_deploy(
    rich_private_key: SecretKey,
    rich_address: Address,
    eth_client: &EthClient,
) -> eyre::Result<Address> {
    let erc20_bytecode = hex::decode(ERC20)?;
    let (tx_hash, contract_address) = eth_client
        .deploy(
            rich_address,
            rich_private_key,
            erc20_bytecode.into(),
            Overrides::default(),
        )
        .await
        .expect("Failed to deploy ERC20 with config");
    let receipt = wait_receipt(tx_hash, None, eth_client).await?;
    match receipt {
        RpcReceipt { receipt, .. } if receipt.status => Ok(contract_address),
        _ => Err(eyre::eyre!("ERC20 deploy failed: deploy tx failed")),
    }
}

// Given a vector of private keys, derive an address and claim
// ERC20 balance for each one of them.
async fn claim_erc20_balances(
    contract_address: Address,
    private_keys: Vec<SecretKey>,
    eth_client: &EthClient,
) -> eyre::Result<()> {
    let mut tasks = JoinSet::new();

    let eth_client_arc = Arc::new(eth_client.clone());
    for pk in private_keys {
        let contract = contract_address;
        let eth_client_cp = eth_client_arc.clone();
        tasks.spawn(async move {
            let claim_balance_calldata = calldata::encode_calldata("freeMint()", &[]).unwrap();
            let claim_tx = eth_client_cp
                .build_eip1559_transaction(
                    contract,
                    get_address_from_secret_key(&pk).unwrap(),
                    claim_balance_calldata.into(),
                    Default::default(),
                )
                .await
                .unwrap();
            let tx_hash = eth_client_cp
                .clone()
                .send_eip1559_transaction(&claim_tx, &pk)
                .await
                .unwrap();
            wait_receipt(tx_hash, None, &eth_client_cp).await
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

async fn erc20_load_test(
    iterations: u64,
    chain_id: u64,
    contract_address: Address,
    senders: Vec<SecretKey>,
    eth_client: &EthClient,
) -> eyre::Result<()> {
    let mut tasks = JoinSet::new();
    let eth_client_arc = Arc::new(eth_client.clone());

    let mut counter = 0;
    for pk in senders {
        let address = get_address_from_secret_key(&pk)?;
        let nonce = eth_client
            .get_nonce(address, BlockByNumber::Latest)
            .await
            .unwrap();
        for i in 0..iterations {
            let send_calldata = calldata::encode_calldata(
                "transfer(address,uint256)",
                &[Value::Address(H160::random()), Value::Uint(U256::one())],
            )
            .unwrap();
            let send_tx = eth_client
                .build_eip1559_transaction(
                    contract_address,
                    address,
                    send_calldata.into(),
                    Overrides {
                        chain_id: Some(chain_id),
                        nonce: Some(nonce + i),
                        max_fee_per_gas: Some(3121115334),
                        max_priority_fee_per_gas: Some(3000000000),
                        gas_limit: Some(TX_GAS_COST * 100),
                        ..Default::default()
                    },
                )
                .await?;
            sleep(Duration::from_micros(800)).await;
            let eth_client_cp: Arc<EthClient> = eth_client_arc.clone();
            tasks.spawn(async move {
                let _sent = eth_client_cp
                    .send_eip1559_transaction(&send_tx, &pk)
                    .await
                    .unwrap();
            });
        }
        counter += 1;
        println!("ERC-20 transfers for account number {} sent!", counter);
    }
    tasks.join_all().await;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn _generic_load_test(
    test_type: TestType,
    iterations: u64,
    chain_id: u64,
    private_keys: &Vec<SecretKey>,
    to_address: Address,
    value: U256,
    verbose: bool,
    calldata: Bytes,
    eth_client: &EthClient,
) -> eyre::Result<()> {
    println!("TEST_TYPE: {test_type:?}");
    println!("Sending to: {to_address:#x}");
    let eth_client_arc = Arc::new(eth_client.clone());

    let now = Instant::now();
    let mut threads = vec![];
    for pk in private_keys {
        let eth_client_cp = eth_client_arc.clone();
        let thread = tokio::spawn(transfer_from(
            *pk,
            to_address,
            value,
            iterations,
            chain_id,
            verbose,
            calldata.clone(),
            eth_client_cp,
        ));
        threads.push(thread);
    }

    let mut retries = 0;
    for thread in threads {
        retries += thread.await??;
    }

    println!("Total retries: {retries}");
    println!("Total time elapsed: {:.2?}", now.elapsed());

    Ok(())
}

impl Command {
    pub async fn run(self) -> eyre::Result<()> {
        let Command {
            path,
            to,
            value,
            iterations,
            verbose,
            test_type,
            private_key,
            ethrex_url,
        } = self;

        let eth_client = EthClient::new(&ethrex_url);

        let rich_address = get_address_from_secret_key(&private_key)?;

        let private_keys: Vec<SecretKey> = read_lines(path)?
            .map(|pk| SecretKey::from_str(pk.unwrap().trim_start_matches("0x")).unwrap())
            .collect();

        if let Err(err) = test_connection(&eth_client).await {
            bail!("Couldn't establish connection with client: {err}")
        }

        let chain_id = eth_client.get_chain_id().await?.as_u64();

        let (calldata, to_address) = match test_type {
            TestType::PlainTransactions => {
                let calldata = Bytes::new();
                let to_address = match to {
                    Some(address) => address,
                    None => Address::random(),
                };
                (calldata, to_address)
            }
            TestType::Fibonacci => {
                // This is the bytecode for the contract with the following functions
                // version() -> always returns 2
                // function fibonacci(uint n) public pure returns (uint) -> returns the nth fib number
                let init_code = hex::decode("6080604052348015600e575f5ffd5b506103198061001c5f395ff3fe608060405234801561000f575f5ffd5b5060043610610034575f3560e01c806354fd4d501461003857806361047ff414610056575b5f5ffd5b610040610086565b60405161004d9190610152565b60405180910390f35b610070600480360381019061006b9190610199565b61008b565b60405161007d9190610152565b60405180910390f35b600281565b5f5f8210156100cf576040517f08c379a00000000000000000000000000000000000000000000000000000000081526004016100c69061021e565b60405180910390fd5b5f82036100de575f9050610135565b600182036100ef5760019050610135565b5f5f90505f600190505f600290505b84811161012e575f82905083836101159190610269565b92508093505080806101269061029c565b9150506100fe565b5080925050505b919050565b5f819050919050565b61014c8161013a565b82525050565b5f6020820190506101655f830184610143565b92915050565b5f5ffd5b6101788161013a565b8114610182575f5ffd5b50565b5f813590506101938161016f565b92915050565b5f602082840312156101ae576101ad61016b565b5b5f6101bb84828501610185565b91505092915050565b5f82825260208201905092915050565b7f496e707574206d757374206265206e6f6e2d6e656761746976650000000000005f82015250565b5f610208601a836101c4565b9150610213826101d4565b602082019050919050565b5f6020820190508181035f830152610235816101fc565b9050919050565b7f4e487b71000000000000000000000000000000000000000000000000000000005f52601160045260245ffd5b5f6102738261013a565b915061027e8361013a565b92508282019050808211156102965761029561023c565b5b92915050565b5f6102a68261013a565b91507fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff82036102d8576102d761023c565b5b60018201905091905056fea264697066735822122021e2c2b56b7e23b9555cc95390dfb2979a8526595038818d133d5bb772c01a6564736f6c634300081c0033")?;

                let (_, contract_address) = eth_client
                    .deploy(
                        rich_address,
                        private_key,
                        init_code.into(),
                        Overrides::default(),
                    )
                    .await?;

                let calldata = calldata::encode_calldata(
                    "fibonacci(uint256)",
                    &[Value::Uint(100000000000000_u64.into())],
                )?
                .into();
                let to_address = contract_address;
                (calldata, to_address)
            }
            TestType::IoHeavy => {
                // Contract with a function that touches 100 storage slots on every transaction.
                // See `test_data/IOHeavyContract.sol` for the code.
                let init_code = hex::decode("6080604052348015600e575f5ffd5b505f5f90505b6064811015603e57805f8260648110602d57602c6043565b5b018190555080806001019150506014565b506070565b7f4e487b71000000000000000000000000000000000000000000000000000000005f52603260045260245ffd5b6102728061007d5f395ff3fe608060405234801561000f575f5ffd5b506004361061003f575f3560e01c8063431aabc21461004357806358faa02f1461007357806362f8e72a1461007d575b5f5ffd5b61005d6004803603810190610058919061015c565b61009b565b60405161006a9190610196565b60405180910390f35b61007b6100b3565b005b61008561010a565b6040516100929190610196565b60405180910390f35b5f81606481106100a9575f80fd5b015f915090505481565b5f5f90505b60648110156101075760015f82606481106100d6576100d56101af565b5b01546100e29190610209565b5f82606481106100f5576100f46101af565b5b018190555080806001019150506100b8565b50565b5f5f5f6064811061011e5761011d6101af565b5b0154905090565b5f5ffd5b5f819050919050565b61013b81610129565b8114610145575f5ffd5b50565b5f8135905061015681610132565b92915050565b5f6020828403121561017157610170610125565b5b5f61017e84828501610148565b91505092915050565b61019081610129565b82525050565b5f6020820190506101a95f830184610187565b92915050565b7f4e487b71000000000000000000000000000000000000000000000000000000005f52603260045260245ffd5b7f4e487b71000000000000000000000000000000000000000000000000000000005f52601160045260245ffd5b5f61021382610129565b915061021e83610129565b9250828201905080821115610236576102356101dc565b5b9291505056fea264697066735822122055f6d7149afdb56c745a203d432710eaa25a8ccdb030503fb970bf1c964ac03264736f6c634300081b0033")?;

                let (_, contract_address) = eth_client
                    .deploy(
                        rich_address,
                        private_key,
                        init_code.into(),
                        Overrides::default(),
                    )
                    .await?;

                let calldata = calldata::encode_calldata("incrementNumbers()", &[])?.into();
                let to_address = contract_address;

                (calldata, to_address)
            }
            TestType::Erc20 => {
                let contract_address = erc20_deploy(private_key, rich_address, &eth_client).await?;
                claim_erc20_balances(contract_address, private_keys.clone(), &eth_client).await?;
                erc20_load_test(
                    iterations,
                    chain_id,
                    contract_address,
                    private_keys,
                    &eth_client,
                )
                .await?;
                return Ok(());
            }
        };

        _generic_load_test(
            test_type,
            iterations,
            chain_id,
            &private_keys,
            to_address,
            value,
            verbose,
            calldata,
            &eth_client,
        )
        .await
    }
}
