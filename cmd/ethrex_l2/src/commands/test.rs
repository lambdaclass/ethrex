use crate::config::EthrexL2Config;
use bytes::Bytes;
use clap::Subcommand;
use ethereum_types::{Address, H256, U256};
use ethrex_blockchain::constants::TX_GAS_COST;
use ethrex_l2_sdk::calldata::{self, Value};
use ethrex_rpc::clients::eth::{eth_sender::Overrides, EthClient};
use ethrex_rpc::clients::EthClientError;
use eyre::bail;
use keccak_hash::keccak;
use secp256k1::SecretKey;
use std::time::Instant;
use std::{
    fs::File,
    io::{self, BufRead},
    path::Path,
    thread::sleep,
    time::Duration,
};

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
            long = "fibonacci",
            default_value = "false",
            help = "Run fibonacci load test"
        )]
        fibonacci: bool,
        #[clap(long = "io", default_value = "false", help = "Run I/O-heavy load test")]
        i_o_heavy: bool,
    },
}

fn read_lines<P>(filename: P) -> io::Result<io::Lines<io::BufReader<File>>>
where
    P: AsRef<Path>,
{
    let file = File::open(filename)?;
    Ok(io::BufReader::new(file).lines())
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
                    gas_limit: Some(TX_GAS_COST * 100),
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

impl Command {
    pub async fn run(self, cfg: EthrexL2Config) -> eyre::Result<()> {
        match self {
            Command::Load {
                path,
                to,
                value,
                iterations,
                verbose,
                fibonacci,
                i_o_heavy,
            } => {
                let lines = read_lines(path)?;

                if let Err(err) = test_connection(cfg.clone()).await {
                    bail!("Couldn't establish connection to L2: {err}")
                }

                let mut to_address = match to {
                    Some(address) => address,
                    None => Address::random(),
                };

                let calldata: Bytes = if fibonacci {
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
                } else if i_o_heavy {
                    // Contract with a function that touches 100 storage slots on every transaction.
                    // See `test_data/IOHeavyContract.sol` for the code.
                    let init_code = hex::decode("6080604052348015600e575f5ffd5b505f5f90505b6064811015603e57805f8260648110602d57602c6043565b5b018190555080806001019150506014565b506070565b7f4e487b71000000000000000000000000000000000000000000000000000000005f52603260045260245ffd5b6102728061007d5f395ff3fe608060405234801561000f575f5ffd5b506004361061003f575f3560e01c8063431aabc21461004357806358faa02f1461007357806362f8e72a1461007d575b5f5ffd5b61005d6004803603810190610058919061015c565b61009b565b60405161006a9190610196565b60405180910390f35b61007b6100b3565b005b61008561010a565b6040516100929190610196565b60405180910390f35b5f81606481106100a9575f80fd5b015f915090505481565b5f5f90505b60648110156101075760015f82606481106100d6576100d56101af565b5b01546100e29190610209565b5f82606481106100f5576100f46101af565b5b018190555080806001019150506100b8565b50565b5f5f5f6064811061011e5761011d6101af565b5b0154905090565b5f5ffd5b5f819050919050565b61013b81610129565b8114610145575f5ffd5b50565b5f8135905061015681610132565b92915050565b5f6020828403121561017157610170610125565b5b5f61017e84828501610148565b91505092915050565b61019081610129565b82525050565b5f6020820190506101a95f830184610187565b92915050565b7f4e487b71000000000000000000000000000000000000000000000000000000005f52603260045260245ffd5b7f4e487b71000000000000000000000000000000000000000000000000000000005f52601160045260245ffd5b5f61021382610129565b915061021e83610129565b9250828201905080821115610236576102356101dc565b5b9291505056fea264697066735822122055f6d7149afdb56c745a203d432710eaa25a8ccdb030503fb970bf1c964ac03264736f6c634300081b0033")?;
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

                    calldata::encode_calldata("incrementNumbers()", &[])?.into()
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
        }
    }
}
