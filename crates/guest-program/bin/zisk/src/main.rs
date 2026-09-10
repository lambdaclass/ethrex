#![no_main]

use std::sync::Arc;

#[cfg(feature = "l2")]
use ethrex_guest_program::l2::run_guest;
#[cfg(not(feature = "l2"))]
use ethrex_guest_program::l1::run_stateless_guest;

use ethrex_guest_program::crypto::zisk::ZiskCrypto;

ziskos::entrypoint!(main);

pub fn main() {
    println!("start reading input");
    let input = ziskos::io::read_slice();
    println!("finish reading input");

    let crypto = Arc::new(ZiskCrypto);

    println!("start execution");
    #[cfg(not(feature = "l2"))]
    let output = run_stateless_guest(&input, crypto);
    #[cfg(feature = "l2")]
    let output = run_guest(&input, crypto).unwrap();
    println!("finish execution");

    println!("start revealing output");
    ziskos::io::commit_slice(&output);
    println!("finish revealing output");
}
