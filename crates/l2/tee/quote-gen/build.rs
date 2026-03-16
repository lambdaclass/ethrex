use std::error::Error;
use vergen_git2::{Emitter, Git2Builder};

// This build code is needed to add some env vars in order to construct the code version
// VERGEN_GIT_SHA to get the git commit hash

fn main() -> Result<(), Box<dyn Error>> {
    // Export git commit hash as environment variable
    // When building tdx image with nix the commit version is stored as an env var
    if let Ok(sha) = std::env::var("VERGEN_GIT_SHA") {
        println!("cargo:rustc-env=VERGEN_GIT_SHA={}", sha.trim());
        return Ok(());
    }
    let git2 = Git2Builder::default().sha(true).build()?;

    Emitter::default().add_instructions(&git2)?.emit()?;
    Ok(())
}
