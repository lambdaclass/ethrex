fn main() {
    #[cfg(not(clippy))]
    #[cfg(feature = "build_sp1")]
    sp1_build::build_program("./sp1/zkvm");
}
