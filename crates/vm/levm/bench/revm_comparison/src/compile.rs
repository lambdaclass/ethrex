use std::fs;
use std::process::Command;

fn main() {
    let contracts = [
        "Factorial",
        "FactorialRecursive",
        "Fibonacci",
        "ManyHashes",
        "BubbleSort",
    ];
    println!("Current directory: {:?}", std::env::current_dir().unwrap());
    contracts.iter().for_each(|name| {
        compile_contract(name);
    });

    compile_erc20_contracts();
}

fn compile_contract(bench_name: &str) {
    let basepath = "crates/vm/levm/bench/revm_comparison/contracts";
    let outpath = format!("{}/bin", basepath);
    let path = format!("{}/{}.sol", basepath, bench_name);
    let args = [
        "--bin-runtime",
        "--optimize",
        "--overwrite",
        &path,
        "--output-dir",
        &outpath,
    ];
    println!("compiling {}", path);
    run_solc(&args);
}

fn compile_erc20_contracts() {
    let basepath = "crates/vm/levm/bench/revm_comparison/contracts/erc20";
    let libpath = format!("{}/lib", basepath);
    let outpath = "crates/vm/levm/bench/revm_comparison/contracts/bin";

    // Collect all `.sol` files from the `erc20` directory
    let paths = fs::read_dir(basepath)
        .expect("Failed to read erc20 directory")
        .filter_map(|entry| {
            let path = entry.ok()?.path();
            if path.extension()?.to_str()? == "sol" {
                Some(path.to_string_lossy().to_string())
            } else {
                None
            }
        })
        .collect::<Vec<String>>();

    let mut args = vec![
        "--bin-runtime",
        "--optimize",
        "--overwrite",
        "--allow-paths",
        &libpath,
        "--output-dir",
        &outpath,
    ];
    // Add the `.sol` files to the arguments
    args.extend(paths.iter().map(|s| s.as_str()));

    println!("compiling erc20 contracts: {:?}", args);
    run_solc(&args);
}

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
