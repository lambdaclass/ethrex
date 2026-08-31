use std::sync::Arc;

#[cfg(feature = "l2")]
use ethrex_guest_program::l2::run_guest;
#[cfg(not(feature = "l2"))]
use ethrex_guest_program::l1::run_stateless_guest;

use ethrex_guest_program::crypto::openvm::OpenVmCrypto;
use openvm_keccak256::keccak256;

openvm::init!();

pub fn main() {
    openvm::io::println("start reading input");
    let input = openvm::io::read_vec();
    openvm::io::println("finish reading input");

    let crypto = Arc::new(OpenVmCrypto);

    openvm::io::println("start execution");
    #[cfg(not(feature = "l2"))]
    let output = run_stateless_guest(&input, crypto);
    #[cfg(feature = "l2")]
    let output = run_guest(&input, crypto).unwrap();
    openvm::io::println("finish execution");

    openvm::io::println("start revealing output");
    openvm::io::reveal_bytes32(keccak256(&output));
    openvm::io::println("finish revealing output");
}
