#![no_main]

use std::sync::Arc;

use ethrex_guest_program::l1::{execution_program, ProgramInput};

use ethrex_guest_program::crypto::airbender::AirbenderCrypto;
use rkyv::rancor::Error;
use sha2::{Digest, Sha256};

#[airbender::main]
fn main() -> [u32; 8] {
    println!("start reading input");
    let input_bytes: Vec<u8> = airbender::guest::read().expect("failed to read input");
    let input = rkyv::from_bytes::<ProgramInput, Error>(&input_bytes).unwrap();
    println!("finish reading input");

    let crypto = Arc::new(AirbenderCrypto);

    println!("start execution");
    let output = execution_program(input, crypto).unwrap();
    println!("finish execution");

    println!("start hashing output");
    let hash: [u8; 32] = Sha256::digest(output.encode()).into();
    println!("finish hashing output");

    // Convert [u8; 32] to [u32; 8] for Airbender's Commit trait
    let mut words = [0u32; 8];
    for (i, chunk) in hash.chunks_exact(4).enumerate() {
        words[i] = u32::from_le_bytes(chunk.try_into().unwrap());
    }
    words
}
