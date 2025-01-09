use crate::utils::prover::errors::SaveStateError;
use crate::utils::prover::proving_systems::{ProverType, ProvingOutput};
use directories::ProjectDirs;
use ethrex_storage::AccountUpdate;
use serde::{Deserialize, Serialize};
use std::ffi::OsString;
use std::fs::{create_dir, read_dir, File};
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};
use std::{
    fs::create_dir_all,
    io::{BufWriter, Write},
};

#[cfg(not(test))]
/// The default directory for data storage when not running tests.
/// This constant is used to define the default path for data files.
const DEFAULT_DATADIR: &str = "ethrex_l2_state";

#[cfg(not(test))]
#[inline(always)]
fn default_datadir() -> Result<PathBuf, SaveStateError> {
    create_datadir(DEFAULT_DATADIR)
}

#[cfg(test)]
#[inline(always)]
fn default_datadir() -> Result<PathBuf, SaveStateError> {
    create_datadir("test_datadir")
}

#[inline(always)]
fn create_datadir(dir_name: &str) -> Result<PathBuf, SaveStateError> {
    let path_buf_data_dir = ProjectDirs::from("", "", dir_name)
        .ok_or_else(|| SaveStateError::FailedToCrateDataDir)?
        .data_local_dir()
        .to_path_buf();
    Ok(path_buf_data_dir)
}

/// Proposed structure
/// 1/
///     account_updates_1.json
///     proof_risc0_1.json
///     proof_sp1_1.json
/// 2/
///     account_updates_2.json
///     proof_risc0_2.json
///     proof_sp1_2.json
/// All the files are saved at the path defined by [ProjectDirs::data_local_dir]
/// and the [DEFAULT_DATADIR] when calling [create_datadir]

/// Enum used to differentiate between the possible types of data we can store per block.
#[derive(Serialize, Deserialize, Debug)]
pub enum StateType {
    Proof(ProvingOutput),
    AccountUpdates(Vec<AccountUpdate>),
}

/// Enum used to differentiate between the possible types of files we can have per block.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum StateFileType {
    Proof(ProverType),
    AccountUpdates,
}

impl From<&StateType> for StateFileType {
    fn from(state_type: &StateType) -> Self {
        match state_type {
            StateType::Proof(p) => match p {
                ProvingOutput::RISC0(_) => StateFileType::Proof(ProverType::RISC0),
                ProvingOutput::SP1(_) => StateFileType::Proof(ProverType::SP1),
            },
            StateType::AccountUpdates(_) => StateFileType::AccountUpdates,
        }
    }
}

impl From<&ProverType> for StateFileType {
    fn from(prover_type: &ProverType) -> Self {
        match prover_type {
            ProverType::RISC0 => StateFileType::Proof(ProverType::RISC0),
            ProverType::SP1 => StateFileType::Proof(ProverType::SP1),
        }
    }
}

#[inline(always)]
fn get_proof_file_name_from_prover_type(prover_type: &ProverType, block_number: u64) -> String {
    match prover_type {
        ProverType::RISC0 => format!("proof_risc0_{block_number}.json"),
        ProverType::SP1 => format!("proof_sp1_{block_number}.json").to_owned(),
    }
}

#[inline(always)]
fn get_block_number_from_path(path_buf: &Path) -> Result<u64, SaveStateError> {
    let block_number = path_buf
        .file_name()
        .ok_or_else(|| SaveStateError::Custom("Error: No file_name()".to_string()))?
        .to_string_lossy();

    let block_number = block_number.parse::<u64>()?;
    Ok(block_number)
}

#[inline(always)]
fn get_state_dir_for_block(block_number: u64) -> Result<PathBuf, SaveStateError> {
    let mut path_buf = default_datadir()?;
    path_buf.push(block_number.to_string());

    Ok(path_buf)
}

#[inline(always)]
fn get_state_file_name(block_number: u64, state_file_type: &StateFileType) -> String {
    match state_file_type {
        StateFileType::AccountUpdates => format!("account_updates_{block_number}.json"),
        // If we have more proving systems we have to match them an create a file name with the following structure:
        // proof_<ProverType>_<block_number>.json
        StateFileType::Proof(prover_type) => {
            get_proof_file_name_from_prover_type(prover_type, block_number)
        }
    }
}

#[inline(always)]
fn get_state_file_path(
    path_buf: &Path,
    block_number: u64,
    state_file_type: &StateFileType,
) -> PathBuf {
    let file_name = get_state_file_name(block_number, state_file_type);
    path_buf.join(file_name)
}

/// CREATE the state_file given the block_number
/// This function will create the following file_path: ../../../<block_number>/state_file_type
fn create_state_file_for_block_number(
    block_number: u64,
    state_file_type: StateFileType,
) -> Result<File, SaveStateError> {
    let path_buf = get_state_dir_for_block(block_number)?;
    if let Some(parent) = path_buf.parent() {
        if let Err(e) = create_dir_all(parent) {
            if e.kind() != std::io::ErrorKind::AlreadyExists {
                return Err(e.into());
            }
        }
    }

    let block_number = get_block_number_from_path(&path_buf)?;

    let file_path: PathBuf = get_state_file_path(&path_buf, block_number, &state_file_type);

    if let Err(e) = create_dir(&path_buf) {
        if e.kind() != std::io::ErrorKind::AlreadyExists {
            return Err(e.into());
        }
    }

    File::create(file_path).map_err(Into::into)
}

/// WRITE to the state_file given the block number and the state_type
/// It also creates the file, if it already exists it will overwrite the file
/// This function will create and write to the following file_path: ../../../<block_number>/state_file_type
pub fn write_state(block_number: u64, state_type: &StateType) -> Result<(), SaveStateError> {
    let inner = create_state_file_for_block_number(block_number, state_type.into())?;

    match state_type {
        StateType::Proof(value) => {
            let mut writer = BufWriter::new(inner);
            serde_json::to_writer(&mut writer, value)?;
            writer.flush()?;
        }
        StateType::AccountUpdates(value) => {
            let mut writer = BufWriter::new(inner);
            serde_json::to_writer(&mut writer, value)?;
            writer.flush()?;
        }
    }

    Ok(())
}

fn get_latest_block_number_and_path() -> Result<(u64, PathBuf), SaveStateError> {
    let data_dir = default_datadir()?;
    let latest_block_number = read_dir(&data_dir)?
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let path = entry.path();
            if path.is_dir() {
                path.file_name()?.to_str()?.parse::<u64>().ok()
            } else {
                None
            }
        })
        .max();

    match latest_block_number {
        Some(block_number) => {
            let latest_path = data_dir.join(block_number.to_string());
            Ok((block_number, latest_path))
        }
        None => Err(SaveStateError::Custom(
            "No valid block directories found".to_owned(),
        )),
    }
}

fn get_block_state_path(block_number: u64) -> Result<PathBuf, SaveStateError> {
    let data_dir = default_datadir()?;
    let block_state_path = data_dir.join(block_number.to_string());
    Ok(block_state_path)
}

/// GET the latest block_number given the proposed structure
pub fn get_latest_block_number() -> Result<u64, SaveStateError> {
    let (block_number, _) = get_latest_block_number_and_path()?;
    Ok(block_number)
}

/// READ the state given the block_number and the [StateFileType]
pub fn read_state(
    block_number: u64,
    state_file_type: StateFileType,
) -> Result<StateType, SaveStateError> {
    // TODO handle path not found
    let block_state_path = get_block_state_path(block_number)?;
    let file_path: PathBuf = get_state_file_path(&block_state_path, block_number, &state_file_type);

    let inner = File::open(file_path)?;
    let mut reader = BufReader::new(inner);
    let mut buf = String::new();

    reader.read_to_string(&mut buf)?;

    let state = match state_file_type {
        StateFileType::Proof(_) => {
            let state: ProvingOutput = serde_json::from_str(&buf)?;
            StateType::Proof(state)
        }
        StateFileType::AccountUpdates => {
            let state: Vec<AccountUpdate> = serde_json::from_str(&buf)?;
            StateType::AccountUpdates(state)
        }
    };

    Ok(state)
}

/// READ the proof given the block_number and the [StateFileType::Proof]
pub fn read_proof(
    block_number: u64,
    state_file_type: StateFileType,
) -> Result<ProvingOutput, SaveStateError> {
    match read_state(block_number, state_file_type)? {
        StateType::Proof(p) => Ok(p),
        StateType::AccountUpdates(_) => Err(SaveStateError::Custom(
            "Failed in read_proof(), make sure that the state_file_type is a Proof".to_owned(),
        )),
    }
}

/// READ the latest state given the [StateFileType].
/// latest means the state for the highest block_number available.
pub fn read_latest_state(state_file_type: StateFileType) -> Result<StateType, SaveStateError> {
    let (latest_block_state_number, _) = get_latest_block_number_and_path()?;
    let state = read_state(latest_block_state_number, state_file_type)?;
    Ok(state)
}

/// DELETE the [StateFileType] for the given block_number
pub fn delete_state_file(
    block_number: u64,
    state_file_type: StateFileType,
) -> Result<(), SaveStateError> {
    let block_state_path = get_block_state_path(block_number)?;
    let file_path: PathBuf = get_state_file_path(&block_state_path, block_number, &state_file_type);
    std::fs::remove_file(file_path)?;

    Ok(())
}

/// DELETE the [StateFileType]
/// latest means the state for the highest block_number available.
pub fn delete_latest_state_file(state_file_type: StateFileType) -> Result<(), SaveStateError> {
    let (latest_block_state_number, _) = get_latest_block_number_and_path()?;
    let latest_block_state_path = get_block_state_path(latest_block_state_number)?;
    let file_path: PathBuf = get_state_file_path(
        &latest_block_state_path,
        latest_block_state_number,
        &state_file_type,
    );
    std::fs::remove_file(file_path)?;

    Ok(())
}

/// PRUNE all the files for the given block_number
pub fn prune_state(block_number: u64) -> Result<(), SaveStateError> {
    let block_state_path = get_block_state_path(block_number)?;
    std::fs::remove_dir_all(block_state_path)?;
    Ok(())
}

/// PRUNE all the files
/// latest means the state for the highest block_number available.
pub fn prune_latest_state() -> Result<(), SaveStateError> {
    let (latest_block_state_number, _) = get_latest_block_number_and_path()?;
    let latest_block_state_path = get_block_state_path(latest_block_state_number)?;
    std::fs::remove_dir_all(latest_block_state_path)?;
    Ok(())
}

/// CHECK if the given path has the given [StateFileType]
/// This function will check if the path: ../../../<block_number>/ contains the state_file_type
pub fn path_has_state_file(
    state_file_type: StateFileType,
    path_buf: &Path,
) -> Result<bool, SaveStateError> {
    // Get the block_number from the path
    let block_number = get_block_number_from_path(path_buf)?;
    let file_name_to_seek: OsString = get_state_file_name(block_number, &state_file_type).into();

    for entry in std::fs::read_dir(path_buf)? {
        let entry = entry?;
        let file_name_stored = entry.file_name();

        if file_name_stored == file_name_to_seek {
            return Ok(true);
        }
    }

    Ok(false)
}

/// CHECK if the given block_number has the given [StateFileType]
/// This function will check if the path: ../../../<block_number>/ contains the state_file_type
pub fn block_number_has_state_file(
    state_file_type: StateFileType,
    block_number: u64,
) -> Result<bool, SaveStateError> {
    let block_state_path = get_block_state_path(block_number)?;
    let file_name_to_seek: OsString = get_state_file_name(block_number, &state_file_type).into();

    for entry in std::fs::read_dir(block_state_path)? {
        let entry = entry?;
        let file_name_stored = entry.file_name();

        if file_name_stored == file_name_to_seek {
            return Ok(true);
        }
    }

    Ok(false)
}

/// CHECK if the given block_number has all the proofs needed
/// This function will check if the path: ../../../<block_number>/ contains the proofs
/// Make sure to add all new proving_systems in the [ProverType::all] function
pub fn block_number_has_all_proofs(block_number: u64) -> Result<bool, SaveStateError> {
    let block_state_path = get_block_state_path(block_number)?;

    let mut has_all_proofs = true;
    for prover_type in ProverType::all() {
        let file_name_to_seek: OsString =
            get_state_file_name(block_number, &StateFileType::from(prover_type)).into();

        // Check if the proof exists
        let proof_exists = std::fs::read_dir(&block_state_path)?
            .filter_map(Result::ok) // Filter out errors
            .any(|entry| entry.file_name() == file_name_to_seek);

        // If the proof is missing return false
        if !proof_exists {
            has_all_proofs = false;
            break;
        }
    }

    Ok(has_all_proofs)
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use ethrex_blockchain::add_block;
    use ethrex_storage::{EngineType, Store};
    use ethrex_vm::execution_db::ExecutionDB;
    use risc0_zkvm::sha::Digest;
    use sp1_sdk::{HashableKey, PlonkBn254Proof, ProverClient, SP1Proof, SP1PublicValues};

    use super::*;
    use crate::utils::{
        prover::proving_systems::{Risc0Proof, Sp1Proof},
        test_data_io,
    };
    use std::fs::{self};

    #[test]
    fn test_state_file_integration() -> Result<(), Box<dyn std::error::Error>> {
        if let Err(e) = fs::remove_dir_all(default_datadir()?) {
            if e.kind() != std::io::ErrorKind::NotFound {
                eprintln!("Directory NotFound: {:?}", default_datadir()?);
            }
        }

        let path = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../test_data"));

        let chain_file_path = path.join("l2-loadtest.rlp");
        let genesis_file_path = path.join("genesis-l2-old.json");

        // Create an InMemory Store to later perform an execute_block so we can have the Vec<AccountUpdate>.
        let store = Store::new("memory", EngineType::InMemory).expect("Failed to create Store");

        let genesis = test_data_io::read_genesis_file(genesis_file_path.to_str().unwrap());
        store.add_initial_state(genesis.clone()).unwrap();

        let blocks = test_data_io::read_chain_file(chain_file_path.to_str().unwrap());
        for block in &blocks {
            add_block(block, &store).unwrap();
        }

        let mut account_updates_vec: Vec<Vec<AccountUpdate>> = Vec::new();

        // Generic RISC0 Receipt
        let risc0_proof = Risc0Proof {
            receipt: Box::new(risc0_zkvm::Receipt::new(
                risc0_zkvm::InnerReceipt::Fake(risc0_zkvm::FakeReceipt::new(
                    risc0_zkvm::ReceiptClaim {
                        pre: risc0_zkvm::MaybePruned::Pruned(Digest::default()),
                        post: risc0_zkvm::MaybePruned::Pruned(Digest::default()),
                        exit_code: risc0_zkvm::ExitCode::Halted(37 * 2),
                        input: risc0_zkvm::MaybePruned::Value(None),
                        output: risc0_zkvm::MaybePruned::Value(None),
                    },
                )),
                vec![37u8; 32],
            )),
            prover_id: vec![5u32; 8],
        };

        // The following is a dummy elf to get an SP1VerifyingKey
        // It's not the best way, but didn't found an easier one.
        // Else, an elf file has to be saved for this test.
        let magic_bytes1: &[u8] = &[
            0x7f, 0x45, 0x4c, 0x46, 0x01, 0x01, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00,
        ];
        let magic_bytes2: &[u8] = &[
            0x02, 0x00, 0xF3, 0x00, 0x01, 0x00, 0x00, 0x00, 0xD4, 0x8E, 0x21, 0x00, 0x34, 0x00,
            0x00, 0x00,
        ];
        let magic_bytes3: &[u8] = &[
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x34, 0x00, 0x20, 0x00, 0x07, 0x00,
            0x28, 0x00,
        ];

        let prover = ProverClient::mock();
        let (_pk, vk) =
            prover.setup(&[magic_bytes1, magic_bytes2, magic_bytes3, &[0; 256]].concat());

        let sp1_proof = Sp1Proof {
            proof: Box::new(sp1_sdk::SP1ProofWithPublicValues {
                proof: SP1Proof::Plonk(PlonkBn254Proof {
                    public_inputs: ["1".to_owned(), "2".to_owned()],
                    encoded_proof: "d".repeat(4),
                    raw_proof: "d".repeat(4),
                    plonk_vkey_hash: [1; 32],
                }),
                stdin: sp1_sdk::SP1Stdin::new(),
                public_values: SP1PublicValues::new(),
                sp1_version: "dummy".to_owned(),
            }),
            vk,
        };

        // Write all the account_updates and proofs for each block
        for block in &blocks {
            let account_updates =
                ExecutionDB::get_account_updates(blocks.last().unwrap(), &store).unwrap();

            account_updates_vec.push(account_updates.clone());

            write_state(
                block.header.number,
                &StateType::AccountUpdates(account_updates),
            )?;

            let risc0_data = ProvingOutput::RISC0(risc0_proof.clone());
            write_state(block.header.number, &StateType::Proof(risc0_data))?;

            let sp1_data = ProvingOutput::SP1(sp1_proof.clone());
            write_state(block.header.number, &StateType::Proof(sp1_data))?;
        }

        // Check if the latest block_number saved matches the latest block in the chain.rlp
        let (latest_block_state_number, _) = get_latest_block_number_and_path()?;

        assert_eq!(
            latest_block_state_number,
            blocks.last().unwrap().header.number
        );

        // Delete account_updates file
        let (_, latest_path) = get_latest_block_number_and_path()?;

        assert!(path_has_state_file(
            StateFileType::AccountUpdates,
            &latest_path
        )?);

        assert!(block_number_has_state_file(
            StateFileType::AccountUpdates,
            latest_block_state_number
        )?);

        delete_latest_state_file(StateFileType::AccountUpdates)?;

        assert!(!path_has_state_file(
            StateFileType::AccountUpdates,
            &latest_path
        )?);

        assert!(!block_number_has_state_file(
            StateFileType::AccountUpdates,
            latest_block_state_number
        )?);

        // Delete latest path
        prune_latest_state()?;
        let (latest_block_state_number, _) = get_latest_block_number_and_path()?;
        assert_eq!(
            latest_block_state_number,
            blocks.last().unwrap().header.number - 1
        );

        // Read account_updates back
        let read_account_updates_blk2 = match read_state(2, StateFileType::AccountUpdates)? {
            StateType::Proof(_) => unimplemented!(),
            StateType::AccountUpdates(a) => a,
        };

        let og_account_updates_blk2 = account_updates_vec.get(2).unwrap();

        for og_au in og_account_updates_blk2 {
            // The read_account_updates aren't sorted in the same way as the og_account_updates.
            let r_au = read_account_updates_blk2
                .iter()
                .find(|au| au.address == og_au.address)
                .unwrap();

            assert_eq!(og_au.added_storage, r_au.added_storage);
            assert_eq!(og_au.address, r_au.address);
            assert_eq!(og_au.info, r_au.info);
            assert_eq!(og_au.code, r_au.code);
        }

        // Read RISC0 Proof back
        let read_proof_updates_blk2 = read_proof(2, StateFileType::Proof(ProverType::RISC0))?;

        if let ProvingOutput::RISC0(read_risc0_proof) = read_proof_updates_blk2 {
            assert_eq!(
                risc0_proof.receipt.journal.bytes,
                read_risc0_proof.receipt.journal.bytes
            );
            assert_eq!(read_risc0_proof.prover_id, risc0_proof.prover_id);
        }

        // Read SP1 Proof back
        let read_proof_updates_blk2 = read_proof(2, StateFileType::Proof(ProverType::SP1))?;

        if let ProvingOutput::SP1(read_sp1_proof) = read_proof_updates_blk2 {
            assert_eq!(read_sp1_proof.proof.bytes(), sp1_proof.proof.bytes());
            assert_eq!(read_sp1_proof.vk.bytes32(), sp1_proof.vk.bytes32());
        }

        fs::remove_dir_all(default_datadir()?)?;

        Ok(())
    }
}
