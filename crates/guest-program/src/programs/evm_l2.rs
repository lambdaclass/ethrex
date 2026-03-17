use crate::traits::{GuestProgram, GuestProgramError, ResourceLimits, backends};

/// The default EVM-L2 guest program.
///
/// This wraps the existing monolithic guest program (stateless EVM block
/// execution + message passing + blob verification) behind the [`GuestProgram`]
/// trait, with zero behavior change.
///
/// ELF binaries are sourced from the compile-time constants already defined in
/// the crate root (`ZKVM_SP1_PROGRAM_ELF`, etc.).  When a feature flag is
/// disabled the corresponding constant is empty and [`elf`](GuestProgram::elf)
/// returns `None`.
pub struct EvmL2GuestProgram;

impl EvmL2GuestProgram {
    /// Returns `Some(elf)` if the slice contains real program data,
    /// `None` if it is empty or a sentinel value (e.g. `&[0]`).
    fn non_empty(elf: &[u8]) -> Option<&[u8]> {
        if elf.is_empty() || elf == [0] {
            None
        } else {
            Some(elf)
        }
    }
}

impl GuestProgram for EvmL2GuestProgram {
    fn program_id(&self) -> &str {
        "evm-l2"
    }

    fn elf(&self, backend: &str) -> Option<&[u8]> {
        match backend {
            backends::SP1 => Self::non_empty(crate::ZKVM_SP1_PROGRAM_ELF),
            backends::RISC0 => Self::non_empty(crate::methods::ETHREX_GUEST_RISC0_ELF),
            backends::ZISK => Self::non_empty(crate::ZKVM_ZISK_PROGRAM_ELF),
            backends::OPENVM => Self::non_empty(crate::ZKVM_OPENVM_PROGRAM_ELF),
            _ => None,
        }
    }

    #[allow(clippy::const_is_empty)] // VK is empty when the feature flag is disabled
    fn vk_bytes(&self, backend: &str) -> Option<Vec<u8>> {
        match backend {
            // RISC0 VK (image ID) is available as a compile-time hex string.
            backends::RISC0 => {
                let vk = crate::ZKVM_RISC0_PROGRAM_VK;
                if vk.is_empty() {
                    None
                } else {
                    Some(vk.trim().as_bytes().to_vec())
                }
            }
            // SP1 VK is generated at runtime via `client.setup(elf)` — no
            // compile-time constant exists.  The VK files produced by build.rs
            // could be included here in the future.
            _ => None,
        }
    }

    fn program_type_id(&self) -> u8 {
        1 // EVM-L2
    }

    fn serialize_input(&self, raw_input: &[u8]) -> Result<Vec<u8>, GuestProgramError> {
        // The EVM-L2 program reads rkyv-serialized ProgramInput from the zkVM
        // stdin.  The caller (ProverBackend) already performs rkyv serialization,
        // so this is a pass-through.
        Ok(raw_input.to_vec())
    }

    fn encode_output(&self, raw_output: &[u8]) -> Result<Vec<u8>, GuestProgramError> {
        // The zkVM's public values are already in the layout expected by
        // OnChainProposer._getPublicInputsFromCommitment(), so pass through.
        Ok(raw_output.to_vec())
    }

    fn resource_limits(&self) -> ResourceLimits {
        ResourceLimits {
            max_input_bytes: Some(256 * 1024 * 1024), // 256 MB
            max_proving_duration: Some(std::time::Duration::from_secs(3600)), // 1 hour
        }
    }

    fn version(&self) -> &str {
        env!("CARGO_PKG_VERSION")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn program_id_is_evm_l2() {
        let gp = EvmL2GuestProgram;
        assert_eq!(gp.program_id(), "evm-l2");
    }

    #[test]
    fn program_type_id_is_one() {
        let gp = EvmL2GuestProgram;
        assert_eq!(gp.program_type_id(), 1);
    }

    #[test]
    fn unknown_backend_returns_none() {
        let gp = EvmL2GuestProgram;
        assert!(gp.elf("nonexistent").is_none());
        assert!(gp.vk_bytes("nonexistent").is_none());
    }

    #[test]
    fn serialize_input_is_identity() {
        let gp = EvmL2GuestProgram;
        let data = b"hello world";
        let result = gp.serialize_input(data).unwrap();
        assert_eq!(result, data);
    }

    #[test]
    fn encode_output_is_identity() {
        let gp = EvmL2GuestProgram;
        let data = b"output bytes";
        let result = gp.encode_output(data).unwrap();
        assert_eq!(result, data);
    }

    #[test]
    fn openvm_backend_elf_lookup() {
        let gp = EvmL2GuestProgram;
        // Without the "openvm" feature the constant is empty, so elf() returns None.
        let result = gp.elf(backends::OPENVM);
        if crate::ZKVM_OPENVM_PROGRAM_ELF.is_empty() {
            assert!(result.is_none());
        } else {
            assert!(result.is_some());
        }
    }

    #[test]
    fn non_empty_filters_sentinels() {
        assert!(EvmL2GuestProgram::non_empty(&[]).is_none());
        assert!(EvmL2GuestProgram::non_empty(&[0]).is_none());
        assert!(EvmL2GuestProgram::non_empty(&[1, 2, 3]).is_some());
    }
}
