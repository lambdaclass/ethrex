use crate::config::EthrexL2Config;
use bytes::Bytes;
use clap::Subcommand;
use ethereum_types::{Address, H256, U256};
use ethrex_blockchain::constants::TX_GAS_COST;
use ethrex_l2::utils::eth_client::{eth_sender::Overrides, EthClient};
use keccak_hash::keccak;
use secp256k1::SecretKey;
use std::{
    fs::File,
    io::{self, BufRead},
    path::Path,
    thread::sleep,
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
                Bytes::new(),
                Overrides {
                    chain_id: Some(cfg.network.l2_chain_id),
                    nonce: Some(i),
                    value: Some(value),
                    gas_price: Some(3121115334),
                    priority_gas_price: Some(3000000000),
                    gas_limit: Some(TX_GAS_COST),
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
    }

    retries
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
            } => {
                if let Ok(lines) = read_lines(path) {
                    let to_address = match to {
                        Some(address) => address,
                        None => Address::random(),
                    };
                    println!("Sending to: {to_address:#x}");

                    let mut threads = vec![];
                    for pk in lines.map_while(Result::ok) {
                        let thread = tokio::spawn(transfer_from(
                            pk,
                            to_address,
                            value,
                            iterations,
                            verbose,
                            cfg.clone(),
                        ));
                        threads.push(thread);
                    }

                    let mut retries = 0;
                    for thread in threads {
                        retries += thread.await?;
                    }

                    println!("Total retries: {retries}");
                }

                Ok(())
            }
        }
    }
}
