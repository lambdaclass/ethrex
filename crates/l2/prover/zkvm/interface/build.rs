fn main() {
    println!("cargo::rerun-if-changed=build.rs");

    #[cfg(not(clippy))]
    #[cfg(feature = "build_risc0")]
    risc0_build::embed_methods();

    #[cfg(not(clippy))]
    #[cfg(feature = "build_sp1")]
    sp1_build::build_program("./sp1");

    if cfg!(feature = "build_pico") {
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
