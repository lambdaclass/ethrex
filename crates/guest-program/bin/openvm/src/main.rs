use std::sync::Arc;

#[cfg(feature = "l2")]
use ethrex_guest_program::l2::{ProgramInput, execution_program};
#[cfg(all(
    not(feature = "l2"),
    not(feature = "eip-8025"),
    not(feature = "stateless-validator")
))]
use ethrex_guest_program::l1::{ProgramInput, execution_program};
#[cfg(all(
    not(feature = "l2"),
    feature = "eip-8025",
    not(feature = "stateless-validator")
))]
use ethrex_guest_program::l1::execution_program;

use ethrex_guest_program::crypto::openvm::OpenVmCrypto;
#[cfg(not(feature = "stateless-validator"))]
use openvm_keccak256::keccak256;

/// Maximum bytes of output the guest may reveal (mirrors ere's OpenVM
/// platform contract; output is zero-padded to whole words).
#[cfg(feature = "stateless-validator")]
const MAX_OUTPUT_BYTES: usize = 256;

openvm::init!();

pub fn main() {
    openvm::io::println("start reading input");
    let input = openvm::io::read_vec();

    #[cfg(feature = "stateless-validator")]
    {
        openvm::io::println("finish reading input");
        openvm::io::println("start execution");
        let output =
            ethrex_stateless_validator::run_stateless_validation(&input, Arc::new(OpenVmCrypto));
        openvm::io::println("finish execution");

        openvm::io::println("start revealing output");
        assert!(
            output.len() <= MAX_OUTPUT_BYTES,
            "Maximum output size is {MAX_OUTPUT_BYTES} bytes"
        );
        for (index, chunk) in output.chunks(4).enumerate() {
            let mut word = [0u8; 4];
            word[..chunk.len()].copy_from_slice(chunk);
            openvm::io::reveal_u32(u32::from_le_bytes(word), index);
        }
        openvm::io::println("finish revealing output");
    }

    #[cfg(not(feature = "stateless-validator"))]
    {
        #[cfg(not(feature = "eip-8025"))]
        let input = {
            use rkyv::rancor::Error;
            rkyv::from_bytes::<ProgramInput, Error>(&input).unwrap()
        };
        openvm::io::println("finish reading input");

        let crypto = Arc::new(OpenVmCrypto);

        openvm::io::println("start execution");
        #[cfg(feature = "eip-8025")]
        let output = execution_program(&input, crypto).unwrap();
        #[cfg(not(feature = "eip-8025"))]
        let output = execution_program(input, crypto).unwrap();
        openvm::io::println("finish execution");

        openvm::io::println("start hashing output");
        let output = keccak256(&output.encode());
        openvm::io::println("finish hashing output");

        openvm::io::println("start revealing output");
        openvm::io::reveal_bytes32(output);
        openvm::io::println("finish revealing output");
    }
}
