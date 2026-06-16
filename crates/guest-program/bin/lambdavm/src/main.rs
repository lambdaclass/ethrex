use std::sync::Arc;

#[cfg(feature = "l2")]
use ethrex_guest_program::l2::{ProgramInput, execution_program};
#[cfg(all(not(feature = "l2"), not(feature = "eip-8025")))]
use ethrex_guest_program::l1::{ProgramInput, execution_program};
#[cfg(all(not(feature = "l2"), feature = "eip-8025"))]
use ethrex_guest_program::l1::execution_program;

use ethrex_guest_program::crypto::lambdavm::LambdaVmCrypto;
#[cfg(not(feature = "eip-8025"))]
use rkyv::rancor::Error;

pub fn main() {
    let input = lambda_vm_syscalls::syscalls::get_private_input();

    #[cfg(not(feature = "eip-8025"))]
    let input = { rkyv::from_bytes::<ProgramInput, Error>(&input).unwrap() };

    let crypto = Arc::new(LambdaVmCrypto);

    #[cfg(feature = "eip-8025")]
    let output = execution_program(&input, crypto).unwrap();
    #[cfg(not(feature = "eip-8025"))]
    let output = execution_program(input, crypto).unwrap();

    lambda_vm_syscalls::syscalls::commit(&output.encode());
}
