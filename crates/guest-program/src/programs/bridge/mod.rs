pub mod analyze;
pub mod circuit;

use crate::traits::{GuestProgram, GuestProgramError, ResourceLimits, backends};

/// Bridge Guest Program — lightweight ZK proof for deposit/withdraw.
///
/// Uses the common `execute_app_circuit` engine which handles:
/// - `handle_privileged_tx()` → deposits from L1
/// - `handle_withdrawal()` → withdrawals to L1
/// - `handle_eth_transfer()` → L2 internal transfers
///
/// No app-specific operations — much faster proof than evm-l2.
pub struct BridgeGuestProgram;

impl BridgeGuestProgram {
    fn non_empty(elf: &[u8]) -> Option<&[u8]> {
        if elf.is_empty() || elf == [0] {
            None
        } else {
            Some(elf)
        }
    }
}

impl GuestProgram for BridgeGuestProgram {
    fn program_id(&self) -> &str {
        "bridge"
    }

    fn elf(&self, backend: &str) -> Option<&[u8]> {
        match backend {
            backends::SP1 => Self::non_empty(crate::ZKVM_SP1_BRIDGE_ELF),
            _ => None,
        }
    }

    fn vk_bytes(&self, _backend: &str) -> Option<Vec<u8>> {
        None
    }

    fn program_type_id(&self) -> u8 {
        4 // Bridge
    }

    fn version(&self) -> &str {
        "0.1.0"
    }

    fn serialize_input(&self, raw_input: &[u8]) -> Result<Vec<u8>, GuestProgramError> {
        #[cfg(feature = "l2")]
        {
            use crate::common::input_converter::convert_to_app_input;
            use crate::l2::ProgramInput;
            use rkyv::rancor::Error as RkyvError;

            let program_input: ProgramInput =
                rkyv::from_bytes::<ProgramInput, RkyvError>(raw_input)
                    .map_err(|e| GuestProgramError::Serialization(e.to_string()))?;

            let (accounts, storage_slots) = analyze::analyze_bridge_transactions(
                &program_input.blocks,
                &program_input.fee_configs,
                &program_input.execution_witness,
            )
            .map_err(|e| GuestProgramError::Internal(e))?;

            let app_input = convert_to_app_input(program_input, &accounts, &storage_slots)
                .map_err(|e| GuestProgramError::Internal(e.to_string()))?;

            let bytes = rkyv::to_bytes::<RkyvError>(&app_input)
                .map_err(|e| GuestProgramError::Serialization(e.to_string()))?;
            Ok(bytes.to_vec())
        }
        #[cfg(not(feature = "l2"))]
        {
            Ok(raw_input.to_vec())
        }
    }

    fn encode_output(&self, raw_output: &[u8]) -> Result<Vec<u8>, GuestProgramError> {
        Ok(raw_output.to_vec())
    }

    fn resource_limits(&self) -> ResourceLimits {
        ResourceLimits {
            max_input_bytes: Some(64 * 1024 * 1024), // 64 MB
            max_proving_duration: Some(std::time::Duration::from_secs(300)), // 5 min (bridge is fast)
        }
    }
}
