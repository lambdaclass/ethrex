/// Well-known backend identifiers used with [`GuestProgram::elf`] and
/// [`GuestProgram::vk_bytes`].
///
/// These string constants allow the `GuestProgram` trait to identify zkVM
/// backends without depending on the `BackendType` enum (which lives in the
/// `ethrex-prover` crate and would create a circular dependency).
pub mod backends {
    pub const SP1: &str = "sp1";
    pub const RISC0: &str = "risc0";
    pub const ZISK: &str = "zisk";
    pub const OPENVM: &str = "openvm";
    pub const EXEC: &str = "exec";
}

/// Error type for guest program operations.
#[derive(Debug, thiserror::Error)]
pub enum GuestProgramError {
    #[error("Serialization error: {0}")]
    Serialization(String),

    #[error("Unsupported backend: {0}")]
    UnsupportedBackend(String),

    #[error("Invalid ELF: {0}")]
    InvalidElf(String),

    #[error("Internal error: {0}")]
    Internal(String),
}

/// Resource limits for a guest program execution.
///
/// These limits protect the prover from unbounded resource consumption due to
/// malicious or buggy inputs.  Each guest program can override the defaults to
/// set program-specific constraints.
#[derive(Debug, Clone, Default)]
pub struct ResourceLimits {
    /// Maximum serialized input size in bytes.  `None` means unlimited.
    pub max_input_bytes: Option<usize>,
    /// Maximum wall-clock time allowed for proving.  `None` means unlimited.
    pub max_proving_duration: Option<std::time::Duration>,
}

/// Trait that abstracts a guest program running inside a zkVM.
///
/// Each guest program is a self-contained program compiled to a RISC-V ELF
/// binary.  The prover executes this ELF inside a zkVM (SP1, RISC0, …) and
/// produces a proof.  Different guest programs can implement different
/// validation logic (EVM execution, simple transfers, DEX order matching, etc.)
/// while sharing the same prover infrastructure.
///
/// # Design choices
///
/// The trait operates at the **bytes level** to avoid generic type proliferation
/// through `ProverBackend`.  Each guest program keeps its own strongly-typed
/// `Input`/`Output` types internally, but exposes only `&[u8]` through this
/// trait.
///
/// Backend identification uses `&str` constants (see [`backends`]) rather than
/// an enum to avoid a circular dependency between the `guest-program` and
/// `prover` crates.
///
/// # Object safety
///
/// This trait is object-safe so that [`std::sync::Arc<dyn GuestProgram>`] can
/// be stored in a runtime registry.
pub trait GuestProgram: Send + Sync {
    /// Unique identifier for this guest program (e.g. `"evm-l2"`, `"transfer"`).
    fn program_id(&self) -> &str;

    /// Compiled ELF binary for a given zkVM backend.
    ///
    /// Returns `None` when the requested backend is not supported or the ELF
    /// has not been compiled (e.g. the corresponding feature flag is disabled).
    ///
    /// `backend` should be one of the constants in [`backends`].
    fn elf(&self, backend: &str) -> Option<&[u8]>;

    /// Verification key bytes for a given zkVM backend.
    ///
    /// The exact format depends on the backend:
    /// - SP1: 32-byte `vk.bytes32()` hash
    /// - RISC0: hex-encoded image ID
    ///
    /// Returns `None` when the VK is not available (e.g. SP1 generates VKs at
    /// setup time from the ELF, so a compile-time VK may not exist).
    fn vk_bytes(&self, backend: &str) -> Option<Vec<u8>>;

    /// Integer identifier for this program type on L1.
    ///
    /// Used as the `programTypeId` key in the on-chain `verificationKeys`
    /// mapping: `verificationKeys[commitHash][programTypeId][verifierId]`.
    fn program_type_id(&self) -> u8;

    /// Serialize raw input data into the bytes the guest program expects.
    ///
    /// The default implementation is the identity (pass-through), which is
    /// correct when the caller already supplies bytes in the format the guest
    /// program reads from the zkVM stdin.
    fn serialize_input(&self, raw_input: &[u8]) -> Result<Vec<u8>, GuestProgramError> {
        Ok(raw_input.to_vec())
    }

    /// Encode the zkVM's raw public-values output into the byte layout
    /// expected by the L1 verifier contract.
    ///
    /// The default implementation is the identity (pass-through), which is
    /// correct when the zkVM output already matches the L1 encoding.
    fn encode_output(&self, raw_output: &[u8]) -> Result<Vec<u8>, GuestProgramError> {
        Ok(raw_output.to_vec())
    }

    /// Resource limits for this guest program.
    ///
    /// The prover checks these limits before and after proving to prevent
    /// denial-of-service from oversized inputs or runaway executions.
    /// The default is unlimited (no restrictions).
    fn resource_limits(&self) -> ResourceLimits {
        ResourceLimits::default()
    }

    /// Semantic version string for this guest program.
    ///
    /// Used for ELF change tracking and VK cache invalidation.
    /// The default is `"0.0.0"` (unversioned).
    fn version(&self) -> &str {
        "0.0.0"
    }

    /// SHA-256 hash of the ELF binary for a given backend.
    ///
    /// Returns `None` when no ELF is available for the backend.
    /// This is useful for VK cache invalidation: when the hash changes,
    /// the verification key must be regenerated.
    fn elf_hash(&self, backend: &str) -> Option<[u8; 32]> {
        use sha2::{Digest, Sha256};
        self.elf(backend).map(|elf| {
            let mut hasher = Sha256::new();
            hasher.update(elf);
            hasher.finalize().into()
        })
    }

    /// Validate that an ELF binary has the correct format for the given backend.
    ///
    /// Checks the ELF magic number, class (32 or 64-bit), and machine type
    /// (RISC-V).  This catches architecture mismatches early, before the
    /// backend attempts to load or prove the ELF.
    ///
    /// The default implementation performs generic ELF header validation.
    /// Override this to add backend- or program-specific checks.
    fn validate_elf(&self, backend: &str, elf: &[u8]) -> Result<(), GuestProgramError> {
        validate_elf_header(backend, elf)
    }
}

// ── ELF header constants ─────────────────────────────────────────────

const ELF_MAGIC: [u8; 4] = [0x7f, b'E', b'L', b'F'];
const ELFCLASS32: u8 = 1;
const ELFCLASS64: u8 = 2;
const EM_RISCV: u16 = 243;

/// Minimum ELF header size (e_ident[16] + e_type[2] + e_machine[2] = 20 bytes).
const ELF_HEADER_MIN: usize = 20;

/// Validate basic ELF header structure for a zkVM guest binary.
///
/// Checks:
/// 1. ELF magic number (`\x7fELF`)
/// 2. ELF class matches backend expectations (32-bit or 64-bit)
/// 3. Machine type is RISC-V (`EM_RISCV = 243`)
pub fn validate_elf_header(backend: &str, elf: &[u8]) -> Result<(), GuestProgramError> {
    if elf.len() < ELF_HEADER_MIN {
        return Err(GuestProgramError::InvalidElf(format!(
            "ELF too short: {} bytes (minimum {})",
            elf.len(),
            ELF_HEADER_MIN
        )));
    }

    // Check magic number (e_ident[0..4]).
    if elf[0..4] != ELF_MAGIC {
        return Err(GuestProgramError::InvalidElf(
            "invalid ELF magic number".to_string(),
        ));
    }

    // Check ELF class (e_ident[4]).
    let elf_class = elf[4];
    let expected_class = match backend {
        // ZisK uses 64-bit RISC-V.
        backends::ZISK => ELFCLASS64,
        // SP1, RISC0, OpenVM use 32-bit RISC-V.
        backends::SP1 | backends::RISC0 | backends::OPENVM => ELFCLASS32,
        // Unknown backends: accept any class.
        _ => elf_class,
    };
    if elf_class != expected_class {
        let class_name = |c: u8| match c {
            1 => "32-bit",
            2 => "64-bit",
            _ => "unknown",
        };
        return Err(GuestProgramError::InvalidElf(format!(
            "ELF class mismatch for {}: expected {} ({}), got {} ({})",
            backend,
            expected_class,
            class_name(expected_class),
            elf_class,
            class_name(elf_class),
        )));
    }

    // Check machine type (e_machine at offset 18, little-endian u16).
    let e_machine = u16::from_le_bytes([elf[18], elf[19]]);
    if e_machine != EM_RISCV {
        return Err(GuestProgramError::InvalidElf(format!(
            "ELF machine type is {} (expected RISC-V = {})",
            e_machine, EM_RISCV
        )));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a minimal valid RISC-V ELF header.
    fn make_elf(class: u8, machine: u16) -> Vec<u8> {
        let mut buf = vec![0u8; 20];
        // Magic.
        buf[0..4].copy_from_slice(&ELF_MAGIC);
        // EI_CLASS.
        buf[4] = class;
        // e_machine at offset 18 (little-endian).
        buf[18..20].copy_from_slice(&machine.to_le_bytes());
        buf
    }

    #[test]
    fn valid_riscv32_elf_for_sp1() {
        let elf = make_elf(ELFCLASS32, EM_RISCV);
        assert!(validate_elf_header(backends::SP1, &elf).is_ok());
    }

    #[test]
    fn valid_riscv64_elf_for_zisk() {
        let elf = make_elf(ELFCLASS64, EM_RISCV);
        assert!(validate_elf_header(backends::ZISK, &elf).is_ok());
    }

    #[test]
    fn rejects_too_short_elf() {
        let elf = vec![0x7f, b'E', b'L', b'F'];
        let err = validate_elf_header(backends::SP1, &elf).unwrap_err();
        assert!(matches!(err, GuestProgramError::InvalidElf(_)));
    }

    #[test]
    fn rejects_bad_magic() {
        let mut elf = make_elf(ELFCLASS32, EM_RISCV);
        elf[0] = 0x00; // corrupt magic
        let err = validate_elf_header(backends::SP1, &elf).unwrap_err();
        assert!(
            format!("{err}").contains("magic"),
            "error should mention magic: {err}"
        );
    }

    #[test]
    fn rejects_wrong_class_for_sp1() {
        // SP1 expects 32-bit, give it 64-bit.
        let elf = make_elf(ELFCLASS64, EM_RISCV);
        let err = validate_elf_header(backends::SP1, &elf).unwrap_err();
        assert!(
            format!("{err}").contains("class mismatch"),
            "error should mention class mismatch: {err}"
        );
    }

    #[test]
    fn rejects_wrong_class_for_zisk() {
        // ZisK expects 64-bit, give it 32-bit.
        let elf = make_elf(ELFCLASS32, EM_RISCV);
        let err = validate_elf_header(backends::ZISK, &elf).unwrap_err();
        assert!(format!("{err}").contains("class mismatch"));
    }

    #[test]
    fn rejects_wrong_machine_type() {
        // Machine type 0x03 = EM_386 (x86).
        let elf = make_elf(ELFCLASS32, 0x03);
        let err = validate_elf_header(backends::SP1, &elf).unwrap_err();
        assert!(format!("{err}").contains("machine type"));
    }

    #[test]
    fn unknown_backend_accepts_any_class() {
        let elf32 = make_elf(ELFCLASS32, EM_RISCV);
        let elf64 = make_elf(ELFCLASS64, EM_RISCV);
        assert!(validate_elf_header("custom", &elf32).is_ok());
        assert!(validate_elf_header("custom", &elf64).is_ok());
    }

    #[test]
    fn validate_elf_via_trait_default() {
        struct TestProgram;
        impl GuestProgram for TestProgram {
            fn program_id(&self) -> &str {
                "test"
            }
            fn elf(&self, _: &str) -> Option<&[u8]> {
                None
            }
            fn vk_bytes(&self, _: &str) -> Option<Vec<u8>> {
                None
            }
            fn program_type_id(&self) -> u8 {
                99
            }
        }

        let prog = TestProgram;
        let elf = make_elf(ELFCLASS32, EM_RISCV);
        assert!(prog.validate_elf(backends::SP1, &elf).is_ok());

        let bad = vec![0u8; 4];
        assert!(prog.validate_elf(backends::SP1, &bad).is_err());
    }

    // ── Resource limits tests ────────────────────────────────────────

    #[test]
    fn default_limits_are_unlimited() {
        let limits = ResourceLimits::default();
        assert!(limits.max_input_bytes.is_none());
        assert!(limits.max_proving_duration.is_none());
    }

    #[test]
    fn stub_uses_default_limits() {
        struct StubProgram;
        impl GuestProgram for StubProgram {
            fn program_id(&self) -> &str {
                "stub"
            }
            fn elf(&self, _: &str) -> Option<&[u8]> {
                None
            }
            fn vk_bytes(&self, _: &str) -> Option<Vec<u8>> {
                None
            }
            fn program_type_id(&self) -> u8 {
                99
            }
        }
        let limits = StubProgram.resource_limits();
        assert!(limits.max_input_bytes.is_none());
        assert!(limits.max_proving_duration.is_none());
    }

    // ── Fuzz-style robustness tests ──────────────────────────────────

    use crate::programs::{EvmL2GuestProgram, TokammonGuestProgram, ZkDexGuestProgram};

    // ── Versioning tests ───────────────────────────────────────────

    #[test]
    fn version_default_is_0_0_0() {
        struct StubV;
        impl GuestProgram for StubV {
            fn program_id(&self) -> &str {
                "stub-v"
            }
            fn elf(&self, _: &str) -> Option<&[u8]> {
                None
            }
            fn vk_bytes(&self, _: &str) -> Option<Vec<u8>> {
                None
            }
            fn program_type_id(&self) -> u8 {
                99
            }
        }
        assert_eq!(StubV.version(), "0.0.0");
    }

    #[test]
    fn evm_l2_version_is_pkg_version() {
        assert_eq!(EvmL2GuestProgram.version(), env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn elf_hash_none_for_missing_elf() {
        struct NoElf;
        impl GuestProgram for NoElf {
            fn program_id(&self) -> &str {
                "no-elf"
            }
            fn elf(&self, _: &str) -> Option<&[u8]> {
                None
            }
            fn vk_bytes(&self, _: &str) -> Option<Vec<u8>> {
                None
            }
            fn program_type_id(&self) -> u8 {
                99
            }
        }
        assert!(NoElf.elf_hash("sp1").is_none());
    }

    #[test]
    fn elf_hash_deterministic() {
        struct FixedElf;
        impl GuestProgram for FixedElf {
            fn program_id(&self) -> &str {
                "fixed"
            }
            fn elf(&self, _: &str) -> Option<&[u8]> {
                Some(b"deterministic-content")
            }
            fn vk_bytes(&self, _: &str) -> Option<Vec<u8>> {
                None
            }
            fn program_type_id(&self) -> u8 {
                99
            }
        }
        let h1 = FixedElf.elf_hash("sp1").unwrap();
        let h2 = FixedElf.elf_hash("sp1").unwrap();
        assert_eq!(h1, h2);
        // Hash should not be all zeros.
        assert_ne!(h1, [0u8; 32]);
    }

    #[test]
    fn evm_l2_has_limits() {
        let limits = EvmL2GuestProgram.resource_limits();
        assert_eq!(limits.max_input_bytes, Some(256 * 1024 * 1024));
        assert_eq!(
            limits.max_proving_duration,
            Some(std::time::Duration::from_secs(3600))
        );
    }

    /// Verify that serialize_input and encode_output never panic on
    /// arbitrary byte inputs.  All three programs use pass-through
    /// implementations, so we expect Ok for any input.
    #[test]
    fn serialize_input_never_panics_on_arbitrary_bytes() {
        let programs: Vec<Box<dyn GuestProgram>> = vec![
            Box::new(EvmL2GuestProgram),
            Box::new(ZkDexGuestProgram),
            Box::new(TokammonGuestProgram),
        ];

        // Test with various edge-case inputs.
        let all_bytes: Vec<u8> = (0..=255).collect();
        let inputs: Vec<&[u8]> = vec![
            b"",                 // empty
            b"\x00",             // null byte
            b"\xff\xff\xff\xff", // all-ones
            &[0u8; 1024],        // large zero-filled
            b"\x7fELF",          // ELF magic (wrong context)
            b"hello world",      // ASCII
            &all_bytes,          // all byte values
        ];

        for prog in &programs {
            for input in &inputs {
                // Must not panic; result is always Ok for pass-through.
                let result = prog.serialize_input(input);
                assert!(
                    result.is_ok(),
                    "{}: serialize_input panicked",
                    prog.program_id()
                );
                assert_eq!(result.unwrap(), *input);
            }
        }
    }

    #[test]
    fn encode_output_never_panics_on_arbitrary_bytes() {
        let programs: Vec<Box<dyn GuestProgram>> = vec![
            Box::new(EvmL2GuestProgram),
            Box::new(ZkDexGuestProgram),
            Box::new(TokammonGuestProgram),
        ];

        let inputs: Vec<&[u8]> = vec![
            b"",
            b"\x00\x00\x00\x00",
            &[0xffu8; 256],
            b"not valid output",
        ];

        for prog in &programs {
            for input in &inputs {
                let result = prog.encode_output(input);
                assert!(
                    result.is_ok(),
                    "{}: encode_output panicked",
                    prog.program_id()
                );
                assert_eq!(result.unwrap(), *input);
            }
        }
    }

    #[test]
    fn validate_elf_never_panics_on_arbitrary_bytes() {
        let test_backends = [
            backends::SP1,
            backends::RISC0,
            backends::ZISK,
            backends::OPENVM,
            backends::EXEC,
            "unknown",
        ];

        // Assorted byte patterns that should never cause a panic.
        let inputs: Vec<Vec<u8>> = vec![
            vec![],
            vec![0],
            vec![0x7f, b'E', b'L', b'F'], // valid magic but too short
            vec![0; 20],                  // right length, wrong magic
            vec![0xff; 100],              // garbage
            make_elf(0, EM_RISCV),        // invalid class 0
            make_elf(3, EM_RISCV),        // invalid class 3
            make_elf(ELFCLASS32, 0),      // machine 0
            make_elf(ELFCLASS32, 0xffff), // machine 0xffff
        ];

        for backend in &test_backends {
            for input in &inputs {
                // Must not panic; may return Ok or Err.
                let _ = validate_elf_header(backend, input);
            }
        }
    }
}
