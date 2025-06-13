fn main() {
    println!("cargo::rerun-if-changed=build.rs");

    #[cfg(feature = "risc0")]
    build_risc0_program();

    #[cfg(feature = "sp1")]
    build_sp1_program();
}

#[cfg(feature = "risc0")]
fn build_risc0_program() {
    use risc0_build::{DockerOptionsBuilder, GuestOptionsBuilder, embed_methods_with_options};

    let features = if cfg!(feature = "l2") {
        vec!["l2".to_string()]
    } else {
        vec![]
    };

    let docker_options = DockerOptionsBuilder::default()
        .root_dir(format!("{}/../../../../../", env!("CARGO_MANIFEST_DIR")))
        .build()
        .unwrap();
    let guest_options = GuestOptionsBuilder::default()
        .features(features)
        //.use_docker(docker_options)
        .build()
        .unwrap();

    embed_methods_with_options(std::collections::HashMap::from([(
        "zkvm-risc0-program",
        guest_options,
    )]));
}

#[cfg(feature = "sp1")]
fn build_sp1_program() {
    use sp1_sdk::{HashableKey, ProverClient};

    let features = if cfg!(feature = "l2") {
        vec!["l2".to_string()]
    } else {
        vec![]
    };

    sp1_build::build_program_with_args(
        "./sp1",
        sp1_build::BuildArgs {
            output_directory: Some("./sp1/out".to_string()),
            elf_name: Some("riscv32im-succinct-zkvm-elf".to_string()),
            features,
            docker: true,
            workspace_directory: Some(format!("{}/../../../../../", env!("CARGO_MANIFEST_DIR"))),
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
