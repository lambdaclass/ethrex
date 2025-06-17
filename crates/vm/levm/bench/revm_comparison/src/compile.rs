use std::fs;
use std::path::Path;
use std::process::Command;

fn main() {
    let contracts_dir = format!("{}/contracts", env!("CARGO_MANIFEST_DIR"));

    walk_and_compile(Path::new(&contracts_dir));
}

/// Recursively walks through the contracts directory and compiles all `.sol` files found.
/// It skips the `lib` directory to avoid compiling library contracts.
fn walk_and_compile(dir: &Path) {
    for entry in fs::read_dir(dir).unwrap() {
        let entry = entry.unwrap();
        let path = entry.path();

        if path.is_dir() && path.file_name().unwrap() != "lib" {
            walk_and_compile(&path);
        } else if let Some(ext) = path.extension() {
            println!("Found file: {}", path.display());
            if ext == "sol" {
                compile_contract(&path);
            }
        }
    }
}

/// Compiles a single Solidity contract file using `solc`.
/// The compiled binary will be placed in the `contracts/bin` directory.
fn compile_contract(sol_path: &Path) {
    let outpath = format!("{}/contracts/bin", env!("CARGO_MANIFEST_DIR"));
    let args = [
        "--bin-runtime",
        "--optimize",
        "--overwrite",
        sol_path.to_str().unwrap(),
        "--output-dir",
        &outpath,
    ];
    println!("compiling {}", sol_path.display());
    run_solc(&args);
}

/// Runs the Solidity compiler (`solc`) with the given arguments and prints the output.
fn run_solc(args: &[&str]) {
    let output = Command::new("solc")
        .args(args)
        .output()
        .expect("Failed to compile contract");

    let stdout = String::from_utf8(output.stdout).unwrap();
    let stderr = String::from_utf8(output.stderr).unwrap();
    println!("{}", stdout);
    println!("{}", stderr);
}
