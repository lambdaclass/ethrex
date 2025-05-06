use configfs_tsm::create_tdx_quote;
use keccak_hash::keccak;
use std::{thread, time::Duration};
use zerocopy::IntoBytes;

use eth_encode_packed::{SolidityDataType, TakeLastXBytes};
use eth_encode_packed::ethabi::ethereum_types::U256;
use eth_encode_packed::abi::encode_packed;

use alloy::signers::{local::PrivateKeySigner, Signer};

fn inc(n: u64) -> u64 {
    n + 1
}

fn run_inc(input: u64) -> (u64, Vec<u8>) {
    let output = inc(input);
    let data = vec![
        SolidityDataType::NumberWithShift(U256::from(input), TakeLastXBytes(64)),
        SolidityDataType::NumberWithShift(U256::from(output), TakeLastXBytes(64))
    ];
    let (bytes, _) = encode_packed(&data);
    (output, bytes)
}

type GenericError = Result<(), Box<dyn std::error::Error>>;

#[tokio::main]
async fn main() -> GenericError {
    let signer = PrivateKeySigner::random();
    println!("{:#?}", signer.address());
    
    let mut digest_slice = [0u8; 64];
    digest_slice.split_at_mut(20).0.copy_from_slice(signer.address().as_bytes());
    let quote = create_tdx_quote(digest_slice).unwrap();
    println!("0x{}", hex::encode(quote));
    
    let mut state = 100;
    loop {
        let (new_state, bound_data) = run_inc(state);
        let signature = signer.sign_message(&bound_data).await?;

        println!("{} -> {}", state, new_state);
        state = new_state;
        println!("{signature}");
        
        thread::sleep(Duration::from_millis(5000));
    }
}
