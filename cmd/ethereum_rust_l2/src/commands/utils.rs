use std::str::FromStr;

use bytes::Bytes;
use clap::Subcommand;
use ethereum_types::{Address, H32, U256};
use eyre::eyre;
use keccak_hash::{keccak, H256};

#[derive(Subcommand)]
pub(crate) enum Command {
    #[clap(about = "Get ABI encode string from a function signature and arguments")]
    Calldata {
        #[clap(long)]
        signature: String,
        #[clap(long)]
        args: String,
        #[clap(long, required = false, default_value = "false")]
        only_args: bool,
    },
}

fn parse_signature(signature: &str) -> (String, Vec<String>) {
    let sig = signature.trim().trim_start_matches("function ");
    let (name, params) = sig.split_once('(').unwrap();
    let params: Vec<String> = params
        .trim_end_matches(')')
        .split(',')
        .map(|x| x.trim().split_once(' ').unzip().0.unwrap_or(x).to_string())
        .collect();
    (name.to_string(), params)
}

fn compute_function_selector(name: &str, params: Vec<String>) -> H32 {
    let normalized_signature = format!("{name}({})", params.join(","));
    let hash = keccak(normalized_signature.as_bytes());

    H32::from(&hash[..4].try_into().unwrap())
}

fn parse_arg(arg_type: &str, arg: &str) -> Vec<u8> {
    match arg_type {
        "address" => {
            let address = Address::from_str(arg).expect("Cannot parse address");
            H256::from(address).0.to_vec()
        }
        "uint8" => {
            let number = u8::from_str(arg).expect("Cannot parse number");
            H256::from_slice(&[number]).0.to_vec()
        }
        "uint256" => {
            let number = U256::from_dec_str(arg).expect("Cannot parse number");
            let buf = &mut [0u8; 32];
            number.to_big_endian(buf);
            buf.to_vec()
        }
        "bytes32" => {
            let bytes = H256::from_str(arg).expect("Cannot parse bytes32");
            bytes.0.to_vec()
        }
        "bytes" => {
            let _bytes =
                Bytes::from(hex::decode(arg.trim_start_matches("0x")).expect("Cannot parse bytes"));
            todo!();
        }
        _ => {
            panic!("Unsupported type: {arg_type}");
        }
    }
}

fn parse_vec_arg(arg_type: &str, arg: &str) -> Vec<u8> {
    let args = arg.split(',');
    let length = &mut [0u8; 32];
    U256::from(args.clone().count()).to_big_endian(length);
    let length = length.to_vec();

    match arg_type {
        "address[]" => {
            return [
                length,
                args.map(|arg| {
                    H256::from(Address::from_str(arg).expect("Cannot parse address[]"))
                        .0
                        .to_vec()
                })
                .collect::<Vec<Vec<u8>>>()
                .concat(),
            ]
            .concat();
        }
        "uint8[]" => {
            return [
                length,
                args.map(|arg| {
                    let buf: &mut [u8] = &mut [0u8; 32];
                    U256::from(u8::from_str(arg).expect("Cannot parse u8[]")).to_big_endian(buf);
                    buf.to_vec()
                })
                .collect::<Vec<Vec<u8>>>()
                .concat(),
            ]
            .concat();
        }
        "uint256[]" => {
            return [
                length,
                args.map(|arg| {
                    let buf: &mut [u8] = &mut [0u8; 32];
                    U256::from_dec_str(arg)
                        .expect("Cannot parse u256[]")
                        .to_big_endian(buf);
                    buf.to_vec()
                })
                .collect::<Vec<Vec<u8>>>()
                .concat(),
            ]
            .concat();
        }
        "bytes32[]" => {
            return [
                length,
                args.map(|arg| {
                    H256::from_str(arg)
                        .expect("Cannot parse bytes32[]")
                        .0
                        .to_vec()
                })
                .collect::<Vec<Vec<u8>>>()
                .concat(),
            ]
            .concat();
        }
        _ => {
            println!("Unsupported type: {arg_type}");
        }
    }
    vec![]
}

pub fn encode_calldata(
    signature: &str,
    args: &str,
    only_args: bool,
) -> Result<Vec<u8>, eyre::Error> {
    let (name, params) = parse_signature(signature);
    let function_selector = compute_function_selector(&name, params.clone());

    let args: Vec<&str> = args.split(' ').collect();

    if params.len() != args.len() {
        return Err(eyre!(
            "Number of arguments does not match ({} != {})",
            params.len(),
            args.len()
        ));
    }

    let mut calldata: Vec<u8> = vec![];
    let mut dynamic_calldata: Vec<u8> = vec![];
    if !only_args {
        calldata.extend(function_selector.as_bytes().to_vec());
    };
    for (param, arg) in params.iter().zip(args.clone()) {
        if param.as_str().ends_with("[]") {
            let offset: &mut [u8] = &mut [0u8; 32];
            (U256::from(args.len())
                .checked_mul(U256::from(32))
                .expect("Calldata too long")
                .checked_add(U256::from(dynamic_calldata.len()))
                .expect("Calldata too long"))
            .to_big_endian(offset);
            calldata.extend(offset.to_vec());
            dynamic_calldata.extend(parse_vec_arg(param, arg));
        } else {
            calldata.extend(parse_arg(param, arg));
        }
    }

    Ok([calldata, dynamic_calldata].concat())
}

impl Command {
    pub async fn run(self) -> eyre::Result<()> {
        match self {
            Command::Calldata {
                signature,
                args,
                only_args,
            } => {
                let calldata = encode_calldata(&signature, &args, only_args)?;
                println!("0x{}", hex::encode(calldata));
            }
        };
        Ok(())
    }
}
