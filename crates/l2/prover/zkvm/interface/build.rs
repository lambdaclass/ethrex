fn main() {
    #[cfg(not(clippy))]
    #[cfg(feature = "build_risc0")]
    risc0_build::embed_methods();

    #[cfg(not(clippy))]
    #[cfg(feature = "build_sp1")]
    sp1_build::build_program("./sp1");
}
