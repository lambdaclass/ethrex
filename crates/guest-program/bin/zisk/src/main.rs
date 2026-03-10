#![no_main]

#[cfg(feature = "l2")]
use ethrex_guest_program::l2::{ProgramInput, execution_program};
#[cfg(all(not(feature = "l2"), not(feature = "eip-8025")))]
use ethrex_guest_program::l1::{ProgramInput, execution_program};
#[cfg(all(not(feature = "l2"), feature = "eip-8025"))]
use ethrex_guest_program::l1::{decode_eip8025, execution_program};

use sha2::{Digest, Sha256};

ziskos::entrypoint!(main);

pub fn main() {
    println!("start reading input");
    let input = ziskos::read_input();

    #[cfg(feature = "eip-8025")]
    let (new_payload_request, execution_witness) = decode_eip8025(&input).unwrap();
    #[cfg(not(feature = "eip-8025"))]
    let input = {
        use rkyv::rancor::Error;
        rkyv::from_bytes::<ProgramInput, Error>(&input).unwrap()
    };
    println!("finish reading input");

    println!("start execution");
    #[cfg(feature = "eip-8025")]
    let output = execution_program(new_payload_request, execution_witness).unwrap();
    #[cfg(not(feature = "eip-8025"))]
    let output = execution_program(input).unwrap();
    println!("finish execution");

    println!("start hashing output");
    let output = Sha256::digest(output.encode());
    println!("finish hashing output");

    println!("start revealing output");
    output.chunks_exact(4).enumerate().for_each(|(idx, bytes)| {
        ziskos::set_output(idx, u32::from_le_bytes(bytes.try_into().unwrap()))
    });
    println!("finish revealing output");
}
