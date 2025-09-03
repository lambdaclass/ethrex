use openvm_sdk::keygen::AppVerifyingKey;

fn main() {
    println!("cargo::rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=PROVER_CLIENT_ALIGNED");

    #[cfg(all(not(clippy), feature = "risc0"))]
    build_risc0_program();

    #[cfg(all(not(clippy), feature = "sp1"))]
    build_sp1_program();
}

#[cfg(all(not(clippy), feature = "risc0"))]
fn build_risc0_program() {
    use hex;
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
        .use_docker(docker_options)
        .build()
        .unwrap();

    let built_guests = embed_methods_with_options(std::collections::HashMap::from([(
        "zkvm-risc0-program",
        guest_options,
    )]));
    let image_id = built_guests[0].image_id;

    // this errs if the dir already exists, so we don't handle an error.
    let _ = std::fs::create_dir("./risc0/out");

    std::fs::write(
        "./risc0/out/riscv32im-risc0-vk",
        format!("0x{}\n", hex::encode(image_id.as_bytes())),
    )
    .expect("could not write Risc0 vk to file");
}

#[cfg(all(not(clippy), feature = "sp1"))]
fn build_sp1_program() {
    use hex;
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
            tag: "v5.0.8".to_string(),
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

    let aligned_mode = std::env::var("PROVER_CLIENT_ALIGNED").unwrap_or("false".to_string());

    if aligned_mode == "true" {
        let vk = vk.vk.hash_bytes();
        std::fs::write(
            "./sp1/out/riscv32im-succinct-zkvm-vk",
            format!("0x{}\n", hex::encode(vk)),
        )
        .expect("could not write SP1 vk to file");
    } else {
        let vk = vk.vk.bytes32();
        std::fs::write("./sp1/out/riscv32im-succinct-zkvm-vk", format!("{}\n", vk))
            .expect("could not write SP1 vk to file");
    };
}

fn build_openvm_program() {
    use openvm_build::GuestOptions;
    use openvm_sdk::Sdk;
    use serde::Serialize;

    let sdk = Sdk::standard();

    let guest_opts = GuestOptions::default();

    let target_path = "./openvm/out/riscv32im-openvm-elf";

    let _elf = sdk
        .build(guest_opts, target_path, &None, None)
        .expect("could not build OpenVM program");

    let (_app_pk, app_vk) = sdk.app_keygen();

    let vk = bincode::serialize(&app_vk).expect("could not serialize OpenVM vk");

    std::fs::write(
        "./openvm/out/riscv32im-openvm-vk",
        format!("0x{}\n", hex::encode(vk)),
    )
    .expect("could not write OpenVM vk to file");
}
