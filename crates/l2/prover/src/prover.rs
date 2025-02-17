use std::{env::temp_dir, fs::read_to_string};

use crate::errors::ProverError;
#[cfg(feature = "build_risc0")]
use ethrex_l2::prover::proving_systems::Risc0Proof;
#[cfg(feature = "build_risc0")]
use ethrex_l2::utils::prover::proving_systems::Risc0Proof;
use ethrex_l2::utils::prover::proving_systems::{
    ExecuteOutput, PicoProof, ProverType, ProvingOutput, Sp1Proof,
};
use pico_sdk::vk_client::KoalaBearProveVKClient;
use tracing::info;

// risc0
#[cfg(feature = "build_risc0")]
use risc0_zkvm::{default_prover, ExecutorEnv, ProverOpts};
#[cfg(feature = "build_risc0")]
use zkvm_interface::methods::{ZKVM_RISC0_PROGRAM_ELF, ZKVM_RISC0_PROGRAM_ID};
use zkvm_interface::{
    io::{ProgramInput, ProgramOutput},
    methods::ZKVM_PICO_PROGRAM_ELF,
    methods::ZKVM_SP1_PROGRAM_ELF,
};

// sp1
use sp1_sdk::{ProverClient, SP1Stdin};

// pico
#[cfg(feature = "build_pico")]
use pico_sdk::client::DefaultProverClient;

#[cfg(feature = "build_risc0")]
/// Structure that wraps all the needed components for the RISC0 proving system
pub struct Risc0Prover<'a> {
    elf: &'a [u8],
    pub id: [u32; 8],
    pub stdout: Vec<u8>,
}

#[cfg(feature = "build_risc0")]
impl<'a> Default for Risc0Prover<'a> {
    fn default() -> Self {
        Self::new()
    }
}
