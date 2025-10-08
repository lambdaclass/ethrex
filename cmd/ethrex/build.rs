use ethrex_common::types::Genesis;
use ethrex_common::{Address, H160};
use std::error::Error;
use std::fs::File;
use std::io::BufReader;
use vergen_git2::{Emitter, Git2Builder, RustcBuilder};
#[cfg(feature = "l2")]
mod build_l2;
// This build code is needed to add some env vars in order to construct the node client version
// VERGEN_RUSTC_HOST_TRIPLE to get the build OS
// VERGEN_RUSTC_SEMVER to get the rustc version
// VERGEN_GIT_BRANCH to get the git branch name
// VERGEN_GIT_SHA to get the git commit hash

// This script downloads dependencies and compiles contracts to be embedded as constants in the deployer.

fn main() -> Result<(), Box<dyn Error>> {
    println!("cargo::rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=COMPILE_CONTRACTS");
    println!("cargo:rerun-if-changed=../../crates/l2/contracts/src");

    // Export build OS and rustc version as environment variables
    let rustc = RustcBuilder::default()
        .semver(true)
        .host_triple(true)
        .build()?;

    // Export git commit hash and branch name as environment variables
    let git2 = Git2Builder::default().branch(true).sha(true).build()?;

    Emitter::default()
        .add_instructions(&rustc)?
        .add_instructions(&git2)?
        .emit()?;

    #[cfg(feature = "l2")]
    {
        use build_l2::download_script;
        use std::env;
        use std::path::Path;

        use crate::build_l2::{L2_GENESIS_PATH, update_genesis_file};

        download_script();

        // If COMPILE_CONTRACTS is not set, skip
        if env::var_os("COMPILE_CONTRACTS").is_some() {
            let out_dir = env::var_os("OUT_DIR").unwrap();
            update_genesis_file(L2_GENESIS_PATH.as_ref(), Path::new(&out_dir))?;
        }
    }

    Ok(())
}

#[allow(clippy::enum_variant_names)]
#[derive(Debug, thiserror::Error)]
pub enum SystemContractsUpdaterError {
    #[error("Failed to deploy contract: {0}")]
    FailedToDecodeRuntimeCode(#[from] hex::FromHexError),
    #[error("Failed to serialize modified genesis: {0}")]
    FailedToSerializeModifiedGenesis(#[from] serde_json::Error),
    #[error("Failed to write modified genesis file: {0}")]
    FailedToWriteModifiedGenesisFile(#[from] std::io::Error),
    #[error("Failed to read path: {0}")]
    InvalidPath(String),
    #[error(
        "Contract bytecode not found. Make sure to compile the updater with `COMPILE_CONTRACTS` set."
    )]
    BytecodeNotFound,
}

/// Address authorized to perform system contract upgrades
/// 0x000000000000000000000000000000000000f000
pub const ADMIN_ADDRESS: Address = H160([
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0xf0, 0x00,
]);

/// Mask used to derive the initial implementation address
/// 0x0000000000000000000000000000000000001000
pub const IMPL_MASK: Address = H160([
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x10, 0x00,
]);
// From cmd/ethrex
pub fn read_genesis_file(genesis_file_path: &str) -> Genesis {
    let genesis_file = std::fs::File::open(genesis_file_path).expect("Failed to open genesis file");
    _genesis_file(genesis_file).expect("Failed to decode genesis file")
}

// From cmd/ethrex/decode.rs
fn _genesis_file(file: File) -> Result<Genesis, serde_json::Error> {
    let genesis_reader = BufReader::new(file);
    serde_json::from_reader(genesis_reader)
}
