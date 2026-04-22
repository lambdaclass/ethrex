#![no_main]

use std::sync::Arc;

#[cfg(feature = "l2")]
use ethrex_guest_program::l2::{ProgramInput, execution_program};
#[cfg(all(not(feature = "l2"), not(feature = "experimental-devnet")))]
use ethrex_guest_program::l1::{ProgramInput, execution_program};
#[cfg(all(not(feature = "l2"), feature = "experimental-devnet"))]
use ethrex_guest_program::l1::{decode_eip8025, execution_program};

use ethrex_guest_program::crypto::zisk::ZiskCrypto;
use rkyv::rancor::Error;

ziskos::entrypoint!(main);

pub fn main() {
    println!("start reading input");
    let input = ziskos::io::read_vec();

    #[cfg(feature = "experimental-devnet")]
    let (new_payload_request, execution_witness) = decode_eip8025(&input).unwrap();
    #[cfg(not(feature = "experimental-devnet"))]
    let input = { rkyv::from_bytes::<ProgramInput, Error>(&input).unwrap() };
    println!("finish reading input");

    let crypto = Arc::new(ZiskCrypto);

    println!("start execution");
    #[cfg(feature = "experimental-devnet")]
    let output = execution_program(new_payload_request, execution_witness, crypto).unwrap();
    #[cfg(not(feature = "experimental-devnet"))]
    let output = execution_program(input, crypto).unwrap();
    println!("finish execution");

    println!("start revealing output");
    ziskos::io::commit(&output.encode());
    println!("finish revealing output");
}
