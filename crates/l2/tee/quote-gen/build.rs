use std::{error::Error, fs};

// This build code is needed to add some env vars in order to construct the code version
// VERGEN_GIT_SHA to get the git commit hash
fn main() -> Result<(), Box<dyn Error>> {
    let sha = fs::read_to_string("git-revision").unwrap();
    println!("cargo:rustc-env=VERGEN_GIT_SHA={}", sha.trim());
    Ok(())
}
