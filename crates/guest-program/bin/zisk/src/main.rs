#![no_main]

use std::sync::Arc;

#[cfg(feature = "l2")]
use ethrex_guest_program::crypto::Crypto;
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
    // ZisK exposes 64 32-bit public slots (256 bytes). The encoded L2 output is
    // 256 bytes of fixed fields plus the batch's balance diffs and L2 message
    // rolling hashes, so the guest commits its sha256 digest: the
    // OnChainProposer rebuilds the same bytes from the batch commitment, hashes
    // them and checks the digest against the first eight public slots.
    #[cfg(feature = "l2")]
    let output = crypto.sha256(&run_guest(&input, crypto.clone()).unwrap());
    println!("finish execution");

    println!("start revealing output");
    ziskos::io::commit_slice(&output);
    println!("finish revealing output");
}
