use std::process::Command;

fn main() {
    let contracts = ["Factorial", "Fibonacci", "ManyHashes"];
    println!("Current directory: {:?}", std::env::current_dir().unwrap());
    contracts.iter().for_each(|name| {
        compile_contract(&name);
    });
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
        .args(&["--bin", &path, "--overwrite", "-o", &outpath])
        .output()
        .expect("Failed to compile contract");

    let stdout = String::from_utf8(output.stdout).unwrap();
    let stderr = String::from_utf8(output.stderr).unwrap();
    println!("{}", stdout);
    println!("{}", stderr);
}
