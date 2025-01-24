use std::fs;
use std::process::Command;

fn main() {
    let contracts = ["Factorial", "Fibonacci", "ManyHashes"];
    println!("Current directory: {:?}", std::env::current_dir().unwrap());
    contracts.iter().for_each(|name| {
        compile_contract(&name);
    });

    compile_erc20_contracts();
}

fn compile_contract(bench_name: &str) {
    let path = format!(
        "crates/vm/levm/bench/revm_comparison/contracts/{}.sol",
        bench_name
    );
    let outpath = format!(
        "crates/vm/levm/bench/revm_comparison/contracts/{}",
        bench_name
    );
    println!("compiling {}", path);
    let output = Command::new("solc")
        .args(&["--bin-runtime", &path, "--overwrite", "-o", &outpath])
        .output()
        .expect("Failed to compile contract");

    let stdout = String::from_utf8(output.stdout).unwrap();
    let stderr = String::from_utf8(output.stderr).unwrap();
    println!("{}", stdout);
    println!("{}", stderr);
}

fn compile_erc20_contracts() {
    let basepath = "crates/vm/levm/bench/revm_comparison/contracts/erc20";
    let libpath = format!("{}/lib", basepath);
    let outpath = format!("{}/bin", basepath);

    // Collect all `.sol` files from the `erc20` directory
    let paths = fs::read_dir(&basepath)
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

    // Prepare solc arguments
    let mut args = vec![
        "--bin-runtime", // Generate binaries
        "--optimize",    // Enable optimization
        "--overwrite",   // Overwrite existing files
        "--allow-paths", // Allow resolving imports from specified paths
        &libpath,
        "--output-dir", // Specify the output directory
        &outpath,
    ];

    // Add the `.sol` files to the arguments
    args.extend(paths.iter().map(|s| s.as_str()));

    println!("compiling erc20 contracts: {:?}", args);

    // Execute the `solc` command
    let output = Command::new("solc")
        .args(&args)
        .output()
        .expect("Failed to compile contracts");

    let stdout = String::from_utf8(output.stdout).unwrap();
    let stderr = String::from_utf8(output.stderr).unwrap();
    println!("{}", stdout);
    println!("{}", stderr);
}
