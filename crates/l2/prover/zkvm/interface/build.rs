fn main() {
    println!("cargo::rerun-if-changed=build.rs");

    #[cfg(not(clippy))]
    {
        #[cfg(any(feature = "risc0", feature = "sp1", feature = "pico"))]
        let features = if cfg!(feature = "l2") {
            vec!["l2".to_string()]
        } else {
            vec![]
        };

        #[cfg(feature = "risc0")]
        risc0_build::embed_methods_with_options(std::collections::HashMap::from([(
            "zkvm-risc0-program",
            risc0_build::GuestOptions {
                features: features.clone(),
                ..Default::default()
            },
        )]));

        // We should use include_elf! instead of doing this.
        // I'm leaving this to avoid complex changes.
        #[cfg(feature = "sp1")]
        sp1_build::build_program_with_args(
            "./sp1",
            sp1_build::BuildArgs {
                output_directory: Some("./sp1/elf".to_string()),
                elf_name: Some("riscv32im-succinct-zkvm-elf".to_string()),
                features: features.clone(),
                ..Default::default()
            },
        )

        if cfg!(feature = "pico") {
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

    }
}
