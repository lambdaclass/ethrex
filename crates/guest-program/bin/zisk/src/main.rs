#![no_main]

#[cfg(feature = "l2")]
use ethrex_guest_program::l2::{ProgramInput, execution_program};
#[cfg(not(feature = "l2"))]
use ethrex_guest_program::l1::{ProgramInput, execution_program};

use rkyv::rancor::Error;
use sha2::{Digest, Sha256};

ziskos::entrypoint!(main);

pub fn main() {
    println!("start reading input");
    let input = ziskos::read_input();

    // DEBUG: Hash of raw input bytes
    let input_hash = Sha256::digest(&input);
    println!("[ZISK DEBUG] Input bytes len: {}", input.len());
    println!("[ZISK DEBUG] Input SHA256: {:x}", input_hash);

    let input = rkyv::from_bytes::<ProgramInput, Error>(&input).unwrap();
    println!("finish reading input");

    println!("start execution");
    let output = execution_program(input).unwrap();
    println!("finish execution");

    println!("start hashing output");
    let encoded_output = output.encode();

    // DEBUG: Show the encoded output details
    println!("[ZISK DEBUG] ProgramOutput.encode() len: {}", encoded_output.len());
    let encoded_hash = Sha256::digest(&encoded_output);
    println!("[ZISK DEBUG] ProgramOutput.encode() SHA256: {:x}", encoded_hash);

    let output = Sha256::digest(&encoded_output);
    println!("[ZISK DEBUG] Output hash (what we commit): {:x}", output);
    println!("finish hashing output");

    println!("start revealing output");
    // ZisK stores set_output values in big-endian in the SNARK proof.
    // SHA256 outputs bytes in big-endian order, so we use from_be_bytes
    // to preserve the byte order for the on-chain verifier.
    output.chunks_exact(4).enumerate().for_each(|(idx, bytes)| {
        ziskos::set_output(idx, u32::from_be_bytes(bytes.try_into().unwrap()))
    });
    println!("finish revealing output");
}
