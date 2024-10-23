pub mod methods {
    #[cfg(any(clippy, not(feature = "build_zkvm")))]
    pub const ZKVM_PROGRAM_ELF: &[u8] = &[0];
    #[cfg(any(clippy, not(feature = "build_zkvm")))]
    pub const ZKVM_PROGRAM_ID: [u32; 8] = [0_u32; 8];

    #[cfg(all(not(clippy), feature = "build_zkvm"))]
    include!(concat!(env!("OUT_DIR"), "/methods.rs"));
}
