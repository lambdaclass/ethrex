#[cfg(feature = "l2")]
use ethrex_guest_program::l2::{ProgramInput, execution_program};
#[cfg(not(any(feature = "l2", feature = "eip-8025")))]
use ethrex_guest_program::l1::{ProgramInput, execution_program};

use openvm_keccak256::keccak256;
use rkyv::rancor::Error;

openvm::init!();

pub fn main() {
    #[cfg(feature = "eip-8025")]
    {
        eip8025_main();
    }
    #[cfg(not(feature = "eip-8025"))]
    {
        openvm::io::println("start reading input");
        let input = openvm::io::read_vec();
        let input = rkyv::from_bytes::<ProgramInput, Error>(&input).unwrap();
        openvm::io::println("finish reading input");

        openvm::io::println("start execution");
        let output = execution_program(input).unwrap();
        openvm::io::println("finish execution");

        openvm::io::println("start hashing output");
        let output = keccak256(&output.encode());
        openvm::io::println("finish hashing output");

        openvm::io::println("start revealing output");
        openvm::io::reveal_bytes32(output);
        openvm::io::println("finish revealing output");
    }
}

#[cfg(feature = "eip-8025")]
fn eip8025_main() {
    use ethrex_guest_program::l1::{Eip8025ProgramInput, eip8025_execution_program};

    openvm::io::println("start reading input");
    let input = openvm::io::read_vec();
    let input = rkyv::from_bytes::<Eip8025ProgramInput, Error>(&input).unwrap();
    openvm::io::println("finish reading input");

    openvm::io::println("start execution");
    let output = eip8025_execution_program(input);
    openvm::io::println("finish execution");

    openvm::io::println("start hashing output");
    let hashed = keccak256(&output.encode());
    openvm::io::println("finish hashing output");

    openvm::io::println("start revealing output");
    openvm::io::reveal_bytes32(hashed);
    openvm::io::println("finish revealing output");
}
