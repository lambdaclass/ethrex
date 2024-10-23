fn main() {
    #[cfg(not(clippy))]
    #[cfg(feature = "build_zkvm")]
    risc0_build::embed_methods();
}
