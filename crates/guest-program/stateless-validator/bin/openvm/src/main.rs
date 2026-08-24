//! OpenVM Ethrex stateless validator guest program.

use ere_platform_openvm::OpenVMPlatform;
use ethrex_stateless_validator::platform::entrypoint;

openvm::init!();

fn main() {
    entrypoint::<OpenVMPlatform>();
}
