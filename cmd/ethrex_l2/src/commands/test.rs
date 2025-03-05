use crate::config::EthrexL2Config;
use bytes::Bytes;
use clap::Subcommand;
use ethereum_types::{Address, H256, U256};
use ethrex_blockchain::constants::TX_GAS_COST;
use ethrex_common::H160;
use ethrex_l2_sdk::calldata::{self, Value};
use ethrex_rpc::clients::EthClientError;
use ethrex_rpc::{
    clients::eth::{eth_sender::Overrides, EthClient, EthClientError},
    types::receipt::RpcReceipt,
};
use eyre::bail;
use itertools::Itertools;
use keccak_hash::keccak;
use secp256k1::{PublicKey, SecretKey};
use std::time::Instant;
use std::{
    fs::File,
    io::{self, BufRead},
    path::Path,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    thread::sleep,
    time::Duration,
};
use tokio::task::JoinSet;

// ERC20 compiled artifact generated from this tutorial:
// https://medium.com/@kaishinaw/erc20-using-hardhat-a-comprehensive-guide-3211efba98d4
// If you want to modify the behaviour of the contract, edit the ERC20.sol file,
// and compile it with solc.
const ERC20: &str =
    include_str!("../../../../test_data/ERC20/ERC20.bin/TestToken.bin").trim_ascii();

#[derive(Subcommand)]
pub(crate) enum Command {
    #[clap(about = "Make a load test sending transactions from a list of private keys.")]
    Load {
        #[clap(
            short = 'p',
            long = "path",
            help = "Path to the file containing private keys."
        )]
        path: String,
        #[clap(
            short = 't',
            long = "to",
            help = "Address to send the transactions. Defaults to random."
        )]
        to: Option<Address>,
        #[clap(
            short = 'a',
            long = "value",
            default_value = "1000",
            help = "Value to send in each transaction."
        )]
        value: U256,
        #[clap(
            short = 'i',
            long = "iterations",
            default_value = "1000",
            help = "Number of transactions per private key."
        )]
        iterations: u64,
        #[clap(
            short = 'v',
            long = "verbose",
            default_value = "false",
            help = "Prints each transaction."
        )]
        verbose: bool,
        #[clap(
            short = 'c',
            long = "contract",
            default_value = "false",
            help = "send value to address with contract"
        )]
        contract: bool,
    },
    #[clap(about = "Load test that deploys an ERC20 and runs transactions")]
    ERC20 {
        #[clap(
            short = 't',
            long = "transactions_amount",
            help = "How many transactions each given account will do"
        )]
        transactions: u64,
        #[clap(
            short = 'p',
            long = "path",
            help = "Path to the file containing private keys."
        )]
        path: String,
    },
}

fn read_lines<P>(filename: P) -> io::Result<io::Lines<io::BufReader<File>>>
where
    P: AsRef<Path>,
{
    let file = File::open(filename)?;
    Ok(io::BufReader::new(file).lines())
}

fn address_from_pub_key(public_key: PublicKey) -> H160 {
    let bytes = public_key.serialize_uncompressed();
    let hash = keccak(&bytes[1..]);
    let address_bytes: [u8; 20] = hash.as_ref().get(12..32).unwrap().try_into().unwrap();
    let address = Address::from(address_bytes);

    return address;
}

async fn transfer_from(
    pk: String,
    to_address: Address,
    value: U256,
    iterations: u64,
    verbose: bool,
    calldata: Bytes,
    cfg: EthrexL2Config,
) -> u64 {
    let client = EthClient::new(&cfg.network.l2_rpc_url);
    let private_key = SecretKey::from_slice(pk.parse::<H256>().unwrap().as_bytes()).unwrap();

    let public_key = private_key
        .public_key(secp256k1::SECP256K1)
        .serialize_uncompressed();
    let hash = keccak(&public_key[1..]);

    // Get the last 20 bytes of the hash
    let address_bytes: [u8; 20] = hash.as_ref().get(12..32).unwrap().try_into().unwrap();

    let address = Address::from(address_bytes);
    let nonce = client.get_nonce(address).await.unwrap();

    let mut retries = 0;

    for i in nonce..nonce + iterations {
        if verbose {
            println!("transfer {i} from {pk}");
        }

        let tx = client
            .build_eip1559_transaction(
                to_address,
                address,
                calldata.clone(),
                Overrides {
                    chain_id: Some(cfg.network.l2_chain_id),
                    nonce: Some(i),
                    value: if calldata.is_empty() {
                        Some(value)
                    } else {
                        None
                    },
                    max_fee_per_gas: Some(3121115334),
                    max_priority_fee_per_gas: Some(3000000000),
                    gas_limit: Some(TX_GAS_COST * 5),
                    ..Default::default()
                },
                10,
            )
            .await
            .unwrap();

        while let Err(e) = client.send_eip1559_transaction(&tx, &private_key).await {
            println!("Transaction failed (PK: {pk} - Nonce: {}): {e}", tx.nonce);
            retries += 1;
            sleep(std::time::Duration::from_secs(2));
        }
        sleep(Duration::from_millis(3));
    }

    retries
}

async fn test_connection(cfg: EthrexL2Config) -> Result<(), EthClientError> {
    const RETRIES: usize = 5;

    let client = EthClient::new(&cfg.network.l2_rpc_url);

    let mut retry = 1;
    loop {
        match client.get_chain_id().await {
            Ok(_) => break Ok(()),
            Err(err) if retry == RETRIES => {
                dbg!(retry);
                break Err(err);
            }
            Err(err) => {
                println!("Couldn't establish connection to L2: {err}, retrying {retry}/{RETRIES}");
                tokio::time::sleep(Duration::from_secs(1)).await;
                retry += 1
            }
        }
    }
}

async fn wait_receipt(
    client: EthClient,
    tx_hash: H256,
    retries: Option<u64>,
) -> eyre::Result<RpcReceipt> {
    let retries = retries.unwrap_or_else(|| 10_u64);
    for _ in 0..retries {
        match client.get_transaction_receipt(tx_hash).await {
            Err(_) | Ok(None) => {
                let _ = tokio::time::sleep(Duration::from_secs(1)).await;
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
async fn erc20_deploy(config: &EthrexL2Config) -> eyre::Result<Address> {
    let client = EthClient::new(&config.network.l2_rpc_url);
    let erc20_bytecode = hex::decode(ERC20)?;
    let (tx_hash, contract_address) = client
        .deploy(
            config.wallet.address,
            config.wallet.private_key,
            erc20_bytecode.into(),
            Overrides::default(),
        )
        .await
        .expect("Failed to deploy ERC20 with config: {config}");
    let receipt = wait_receipt(client, tx_hash, None).await?;
    match receipt {
        RpcReceipt { receipt, .. } if receipt.status => Ok(contract_address),
        _ => Err(eyre::eyre!("ERC20 deploy failed: deploy tx failed")),
    }
}

// Given a vector of private keys, derive an address and claim
// ERC20 balance for each one of them.
async fn claim_erc20_balances(
    cfg: &EthrexL2Config,
    contract_address: Address,
    private_keys: &Vec<SecretKey>,
) -> eyre::Result<()> {
    let accounts = private_keys
        .iter()
        .map(|pk| (pk.clone(), pk.public_key(secp256k1::SECP256K1)))
        .collect_vec();
    let mut tasks = JoinSet::new();

    for (sk, pk) in accounts {
        let contract = contract_address.clone();
        let url = cfg.network.l2_rpc_url.clone();
        tasks.spawn(async move {
            let client = EthClient::new(url.as_str());
            let claim_balance_calldata = calldata::encode_calldata("freeMint()", &[]).unwrap();
            let claim_tx = client
                .build_eip1559_transaction(
                    contract,
                    address_from_pub_key(pk),
                    claim_balance_calldata.into(),
                    Default::default(),
                    10,
                )
                .await
                .unwrap();
            let tx_hash = client
                .send_eip1559_transaction(&claim_tx, &sk)
                .await
                .unwrap();
            wait_receipt(client, tx_hash, None).await
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
    return Ok(());
}

async fn erc20_load_test(
    config: &EthrexL2Config,
    tx_amount: u64,
    contract_address: Address,
    senders: &Vec<SecretKey>,
) -> eyre::Result<()> {
    let client = EthClient::new(&config.network.l2_rpc_url);
    let mut tasks = JoinSet::new();
    let accounts = senders
        .iter()
        .map(|pk| (pk.clone(), pk.public_key(secp256k1::SECP256K1)))
        .collect_vec();
    for (sk, pk) in accounts {
        let nonce = client.get_nonce(address_from_pub_key(pk)).await.unwrap();
        for i in 0..tx_amount {
            let send_calldata = calldata::encode_calldata(
                "transfer(address,uint256)",
                &[Value::Address(H160::random()), Value::Uint(U256::one())],
            )
            .unwrap();
            let send_tx = client
                .build_eip1559_transaction(
                    contract_address,
                    address_from_pub_key(pk),
                    send_calldata.into(),
                    Overrides {
                        chain_id: Some(config.network.l2_chain_id),
                        nonce: Some(nonce + i),
                        max_fee_per_gas: Some(3121115334),
                        max_priority_fee_per_gas: Some(3000000000),
                        gas_limit: Some(TX_GAS_COST * 100),
                        ..Default::default()
                    },
                    1,
                )
                .await?;
            let client = client.clone();
            tokio::time::sleep(Duration::from_millis(10)).await;
            tasks.spawn(async move {
                let _sent = client.send_eip1559_transaction(&send_tx, &sk).await.unwrap();
                println!("ERC-20 transfer number {} sent!", nonce + i + 1);
            });
        }
    }
    tasks.join_all().await;
    Ok(())
}

impl Command {
    pub async fn run(self, cfg: EthrexL2Config) -> eyre::Result<()> {
        match self {
            Command::Load {
                path,
                to,
                value,
                iterations,
                verbose,
                contract,
            } => {
                let lines = read_lines(path)?;

                if let Err(err) = test_connection(cfg.clone()).await {
                    bail!("Couldn't establish connection to L2: {err}")
                }

                let mut to_address = match to {
                    Some(address) => address,
                    None => Address::random(),
                };

                let calldata: Bytes = if contract {
                    // This is the bytecode for the contract with the following functions
                    // version() -> always returns 2
                    // function fibonacci(uint n) public pure returns (uint) -> returns the nth fib number
                    let init_code = hex::decode("6080604052348015600e575f5ffd5b506103198061001c5f395ff3fe608060405234801561000f575f5ffd5b5060043610610034575f3560e01c806354fd4d501461003857806361047ff414610056575b5f5ffd5b610040610086565b60405161004d9190610152565b60405180910390f35b610070600480360381019061006b9190610199565b61008b565b60405161007d9190610152565b60405180910390f35b600281565b5f5f8210156100cf576040517f08c379a00000000000000000000000000000000000000000000000000000000081526004016100c69061021e565b60405180910390fd5b5f82036100de575f9050610135565b600182036100ef5760019050610135565b5f5f90505f600190505f600290505b84811161012e575f82905083836101159190610269565b92508093505080806101269061029c565b9150506100fe565b5080925050505b919050565b5f819050919050565b61014c8161013a565b82525050565b5f6020820190506101655f830184610143565b92915050565b5f5ffd5b6101788161013a565b8114610182575f5ffd5b50565b5f813590506101938161016f565b92915050565b5f602082840312156101ae576101ad61016b565b5b5f6101bb84828501610185565b91505092915050565b5f82825260208201905092915050565b7f496e707574206d757374206265206e6f6e2d6e656761746976650000000000005f82015250565b5f610208601a836101c4565b9150610213826101d4565b602082019050919050565b5f6020820190508181035f830152610235816101fc565b9050919050565b7f4e487b71000000000000000000000000000000000000000000000000000000005f52601160045260245ffd5b5f6102738261013a565b915061027e8361013a565b92508282019050808211156102965761029561023c565b5b92915050565b5f6102a68261013a565b91507fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff82036102d8576102d761023c565b5b60018201905091905056fea264697066735822122021e2c2b56b7e23b9555cc95390dfb2979a8526595038818d133d5bb772c01a6564736f6c634300081c0033")?;
                    let client = EthClient::new(&cfg.network.l2_rpc_url);

                    let (_, contract_address) = client
                        .deploy(
                            cfg.wallet.address,
                            cfg.wallet.private_key,
                            init_code.into(),
                            Overrides::default(),
                        )
                        .await?;

                    to_address = contract_address;

                    calldata::encode_calldata(
                        "fibonacci(uint256)",
                        &[Value::Uint(100000000000000_u64.into())],
                    )?
                    .into()
                } else {
                    Bytes::new()
                };

                println!("Sending to: {to_address:#x}");

                let now = Instant::now();
                let mut threads = vec![];
                for pk in lines.map_while(Result::ok) {
                    let thread = tokio::spawn(transfer_from(
                        pk,
                        to_address,
                        value,
                        iterations,
                        verbose,
                        calldata.clone(),
                        cfg.clone(),
                    ));
                    threads.push(thread);
                }

                let mut retries = 0;
                for thread in threads {
                    retries += thread.await?;
                }

                println!("Total retries: {retries}");
                println!("Total time elapsed: {:.2?}", now.elapsed());

                Ok(())
            }
            Command::ERC20 {
                transactions: transaction_count,
                path,
            } => {
                let contract_address = erc20_deploy(&cfg).await?;
                let private_keys: Result<Vec<_>, _> = read_lines(path)?
                    .into_iter()
                    .map(|pk| {
                        pk.unwrap()
                            .parse::<H256>()
                            .expect("One of the private keys is invalid")
                    })
                    .map(|pk| SecretKey::from_slice(pk.as_bytes()))
                    .collect();
                let private_keys = private_keys?;
                claim_erc20_balances(&cfg, contract_address, &private_keys).await?;
                erc20_load_test(&cfg, transaction_count, contract_address, &private_keys).await?;
                Ok(())
            }
        }
    }
}
