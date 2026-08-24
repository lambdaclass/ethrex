//! ZisK Ethrex stateless validator guest program.

#![no_main]

use ere_platform_zisk::{ZiskPlatform, ziskos};
use ethrex_stateless_validator::platform::entrypoint;

ziskos::entrypoint!(main);

fn main() {
    entrypoint::<ZiskPlatform>();
}
