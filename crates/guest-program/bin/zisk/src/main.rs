#![no_main]

#[cfg(feature = "l2")]
use ethrex_guest_program::l2::{ProgramInput, execution_program};
#[cfg(not(any(feature = "l2", feature = "eip-8025")))]
use ethrex_guest_program::l1::{ProgramInput, execution_program};

use rkyv::rancor::Error;
use sha2::{Digest, Sha256};

ziskos::entrypoint!(main);

pub fn main() {
    #[cfg(feature = "eip-8025")]
    {
        eip8025_main();
    }
    #[cfg(not(feature = "eip-8025"))]
    {
        println!("start reading input");
        let input = ziskos::read_input();
        let input = rkyv::from_bytes::<ProgramInput, Error>(&input).unwrap();
        println!("finish reading input");

        println!("start execution");
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
}

#[cfg(feature = "eip-8025")]
fn eip8025_main() {
    use ethrex_guest_program::l1::{Eip8025ProgramInput, eip8025_execution_program};

    println!("start reading input");
    let input = ziskos::read_input();
    let input = rkyv::from_bytes::<Eip8025ProgramInput, Error>(&input).unwrap();
    println!("finish reading input");

    println!("start execution");
    let output = eip8025_execution_program(input);
    println!("finish execution");

    println!("start hashing output");
    let hashed = Sha256::digest(output.encode());
    println!("finish hashing output");

    println!("start revealing output");
    hashed.chunks_exact(4).enumerate().for_each(|(idx, bytes)| {
        ziskos::set_output(idx, u32::from_le_bytes(bytes.try_into().unwrap()))
    });
    println!("finish revealing output");
}
