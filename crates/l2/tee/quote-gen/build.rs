use std::{error::Error, fs};
use vergen_git2::{Emitter, Git2Builder};

// This build code is needed to add some env vars in order to construct the code version
// VERGEN_GIT_SHA to get the git commit hash

fn main() -> Result<(), Box<dyn Error>> {
    // Export git commit hash and branch name as environment variables
    if let Ok(sha) = fs::read_to_string("git-revision") {
        println!("cargo:rustc-env=VERGEN_GIT_SHA={}", sha.trim());
        return Ok(());
    }
    let git2 = Git2Builder::default().sha(true).build()?;

    Emitter::default().add_instructions(&git2)?.emit()?;
    Ok(())
}
