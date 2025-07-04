use crate::utils::prover::errors::SaveStateError;
use crate::utils::prover::proving_systems::ProverType;
use directories::ProjectDirs;
use ethrex_common::types::AccountUpdate;
use serde::{Deserialize, Serialize};
use std::ffi::OsString;
use std::fs::{File, create_dir, read_dir};
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};
use std::{
    fs::create_dir_all,
    io::{BufWriter, Write},
};
use tracing::info;

use super::proving_systems::BatchProof;

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
        .ok_or(SaveStateError::FailedToCrateDataDir)?
        .data_local_dir()
        .to_path_buf();
    Ok(path_buf_data_dir)
}

// Proposed structure
// 1/
//     account_updates_1.json
//     proof_risc0_1.json
//     proof_sp1_1.json
// 2/
//     account_updates_2.json
//     proof_risc0_2.json
//     proof_sp1_2.json
// All the files are saved at the path defined by [ProjectDirs::data_local_dir]
// and the [DEFAULT_DATADIR] when calling [create_datadir]

/// Enum used to differentiate between the possible types of data we can store per batch.
#[derive(Serialize, Deserialize, Debug)]
pub enum StateType {
    BatchProof(BatchProof),
    AccountUpdates(Vec<AccountUpdate>),
}

/// Enum used to differentiate between the possible types of files we can have per batch.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum StateFileType {
    BatchProof(ProverType),
    AccountUpdates,
}

impl From<&StateType> for StateFileType {
    fn from(state_type: &StateType) -> Self {
        match state_type {
            StateType::BatchProof(proof) => StateFileType::BatchProof(proof.prover_type()),
            StateType::AccountUpdates(_) => StateFileType::AccountUpdates,
        }
    }
}

#[inline(always)]
fn get_proof_file_name_from_prover_type(prover_type: &ProverType, batch_number: u64) -> String {
    match prover_type {
        ProverType::Exec => format!("proof_exec_{batch_number}.json"),
        ProverType::TDX => format!("proof_tdx_{batch_number}.json"),
        ProverType::RISC0 => format!("proof_risc0_{batch_number}.json"),
        ProverType::SP1 => format!("proof_sp1_{batch_number}.json").to_owned(),
        ProverType::Aligned => format!("proof_aligned_{batch_number}.json").to_owned(),
    }
}

#[inline(always)]
fn get_batch_number_from_path(path_buf: &Path) -> Result<u64, SaveStateError> {
    let batch_number = path_buf
        .file_name()
        .ok_or_else(|| SaveStateError::Custom("Error: No file_name()".to_string()))?
        .to_string_lossy();

    let batch_number = batch_number.parse::<u64>()?;
    Ok(batch_number)
}

#[inline(always)]
fn get_state_dir_for_batch(batch_number: u64) -> Result<PathBuf, SaveStateError> {
    let mut path_buf = default_datadir()?;
    path_buf.push(batch_number.to_string());

    Ok(path_buf)
}

#[inline(always)]
fn get_state_file_name(batch_number: u64, state_file_type: &StateFileType) -> String {
    match state_file_type {
        StateFileType::AccountUpdates => format!("account_updates_{batch_number}.json"),
        // If we have more proving systems we have to match them an create a file name with the following structure:
        // proof_<ProverType>_<batch_number>.json
        StateFileType::BatchProof(prover_type) => {
            get_proof_file_name_from_prover_type(prover_type, batch_number)
        }
    }
}

#[inline(always)]
fn get_state_file_path(
    path_buf: &Path,
    batch_number: u64,
    state_file_type: &StateFileType,
) -> PathBuf {
    let file_name = get_state_file_name(batch_number, state_file_type);
    path_buf.join(file_name)
}

/// CREATE the state_file given the batch_number
/// This function will create the following file_path: ../../../<batch_number>/state_file_type
fn create_state_file_for_batch_number(
    batch_number: u64,
    state_file_type: StateFileType,
) -> Result<File, SaveStateError> {
    let path_buf = get_state_dir_for_batch(batch_number)?;
    if let Some(parent) = path_buf.parent() {
        if let Err(e) = create_dir_all(parent) {
            if e.kind() != std::io::ErrorKind::AlreadyExists {
                return Err(e.into());
            }
        }
    }

    let batch_number = get_batch_number_from_path(&path_buf)?;

    let file_path: PathBuf = get_state_file_path(&path_buf, batch_number, &state_file_type);

    if let Err(e) = create_dir(&path_buf) {
        if e.kind() != std::io::ErrorKind::AlreadyExists {
            return Err(e.into());
        }
    }

    File::create(file_path).map_err(Into::into)
}

/// WRITE to the state_file given the batch number and the state_type
/// It also creates the file, if it already exists it will overwrite the file
/// This function will create and write to the following file_path: ../../../<batch_number>/state_file_type
pub fn write_state(batch_number: u64, state_type: &StateType) -> Result<(), SaveStateError> {
    let inner = create_state_file_for_batch_number(batch_number, state_type.into())?;

    match state_type {
        StateType::BatchProof(value) => {
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

fn get_latest_batch_number_and_path() -> Result<(u64, PathBuf), SaveStateError> {
    let data_dir = default_datadir()?;
    let latest_batch_number = read_dir(&data_dir)?
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

    match latest_batch_number {
        Some(batch_number) => {
            let latest_path = data_dir.join(batch_number.to_string());
            Ok((batch_number, latest_path))
        }
        None => Err(SaveStateError::Custom(
            "No valid batch directories found".to_owned(),
        )),
    }
}

fn get_batch_state_path(batch_number: u64) -> Result<PathBuf, SaveStateError> {
    let data_dir = default_datadir()?;
    let batch_state_path = data_dir.join(batch_number.to_string());
    Ok(batch_state_path)
}

// Not used
/// GET the latest batch_number given the proposed structure
pub fn get_latest_batch_number() -> Result<u64, SaveStateError> {
    let (batch_number, _) = get_latest_batch_number_and_path()?;
    Ok(batch_number)
}

/// READ the state given the batch_number and the [StateFileType]
pub fn read_state(
    batch_number: u64,
    state_file_type: StateFileType,
) -> Result<StateType, SaveStateError> {
    // TODO handle path not found
    let batch_state_path = get_batch_state_path(batch_number)?;
    let file_path: PathBuf = get_state_file_path(&batch_state_path, batch_number, &state_file_type);

    let inner = File::open(file_path)?;
    let mut reader = BufReader::new(inner);
    let mut buf = String::new();

    reader.read_to_string(&mut buf)?;

    let state = match state_file_type {
        StateFileType::BatchProof(_) => {
            let state: BatchProof = serde_json::from_str(&buf)?;
            StateType::BatchProof(state)
        }
        StateFileType::AccountUpdates => {
            let state: Vec<AccountUpdate> = serde_json::from_str(&buf)?;
            StateType::AccountUpdates(state)
        }
    };

    Ok(state)
}

/// READ the proof given the batch_number and the [StateFileType::Proof]
pub fn read_proof(
    batch_number: u64,
    state_file_type: StateFileType,
) -> Result<BatchProof, SaveStateError> {
    match read_state(batch_number, state_file_type)? {
        StateType::BatchProof(p) => Ok(p),
        StateType::AccountUpdates(_) => Err(SaveStateError::Custom(
            "Failed in read_proof(), make sure that the state_file_type is a Proof".to_owned(),
        )),
    }
}

/// READ the latest state given the [StateFileType].
/// latest means the state for the highest batch_number available.
pub fn read_latest_state(state_file_type: StateFileType) -> Result<StateType, SaveStateError> {
    let (latest_batch_state_number, _) = get_latest_batch_number_and_path()?;
    let state = read_state(latest_batch_state_number, state_file_type)?;
    Ok(state)
}

/// DELETE the [StateFileType] for the given batch_number
pub fn delete_state_file(
    batch_number: u64,
    state_file_type: StateFileType,
) -> Result<(), SaveStateError> {
    let batch_state_path = get_batch_state_path(batch_number)?;
    let file_path: PathBuf = get_state_file_path(&batch_state_path, batch_number, &state_file_type);
    std::fs::remove_file(file_path)?;

    Ok(())
}

/// DELETE the [StateFileType]
/// latest means the state for the highest batch_number available.
pub fn delete_latest_state_file(state_file_type: StateFileType) -> Result<(), SaveStateError> {
    let (latest_batch_state_number, _) = get_latest_batch_number_and_path()?;
    let latest_batch_state_path = get_batch_state_path(latest_batch_state_number)?;
    let file_path: PathBuf = get_state_file_path(
        &latest_batch_state_path,
        latest_batch_state_number,
        &state_file_type,
    );
    std::fs::remove_file(file_path)?;

    Ok(())
}

/// PRUNE all the files for the given batch_number
pub fn prune_state(batch_number: u64) -> Result<(), SaveStateError> {
    let batch_state_path = get_batch_state_path(batch_number)?;
    std::fs::remove_dir_all(batch_state_path)?;
    Ok(())
}

/// PRUNE all the files
/// latest means the state for the highest batch_number available.
pub fn prune_latest_state() -> Result<(), SaveStateError> {
    let (latest_block_state_number, _) = get_latest_batch_number_and_path()?;
    let latest_block_state_path = get_batch_state_path(latest_block_state_number)?;
    std::fs::remove_dir_all(latest_block_state_path)?;
    Ok(())
}

/// CHECK if the given path has the given [StateFileType]
/// This function will check if the path: ../../../<batch_number>/ contains the state_file_type
pub fn path_has_state_file(
    state_file_type: StateFileType,
    path_buf: &Path,
) -> Result<bool, SaveStateError> {
    // Get the batch_number from the path
    let batch_number = get_batch_number_from_path(path_buf)?;
    let file_name_to_seek: OsString = get_state_file_name(batch_number, &state_file_type).into();

    for entry in std::fs::read_dir(path_buf)? {
        let entry = entry?;
        let file_name_stored = entry.file_name();

        if file_name_stored == file_name_to_seek {
            return Ok(true);
        }
    }

    Ok(false)
}

/// CHECK if the given batch_number has the given [StateFileType]
/// This function will check if the path: ../../../<batch_number>/ contains the state_file_type
pub fn batch_number_has_state_file(
    state_file_type: StateFileType,
    batch_number: u64,
) -> Result<bool, SaveStateError> {
    let batch_state_path = get_batch_state_path(batch_number)?;
    let file_name_to_seek: OsString = get_state_file_name(batch_number, &state_file_type).into();

    if !batch_state_path.exists() {
        return Ok(false);
    }

    for entry in std::fs::read_dir(batch_state_path)? {
        let entry = entry?;
        let file_name_stored = entry.file_name();

        if file_name_stored == file_name_to_seek {
            return Ok(true);
        }
    }

    Ok(false)
}

/// Check if the given batch_number has all the proofs needed
/// This function will check if the path: ../../../<batch_number>/
/// contains the needed_proof_types passed as parameter.
pub fn batch_number_has_all_needed_proofs(
    batch_number: u64,
    needed_proof_types: &[ProverType],
) -> Result<bool, SaveStateError> {
    if needed_proof_types.is_empty() {
        return Ok(true);
    }

    let batch_state_path = get_batch_state_path(batch_number)?;

    let mut has_all_proofs = true;
    for prover_type in needed_proof_types {
        let file_name_to_seek: OsString =
            get_state_file_name(batch_number, &StateFileType::BatchProof(*prover_type)).into();

        // Check if the proof exists
        let proof_exists = std::fs::read_dir(&batch_state_path)?
            .filter_map(Result::ok) // Filter out errors
            .any(|entry| entry.file_name() == file_name_to_seek);

        // If the proof is missing return false
        if !proof_exists {
            info!("Missing {prover_type} proof");
            has_all_proofs = false;
            break;
        }
    }

    Ok(has_all_proofs)
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use ethrex_blockchain::{Blockchain, vm::StoreVmDatabase};
    use ethrex_levm::{db::gen_db::GeneralizedDatabase, vm::VMType};
    use ethrex_storage::{EngineType, Store};
    use ethrex_vm::{
        DynVmDatabase,
        backends::levm::{CacheDB, LEVM},
    };

    use super::*;
    use crate::utils::{prover::proving_systems::ProofCalldata, test_data_io};
    use std::{
        fs::{self},
        sync::Arc,
    };

    #[tokio::test]
    async fn test_state_file_integration() -> Result<(), Box<dyn std::error::Error>> {
        if let Err(e) = fs::remove_dir_all(default_datadir()?) {
            if e.kind() != std::io::ErrorKind::NotFound {
                eprintln!("Directory NotFound: {:?}", default_datadir()?);
            }
        }

        let path = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../fixtures"));

        let chain_file_path = path.join("blockchain/l2-loadtest.rlp");
        let genesis_file_path = path.join("genesis/perf-ci.json");

        // Create an InMemory Store to later perform an execute_block so we can have the Vec<AccountUpdate>.
        let in_memory_db =
            Store::new("memory", EngineType::InMemory).expect("Failed to create Store");

        let genesis = test_data_io::read_genesis_file(genesis_file_path.to_str().unwrap());
        in_memory_db
            .add_initial_state(genesis.clone())
            .await
            .unwrap();

        let blocks = test_data_io::read_chain_file(chain_file_path.to_str().unwrap());
        // create blockchain
        let blockchain = Blockchain::default_with_store(in_memory_db.clone());
        for block in &blocks {
            blockchain.add_block(block).await.unwrap();
        }

        let mut account_updates_vec: Vec<Vec<AccountUpdate>> = Vec::new();

        let exec_calldata = BatchProof::ProofCalldata(ProofCalldata {
            prover_type: ProverType::Exec,
            calldata: Vec::new(),
        });
        let risc0_calldata = BatchProof::ProofCalldata(ProofCalldata {
            prover_type: ProverType::RISC0,
            calldata: Vec::new(),
        });
        let sp1_calldata = BatchProof::ProofCalldata(ProofCalldata {
            prover_type: ProverType::SP1,
            calldata: Vec::new(),
        });

        // Write all the account_updates and proofs for each block
        // TODO: Update. We are executing only the last block and using the block_number as batch_number
        for block in &blocks {
            let store: DynVmDatabase =
                Box::new(StoreVmDatabase::new(in_memory_db.clone(), block.hash()));
            let mut db = GeneralizedDatabase::new(Arc::new(store), CacheDB::new());
            LEVM::execute_block(blocks.last().unwrap(), &mut db, VMType::L2)?;
            let account_updates = LEVM::get_state_transitions(&mut db)?;

            account_updates_vec.push(account_updates.clone());

            write_state(
                block.header.number,
                &StateType::AccountUpdates(account_updates),
            )?;

            write_state(
                block.header.number,
                &StateType::BatchProof(exec_calldata.clone()),
            )?;

            write_state(
                block.header.number,
                &StateType::BatchProof(risc0_calldata.clone()),
            )?;

            write_state(
                block.header.number,
                &StateType::BatchProof(sp1_calldata.clone()),
            )?;
        }

        // Check if the latest batch_number saved matches the latest block in the chain.rlp
        let (latest_batch_state_number, _) = get_latest_batch_number_and_path()?;

        assert_eq!(
            latest_batch_state_number,
            blocks.last().unwrap().header.number
        );

        // Delete account_updates file
        let (_, latest_path) = get_latest_batch_number_and_path()?;

        assert!(path_has_state_file(
            StateFileType::AccountUpdates,
            &latest_path
        )?);

        assert!(batch_number_has_state_file(
            StateFileType::AccountUpdates,
            latest_batch_state_number
        )?);

        delete_latest_state_file(StateFileType::AccountUpdates)?;

        assert!(!path_has_state_file(
            StateFileType::AccountUpdates,
            &latest_path
        )?);

        assert!(!batch_number_has_state_file(
            StateFileType::AccountUpdates,
            latest_batch_state_number
        )?);

        // Delete latest path
        prune_latest_state()?;
        let (latest_batch_state_number, _) = get_latest_batch_number_and_path()?;
        assert_eq!(
            latest_batch_state_number,
            blocks.last().unwrap().header.number - 1
        );

        // Read account_updates back
        let read_account_updates_blk2 = match read_state(2, StateFileType::AccountUpdates)? {
            StateType::BatchProof(_) => unimplemented!(),
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

        // Read Exec Proof back
        let read_proof_updates_blk2 = read_proof(2, StateFileType::BatchProof(ProverType::Exec))?;
        assert_eq!(read_proof_updates_blk2, exec_calldata);

        // Read RISC0 Proof back
        let read_proof_updates_blk2 = read_proof(2, StateFileType::BatchProof(ProverType::RISC0))?;
        assert_eq!(read_proof_updates_blk2, risc0_calldata);

        // Read SP1 Proof back
        let read_proof_updates_blk2 = read_proof(2, StateFileType::BatchProof(ProverType::SP1))?;
        assert_eq!(read_proof_updates_blk2, sp1_calldata);

        fs::remove_dir_all(default_datadir()?)?;

        Ok(())
    }
}
