use std::error::Error;
use vergen_git2::{Emitter, Git2Builder};

// This build code is needed to add some env vars in order to construct the code version
// VERGEN_GIT_SHA to get the git commit hash

fn main() -> Result<(), Box<dyn Error>> {
    // Export git commit hash and branch name as environment variables
    // When building tdx image with nix the commit version is stored as an env var
    if let Ok(sha) = std::env::var("VERGEN_GIT_SHA") {
        println!("cargo:rustc-env=VERGEN_GIT_SHA={}", sha.trim());
        return Ok(());
    }
    // Emit the full SHA, not the abbreviated form: it must match the
    // VERGEN_GIT_SHA build-arg (full github.sha) baked into the docker image so
    // a source-built component and a docker-built one agree on the commit hash
    // used for the prover version check. git's and libgit2's short-SHA lengths
    // diverge by repo size, so the abbreviated form can't be compared reliably.
    let git2 = Git2Builder::default().sha(false).build()?;

    Emitter::default().add_instructions(&git2)?.emit()?;
    Ok(())
}
