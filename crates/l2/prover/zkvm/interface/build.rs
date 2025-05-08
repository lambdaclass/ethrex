fn main() {
    println!("cargo::rerun-if-changed=build.rs");

    let features = if cfg!(feature = "l2") {
        vec!["l2".to_string()]
    } else {
        vec![]
    };

    #[cfg(feature = "pico")]
    build_pico_program();

    #[cfg(feature = "risc0")]
    build_risc0_program(&features);

    #[cfg(feature = "sp1")]
    build_sp1_program(&features);
}

#[cfg(feature = "pico")]
fn build_pico_program() {
    let output = std::process::Command::new("make")
        .output()
        .expect("failed to execute Makefile when building Pico ELF");

    if !output.status.success() {
        panic!(
            "Failed to build pico elf: {}",
            std::str::from_utf8(&output.stderr).unwrap()
        );
    }
}

#[cfg(feature = "risc0")]
fn build_risc0_program(features: &[String]) {
    risc0_build::embed_methods_with_options(std::collections::HashMap::from([(
        "zkvm-risc0-program",
        risc0_build::GuestOptions {
            features: features.to_vec(),
            ..Default::default()
        },
    )]));
}

#[cfg(feature = "sp1")]
fn build_sp1_program(features: &[String]) {
    use sp1_sdk::{HashableKey, ProverClient};

    sp1_build::build_program_with_args(
        "./sp1",
        sp1_build::BuildArgs {
            output_directory: Some("./sp1/elf".to_string()),
            elf_name: Some("riscv32im-succinct-zkvm-elf".to_string()),
            features: features.to_vec(),
            ..Default::default()
        },
    );

    // Get verification key
    // ref: https://github.com/succinctlabs/sp1/blob/dev/crates/cli/src/commands/vkey.rs
    let elf = std::fs::read("./sp1/out/riscv32im-succinct-zkvm-elf")
        .expect("could not read SP1 elf file");
    let prover = ProverClient::from_env();
    let (_, vk) = prover.setup(&elf);
    let vk = vk.vk.bytes32();
    dbg!(&vk);
    std::fs::write("./sp1/out/riscv32im-succinct-zkvm-vk", &vk)
        .expect("could not write SP1 vk to file");
}
