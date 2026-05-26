#![no_main]

use std::sync::Arc;

#[cfg(feature = "l2")]
use ethrex_guest_program::l2::{ProgramInput, execution_program};
#[cfg(not(feature = "l2"))]
use ethrex_guest_program::l1::execution_program;

use ethrex_guest_program::crypto::zisk::ZiskCrypto;
#[cfg(feature = "l2")]
use rkyv::rancor::Error;

ziskos::entrypoint!(main);

pub fn main() {
    println!("start reading input");
    let input = ziskos::io::read_vec();

    #[cfg(feature = "l2")]
    let input = { rkyv::from_bytes::<ProgramInput, Error>(&input).unwrap() };
    println!("finish reading input");

    let crypto = Arc::new(ZiskCrypto);

    println!("start execution");
    #[cfg(feature = "l2")]
    let output = execution_program(input, crypto).unwrap();
    #[cfg(not(feature = "l2"))]
    let output = execution_program(&input, crypto).unwrap();
    println!("finish execution");

    println!("start revealing output");
    ziskos::io::commit(&output.encode());
    println!("finish revealing output");
}
