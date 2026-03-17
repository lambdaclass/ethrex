//! zkVM integration tests for guest program execution.
//!
//! These tests require a zkVM environment (SP1 or RISC0 toolchain installed)
//! and compiled ELF binaries.  They are `#[ignore]`-d by default and must be
//! run explicitly:
//!
//! ```sh
//! # SP1 tests (requires sp1 toolchain + compiled ELFs):
//! cargo test -p ethrex-prover --features sp1 -- --ignored sp1
//!
//! # RISC0 tests (requires risc0 toolchain + compiled ELFs):
//! cargo test -p ethrex-prover --features risc0 -- --ignored risc0
//! ```
//!
//! See `tokamak-notes/guest-program-modularization/05-improvements-proposal.md`
//! for CI setup instructions.

#[cfg(feature = "sp1")]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod sp1_tests {
    use ethrex_guest_program::programs::EvmL2GuestProgram;
    use ethrex_guest_program::traits::{GuestProgram, backends};

    #[test]
    #[ignore = "requires SP1 toolchain and compiled ELF"]
    fn sp1_evm_l2_elf_available() {
        let program = EvmL2GuestProgram;
        let elf = program.elf(backends::SP1);
        assert!(
            elf.is_some(),
            "SP1 ELF should be available when feature is enabled"
        );
        let elf = elf.unwrap();
        assert!(elf.len() > 20, "ELF should be a non-trivial binary");
        program
            .validate_elf(backends::SP1, elf)
            .expect("ELF should pass validation");
    }

    #[test]
    #[ignore = "requires SP1 toolchain and compiled ELF"]
    fn sp1_zk_dex_elf_available() {
        use ethrex_guest_program::programs::ZkDexGuestProgram;
        let program = ZkDexGuestProgram;
        let elf = program.elf(backends::SP1);
        assert!(elf.is_some(), "SP1 ZK-DEX ELF should be available");
        program
            .validate_elf(backends::SP1, elf.unwrap())
            .expect("ELF should pass validation");
    }

    #[test]
    #[ignore = "requires SP1 toolchain and compiled ELF"]
    fn sp1_tokamon_elf_available() {
        use ethrex_guest_program::programs::TokammonGuestProgram;
        let program = TokammonGuestProgram;
        let elf = program.elf(backends::SP1);
        assert!(elf.is_some(), "SP1 Tokamon ELF should be available");
        program
            .validate_elf(backends::SP1, elf.unwrap())
            .expect("ELF should pass validation");
    }
}

#[cfg(feature = "risc0")]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod risc0_tests {
    use ethrex_guest_program::programs::EvmL2GuestProgram;
    use ethrex_guest_program::traits::{GuestProgram, backends};

    #[test]
    #[ignore = "requires RISC0 toolchain and compiled ELF"]
    fn risc0_evm_l2_elf_available() {
        let program = EvmL2GuestProgram;
        let elf = program.elf(backends::RISC0);
        assert!(
            elf.is_some(),
            "RISC0 ELF should be available when feature is enabled"
        );
        let elf = elf.unwrap();
        assert!(elf.len() > 20, "ELF should be a non-trivial binary");
        program
            .validate_elf(backends::RISC0, elf)
            .expect("ELF should pass validation");
    }

    #[test]
    #[ignore = "requires RISC0 toolchain and compiled ELF"]
    fn risc0_evm_l2_vk_available() {
        let program = EvmL2GuestProgram;
        let vk = program.vk_bytes(backends::RISC0);
        assert!(
            vk.is_some(),
            "RISC0 VK should be available when feature is enabled"
        );
        assert!(!vk.unwrap().is_empty(), "VK bytes should not be empty");
    }
}
