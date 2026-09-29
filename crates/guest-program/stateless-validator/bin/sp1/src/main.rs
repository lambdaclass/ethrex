//! SP1 Ethrex stateless validator guest program.

#![no_main]

use ere_platform_sp1::{SP1Platform, sp1_zkvm};
use ethrex_stateless_validator::platform::entrypoint;

sp1_zkvm::entrypoint!(main);

fn main() {
    entrypoint::<SP1Platform>();
}
