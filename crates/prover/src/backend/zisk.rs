//! ZisK prover backend.
//!
//! The backend has two tiers, selected by cargo features:
//!
//! - `zisk`: the guest ELF, input serialization, `cargo-zisk execute` dry runs
//!   and the `ziskemu -X` profiling used by the zkEVM benchmark. This tier only
//!   shells out to the ZisK CLI tools, so it builds on every host.
//! - `zisk-sdk`: in-process proving, verification and calldata extraction through
//!   the `zisk-sdk` crate. The SDK compiles pil2-stark (with CUDA when a toolkit
//!   is installed) and links MPI, OpenMP and GMP, so it is Linux x86-64 only and
//!   needs the ZisK bundle installed by `ziskup` for the proving keys. It keeps
//!   the STARK and PLONK proving keys loaded between proofs, which a
//!   `cargo-zisk prove` subprocess per batch cannot.

use std::{
    io::ErrorKind,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

use ethrex_common::types::prover::{ProofFormat, ProverOutput, ProverType};
use ethrex_guest_program::{ZKVM_ZISK_PROGRAM_ELF, input::ProgramInput};

use crate::backend::zisk_profile::{ZiskAirCost, parse_air_cost};
use crate::backend::{BackendError, ProverBackend};

const INPUT_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/zisk_input.bin");
const ELF_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/zkvm-zisk-program");

/// Proof produced by the `zisk-sdk` tier. Without that feature the backend
/// cannot prove, so the type carries nothing.
pub struct ZiskProveOutput(#[cfg(feature = "zisk-sdk")] pub zisk_sdk::Proof);

/// ZisK prover backend. See the module docs for the two feature tiers.
#[derive(Default)]
pub struct ZiskBackend;

impl ZiskBackend {
    pub fn new() -> Self {
        Self
    }

    /// Initializes the in-process prover (client, program setup, verification
    /// key) so that the first batch does not pay for it and configuration errors
    /// surface at startup. Fails without the `zisk-sdk` feature.
    pub fn warm_up(&self) -> Result<(), BackendError> {
        #[cfg(feature = "zisk-sdk")]
        {
            sdk::setup().map(|_| ())
        }
        #[cfg(not(feature = "zisk-sdk"))]
        {
            Err(Self::sdk_missing())
        }
    }

    #[cfg(not(feature = "zisk-sdk"))]
    fn sdk_missing() -> BackendError {
        BackendError::not_implemented(
            "ZisK proving needs the `zisk-sdk` feature (Linux x86-64 with the ZisK bundle installed)",
        )
    }

    fn write_elf_file() -> Result<(), BackendError> {
        let needs_write = match std::fs::metadata(ELF_PATH) {
            Ok(meta) => {
                // If the file size doesn't match, we know we need to rewrite without
                // reading potentially large/corrupted file contents.
                if meta.len() != u64::try_from(ZKVM_ZISK_PROGRAM_ELF.len()).unwrap_or(0) {
                    true
                } else {
                    // Size matches — read and compare contents.
                    let existing_content =
                        std::fs::read(ELF_PATH).map_err(BackendError::execution)?;
                    existing_content != ZKVM_ZISK_PROGRAM_ELF
                }
            }
            Err(e) if e.kind() == ErrorKind::NotFound => true,
            Err(e) => return Err(BackendError::execution(e)),
        };

        if needs_write {
            // Atomic write: write to a temporary file in the same directory, then
            // rename into place. rename() is atomic on POSIX filesystems, so we
            // never leave a half-written ELF file behind if the process crashes.
            let tmp_path = format!("{ELF_PATH}.{}.tmp", std::process::id());
            std::fs::write(&tmp_path, ZKVM_ZISK_PROGRAM_ELF).map_err(|e| {
                let _ = std::fs::remove_file(&tmp_path);
                BackendError::execution(e)
            })?;
            std::fs::rename(&tmp_path, ELF_PATH).map_err(|e| {
                let _ = std::fs::remove_file(&tmp_path);
                BackendError::execution(e)
            })?;
        }

        Ok(())
    }

    /// Writes the input to `INPUT_PATH` in ZiskStdin format: an 8-byte
    /// little-endian length prefix, the data, then zero-padding to 8-byte
    /// alignment. The guest reads it back via `ziskos::io::read_slice()`.
    fn write_stdin_file(input_bytes: &[u8]) -> Result<(), BackendError> {
        let data_len = input_bytes.len();
        let total_len = 8 + data_len;
        let padding = (8 - (total_len % 8)) % 8;

        let mut buf = Vec::with_capacity(total_len + padding);
        buf.extend_from_slice(&data_len.to_le_bytes());
        buf.extend_from_slice(input_bytes);
        buf.extend(std::iter::repeat_n(0u8, padding));

        std::fs::write(INPUT_PATH, &buf).map_err(BackendError::serialization)
    }

    /// Execute assuming input is already serialized to INPUT_PATH.
    fn execute_core(&self) -> Result<(), BackendError> {
        let output = Command::new("cargo-zisk")
            .args(["execute", "-e", ELF_PATH, "-i", INPUT_PATH])
            .stdin(Stdio::inherit())
            .stderr(Stdio::inherit())
            .output()
            .map_err(BackendError::execution)?;

        if !output.status.success() {
            return Err(BackendError::execution(format!(
                "ZisK execution failed: {}",
                String::from_utf8_lossy(&output.stderr)
            )));
        }

        Ok(())
    }

    /// Run the guest under ziskemu with profiling and return the AIR-cost
    /// breakdown. No proof generated.
    pub fn execute_profiled(&self, input: ProgramInput) -> Result<ZiskAirCost, BackendError> {
        Self::write_elf_file()?;
        Self::write_stdin_file(&self.serialize_input(&input)?)?;
        const PROFILE_FLAGS: &[&str] = &["-X"];
        let mut args: Vec<&str> = vec!["--elf", ELF_PATH, "--inputs", INPUT_PATH];
        args.extend_from_slice(PROFILE_FLAGS);
        let output = Command::new("ziskemu")
            .args(&args)
            .stdin(Stdio::inherit())
            .output()
            .map_err(BackendError::execution)?;
        if !output.status.success() {
            return Err(BackendError::execution(format!(
                "ziskemu profiled execution failed: {}",
                String::from_utf8_lossy(&output.stderr)
            )));
        }
        let mut combined = String::from_utf8_lossy(&output.stdout).into_owned();
        combined.push('\n');
        combined.push_str(&String::from_utf8_lossy(&output.stderr));
        parse_air_cost(&combined)
    }
}

impl ProverBackend for ZiskBackend {
    type ProofOutput = ZiskProveOutput;
    /// The raw guest input, before ZiskStdin framing.
    type SerializedInput = Vec<u8>;

    fn prover_type(&self) -> ProverType {
        ProverType::ZisK
    }

    fn serialize_input(&self, input: &ProgramInput) -> Result<Self::SerializedInput, BackendError> {
        // On the L1 path `ProgramInput` IS the spec's `statelessInputBytes`, so it
        // must reach the guest byte-for-byte — rkyv-wrapping it would make the
        // guest's schema-prefix check fail. Only the L2 batch input is rkyv.
        #[cfg(feature = "l2")]
        let input_bytes = rkyv::to_bytes::<rkyv::rancor::Error>(input)
            .map_err(BackendError::serialization)?
            .to_vec();
        #[cfg(not(feature = "l2"))]
        let input_bytes = input.clone();

        Ok(input_bytes)
    }

    fn execute(&self, input: ProgramInput) -> Result<(), BackendError> {
        Self::write_elf_file()?;
        Self::write_stdin_file(&self.serialize_input(&input)?)?;
        self.execute_core()
    }

    fn prove(
        &self,
        input: ProgramInput,
        format: ProofFormat,
    ) -> Result<Self::ProofOutput, BackendError> {
        #[cfg(feature = "zisk-sdk")]
        {
            let input_bytes = self.serialize_input(&input)?;
            sdk::prove(sdk::setup()?, input_bytes, format).map(ZiskProveOutput)
        }
        #[cfg(not(feature = "zisk-sdk"))]
        {
            let _ = (input, format);
            Err(Self::sdk_missing())
        }
    }

    fn execute_timed(&self, input: ProgramInput) -> Result<Duration, BackendError> {
        Self::write_elf_file()?;
        Self::write_stdin_file(&self.serialize_input(&input)?)?;
        let start = Instant::now();
        self.execute_core()?;
        Ok(start.elapsed())
    }

    fn verify(&self, proof: &Self::ProofOutput) -> Result<(), BackendError> {
        #[cfg(feature = "zisk-sdk")]
        {
            sdk::verify(sdk::setup()?, &proof.0)
        }
        #[cfg(not(feature = "zisk-sdk"))]
        {
            let _ = proof;
            Err(Self::sdk_missing())
        }
    }

    fn to_proof_bytes(
        &self,
        proof: Self::ProofOutput,
        format: ProofFormat,
    ) -> Result<ProverOutput, BackendError> {
        #[cfg(feature = "zisk-sdk")]
        {
            sdk::to_proof_bytes(proof.0, format)
        }
        #[cfg(not(feature = "zisk-sdk"))]
        {
            let _ = (proof, format);
            Err(Self::sdk_missing())
        }
    }
}

/// In-process proving through `zisk-sdk`.
#[cfg(feature = "zisk-sdk")]
mod sdk {
    use std::sync::OnceLock;

    use ethrex_common::types::prover::{ProofBytes, ProofFormat, ProverOutput, ProverType};
    use ethrex_guest_program::ZKVM_ZISK_PROGRAM_ELF;
    use tracing::{info, warn};
    use zisk_sdk::{
        EmbeddedClient, GuestProgram, ProgramVK, Proof, ProofBody, ProofKind, ProverClient,
        ZiskStdin,
    };

    use crate::backend::BackendError;

    /// Name the ZisK cache files are keyed under (together with the ELF hash).
    const PROGRAM_NAME: &str = "ethrex-guest-zisk";
    /// Directory to save every generated proof into, when set.
    const PROOF_DUMP_DIR_ENV: &str = "ETHREX_ZISK_PROOF_DIR";

    /// Everything a proof needs that is built once per process.
    pub struct ZiskSetup {
        client: EmbeddedClient,
        program: GuestProgram,
        /// The program verification key, as the Solidity verifier hashes it.
        vk: ProgramVK,
    }

    // The SDK allows one prover client per process, so a failed initialization
    // is cached too: the operator has to restart the prover after fixing it.
    static SETUP: OnceLock<Result<ZiskSetup, String>> = OnceLock::new();

    /// The process-wide prover, initialized on first use.
    pub fn setup() -> Result<&'static ZiskSetup, BackendError> {
        SETUP
            .get_or_init(init)
            .as_ref()
            .map_err(BackendError::proving)
    }

    fn init() -> Result<ZiskSetup, String> {
        let program = GuestProgram::from_bytes(PROGRAM_NAME, ZKVM_ZISK_PROGRAM_ELF.to_vec());

        // The assembly executor is the fast native witness generator; it is
        // available wherever the SDK builds (Linux x86-64). `.gpu()` is a
        // runtime switch: the CUDA kernels are compiled in whenever the toolkit
        // was present at build time.
        let mut builder = ProverClient::embedded().assembly().plonk();
        if cfg!(feature = "gpu") {
            builder = builder.gpu();
        }
        let client = builder
            .build()
            .map_err(|e| format!("failed to build the ZisK prover client: {e}"))?;

        info!("Running the ZisK program setup (cached under the ZisK home after the first run)");
        client
            .setup(&program)
            .run_sync()
            .map_err(|e| format!("ZisK program setup failed: {e}"))?;

        let vk = program
            .vk()
            .map_err(|e| format!("failed to derive the ZisK program verification key: {e}"))?;
        info!(
            program_vk = %format!("0x{}", hex::encode(vk_bytes32(&vk.vk))),
            gpu = cfg!(feature = "gpu"),
            "ZisK prover ready"
        );

        Ok(ZiskSetup {
            client,
            program,
            vk,
        })
    }

    /// Proves one execution and wraps the proof in the requested shape.
    pub fn prove(
        setup: &ZiskSetup,
        input_bytes: Vec<u8>,
        format: ProofFormat,
    ) -> Result<Proof, BackendError> {
        // `Groth16` is ethrex's name for the EVM-verifiable shape; for ZisK that
        // is the PLONK wrap. `Compressed` is the smaller fixed-size STARK.
        let kind = match format {
            ProofFormat::Groth16 => ProofKind::Plonk,
            ProofFormat::Compressed => ProofKind::VadcopFinalMinimal,
        };
        // Alongside the proof dump: the raw guest input, for replaying the run
        // under `ziskemu` (which needs the 8-byte length prefix `ZiskStdin` adds).
        if let Some(dir) = std::env::var_os(PROOF_DUMP_DIR_ENV) {
            let path = std::path::Path::new(&dir).join(format!("zisk-input-{}.bin", unix_now()));
            if let Err(e) = std::fs::write(&path, &input_bytes) {
                warn!(path = %path.display(), "could not save the ZisK input: {e}");
            }
        }
        let stdin = ZiskStdin::from_bytes(input_bytes);
        let result = setup
            .client
            .prove(&setup.program, stdin)
            .wrap(kind)
            .run_sync()
            .map_err(BackendError::proving)?;
        info!(
            steps = result.get_execution_steps(),
            proving_time_ms = result.get_proving_time(),
            "ZisK proof generated"
        );
        let proof = result.get_proof().clone();
        // Keeps the proof file around for `cargo-zisk verify` and
        // `cargo-zisk-dev export-solidity-calldata` when an on-chain rejection
        // needs to be inspected.
        if let Some(dir) = std::env::var_os(PROOF_DUMP_DIR_ENV) {
            let path = std::path::Path::new(&dir).join(format!("zisk-proof-{}.bin", unix_now()));
            match proof.save(&path) {
                Ok(()) => info!(path = %path.display(), "ZisK proof saved"),
                Err(e) => warn!(path = %path.display(), "could not save the ZisK proof: {e}"),
            }
        }
        Ok(proof)
    }

    /// Verifies the proof against the embedded program's verification key, so a
    /// proof of a different ELF is rejected even though it would verify on its own.
    pub fn verify(setup: &ZiskSetup, proof: &Proof) -> Result<(), BackendError> {
        proof
            .with_program_vk(&setup.vk)
            .verify()
            .map_err(BackendError::verification)
    }

    /// Converts the proof into what the proof sender submits.
    ///
    /// For the EVM shape only the PLONK proof bytes travel: the OnChainProposer
    /// rebuilds the public values from the batch commitment and takes the program
    /// key and the recursion root from its own storage and the verifier contract.
    pub fn to_proof_bytes(proof: Proof, format: ProofFormat) -> Result<ProverOutput, BackendError> {
        match format {
            ProofFormat::Groth16 => {
                let ProofBody::Plonk {
                    proof_bytes, rootc, ..
                } = &proof.body
                else {
                    return Err(BackendError::proof_conversion(format!(
                        "expected a PLONK proof, got {:?}",
                        proof.kind()
                    )));
                };
                let mut digest = [0u8; 32];
                proof.publics().read_slice(&mut digest);
                info!(
                    program_vk = %format!("0x{}", hex::encode(vk_bytes32(&proof.program_vk.vk))),
                    root_c_vadcop_final = %format!("0x{}", hex::encode(vk_bytes32(rootc))),
                    committed_output_digest = %format!("0x{}", hex::encode(digest)),
                    proof_len = proof_bytes.len(),
                    "ZisK PLONK proof ready for L1"
                );
                Ok(ProverOutput::Proof(ProofBytes {
                    prover_type: ProverType::ZisK,
                    proof: proof_bytes.clone(),
                }))
            }
            ProofFormat::Compressed => {
                // The guest commits sha256(ProgramOutput.encode()) into the first
                // eight 32-bit public slots; the rest stay zero.
                let mut digest = [0u8; 32];
                proof.publics().read_slice(&mut digest);
                Ok(ProverOutput::ProofWithPublicValues {
                    proof_bytes: ProofBytes {
                        prover_type: ProverType::ZisK,
                        proof: proof
                            .get_proof_bytes()
                            .map_err(BackendError::proof_conversion)?,
                    },
                    public_values: digest.to_vec(),
                })
            }
        }
    }

    /// Packs four u64 words big-endian, the `bytes32` form `ZiskVerifier.sol`
    /// hashes for both the program key and `rootCVadcopFinal`.
    fn vk_bytes32(words: &[u64]) -> Vec<u8> {
        words.iter().flat_map(|w| w.to_be_bytes()).collect()
    }

    /// Seconds since the Unix epoch, for naming dumped files.
    fn unix_now() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or_default()
    }
}
