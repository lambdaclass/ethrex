use std::error::Error;
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
    let git2 = Git2Builder::default().branch(true).sha(false).build()?;

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
