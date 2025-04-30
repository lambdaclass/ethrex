use std::error::Error;
use vergen::{Emitter, RustcBuilder};

// This build code is needed to add some env vars in order to construct the node client version
// VERGEN_RUSTC_COMMIT_HASH to get the commit hash
// VERGEN_RUSTC_HOST_TRIPLE to get the build OS
// VERGEN_RUSTC_SEMVER to get the rustc version

fn main() -> Result<(), Box<dyn Error>> {
    let rustc = RustcBuilder::all_rustc()?;

    Emitter::default().add_instructions(&rustc)?.emit()?;
    Ok(())
}
