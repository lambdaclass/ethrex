fn main() {
    let features = if cfg!(feature = "l2") {
        vec!["l2".to_string()]
    } else {
        vec![]
    };

    #[cfg(not(clippy))]
    #[cfg(feature = "build_risc0")]
    risc0_build::embed_methods_with_options(std::collections::HashMap::from([(
        "zkvm-risc0-program",
        risc0_build::GuestOptions {
            features,
            ..Default::default()
        },
    )]));

    // We should use include_elf! instead of doing this.
    // I'm leaving this to avoid complex changes.
    #[cfg(not(clippy))]
    #[cfg(feature = "build_sp1")]
    sp1_build::build_program_with_args(
        "./sp1",
        sp1_build::BuildArgs {
            output_directory: Some("./sp1/elf".to_string()),
            elf_name: Some("riscv32im-succinct-zkvm-elf".to_string()),
            features,
            ..Default::default()
        },
    )
}
