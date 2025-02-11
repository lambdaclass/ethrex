fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    #[cfg(not(clippy))]
    #[cfg(feature = "build_risc0")]
    risc0_build::embed_methods();

    #[cfg(not(clippy))]
    #[cfg(feature = "build_sp1")]
    sp1_build::build_program("./sp1");

    if cfg!(feature = "build_pico") {
        use std::process::Command;
        Command::new("cargo")
            .arg("pico")
            .arg("build")
            .current_dir("./pico")
            .spawn()
            .expect("failed to build pico zkvm program");
    }
}
