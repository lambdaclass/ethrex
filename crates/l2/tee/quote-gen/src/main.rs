use configfs_tsm::create_tdx_quote;
use serde::Serialize;
use keccak_hash::keccak;
use std::{thread, time::Duration};

#[derive(Serialize)]
struct BoundData {
    input: u64,
    output: u64
}

fn inc(n: u64) -> u64 {
    n + 1
}

fn run_inc(input: u64) -> BoundData {
    BoundData {
        input,
        output: inc(input)
    }
}

type GenericError = Result<(), Box<dyn std::error::Error>>;

#[tokio::main]
async fn main() -> GenericError {
    let mut state = 100;
    loop {
        let bound_data = run_inc(state);
        state = bound_data.output;
        let digest = keccak(serde_json::to_string(&bound_data)?);
        let mut digest_slice = [0u8; 64];
        digest_slice.split_at_mut(32).1.copy_from_slice(digest.as_bytes());
        let quote = create_tdx_quote(digest_slice).unwrap();
        println!("{} -> {}", bound_data.input, bound_data.output);
        println!("0x{:x?}", quote);
        thread::sleep(Duration::from_millis(5000));
    }
}
