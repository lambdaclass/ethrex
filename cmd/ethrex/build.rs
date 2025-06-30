use std::error::Error;
use vergen_git2::{Emitter, Git2Builder, RustcBuilder};

// This build code is needed to add some env vars in order to construct the node client version
// VERGEN_RUSTC_HOST_TRIPLE to get the build OS
// VERGEN_RUSTC_SEMVER to get the rustc version
// VERGEN_GIT_BRANCH to get the git branch name
// VERGEN_GIT_SHA to get the git commit hash

fn main() -> Result<(), Box<dyn Error>> {
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
    Ok(())
}
