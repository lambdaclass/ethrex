#![no_main]

#[cfg(not(feature = "l2"))]
use ethrex_guest_program::l1::{ProgramInput, execution_program};
#[cfg(feature = "l2")]
use ethrex_guest_program::l2::{ProgramInput, execution_program};

use rkyv::rancor::Error;
use sha2::{Digest, Sha256};

ziskos::entrypoint!(main);

pub fn main() {
    let input = ziskos::read_input();
    let input = rkyv::from_bytes::<ProgramInput, Error>(&input).unwrap();

    let output = execution_program(input).unwrap();

    let encoded_output = output.encode();
    let output = Sha256::digest(&encoded_output);

    // Output the sha256 hash as 8 little-endian u32 values
    output.chunks_exact(4).enumerate().for_each(|(idx, bytes)| {
        ziskos::set_output(idx, u32::from_le_bytes(bytes.try_into().unwrap()))
    });
}
