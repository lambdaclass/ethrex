fn main() {
    #[cfg(not(clippy))]
    #[cfg(feature = "build_risc0")]
    risc0_build::embed_methods();

    // We should use include_elf! instead of doing this.
    // I'm leaving this to avoid complex changes.
    #[cfg(not(clippy))]
    #[cfg(feature = "build_sp1")]
    sp1_build::build_program_with_args(
        "./sp1",
        sp1_build::BuildArgs {
            output_directory: Some("./sp1/elf".to_string()),
            elf_name: Some("riscv32im-succinct-zkvm-elf".to_string()),
            ..Default::default()
        },
    )
}
