use directories::ProjectDirs;
use ethrex_storage::AccountUpdate;
use serde::{Deserialize, Serialize};
use std::fs::{create_dir, read_dir, File};
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};
use std::{
    fs::create_dir_all,
    io::{BufWriter, Write},
};

#[derive(Serialize, Deserialize, Debug)]
pub enum StateType {
    Proof(Box<(risc0_zkvm::Receipt, Vec<u32>)>),
    AccountUpdates(Vec<AccountUpdate>),
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy)]
pub enum StateFileType {
    Proof,
    AccountUpdates,
}

#[cfg(not(test))]
const DEFAULT_DATADIR: &str = "ethrex_l2_state";

#[cfg(not(test))]
#[inline(always)]
fn default_datadir() -> Result<PathBuf, Box<dyn std::error::Error>> {
    create_datadir(DEFAULT_DATADIR)
}

#[cfg(test)]
#[inline(always)]
fn default_datadir() -> Result<PathBuf, Box<dyn std::error::Error>> {
    create_datadir("test_datadir")
}

#[inline(always)]
fn create_datadir(dir_name: &str) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let path_buf_data_dir = ProjectDirs::from("", "", dir_name)
        .ok_or_else(|| Box::<dyn std::error::Error>::from("Couldn't get project_dir."))?
        .data_local_dir()
        .to_path_buf();
    Ok(path_buf_data_dir)
}

#[inline(always)]
fn get_state_dir_for_block(block_number: u64) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let mut path_buf = default_datadir()?;
    path_buf.push(block_number.to_string());

    Ok(path_buf)
}

#[inline(always)]
fn get_state_file_path(
    path_buf: &Path,
    block_number: u64,
    state_file_type: StateFileType,
) -> PathBuf {
    match state_file_type {
        StateFileType::AccountUpdates => {
            path_buf.join(format!("account_updates{block_number}.json"))
        }
        StateFileType::Proof => path_buf.join(format!("proof_{block_number}.json")),
    }
}

/// CREATE
fn create_state_file_for_block_number(
    block_number: u64,
    state_file_type: StateFileType,
) -> Result<File, Box<dyn std::error::Error>> {
    let path_buf = get_state_dir_for_block(block_number)?;
    if let Some(parent) = path_buf.parent() {
        if let Err(e) = create_dir_all(parent) {
            if e.kind() != std::io::ErrorKind::AlreadyExists {
                eprintln!("Failed to create directory: {:?}. Error: {e}", parent);
                return Err(e.into());
            }
        }
    }

    let block_number = path_buf
        .file_name()
        .ok_or_else(|| Box::<dyn std::error::Error>::from("Error: No file_name()"))?
        .to_string_lossy();

    let block_number = block_number.parse::<u64>()?;

    let file_path: PathBuf = get_state_file_path(&path_buf, block_number, state_file_type);

    if let Err(e) = create_dir(&path_buf) {
        if e.kind() != std::io::ErrorKind::AlreadyExists {
            eprintln!("Failed to create directory: {:?}. Error: {e}", path_buf);
            return Err(e.into());
        }
    }

    File::create(file_path).map_err(Into::into)
}

/// WRITE
pub fn write_state_in_block_state_path(
    block_number: u64,
    state_type: StateType,
    state_file_type: StateFileType,
) -> Result<(), Box<dyn std::error::Error>> {
    let inner = create_state_file_for_block_number(block_number, state_file_type)?;

    match state_type {
        StateType::Proof(value) => {
            let mut writer = BufWriter::new(inner);
            serde_json::to_writer(&mut writer, &value)?;
            writer.flush()?;
        }
        StateType::AccountUpdates(value) => {
            let mut writer = BufWriter::new(inner);
            serde_json::to_writer(&mut writer, &value)?;
            writer.flush()?;
        }
    }

    Ok(())
}

fn get_latest_block_number_and_path_from_state_path(
) -> Result<(u64, PathBuf), Box<dyn std::error::Error>> {
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
        None => Err(Box::from("No valid block directories found")),
    }
}

fn get_block_state_path(block_number: u64) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let data_dir = default_datadir()?;
    let block_state_path = data_dir.join(block_number.to_string());
    Ok(block_state_path)
}

pub fn get_latest_block_number() -> Result<u64, Box<dyn std::error::Error>> {
    let (block_number, _) = get_latest_block_number_and_path_from_state_path()?;
    Ok(block_number)
}

/// READ
pub fn read_state_file_for_block_number(
    block_number: u64,
    state_file_type: StateFileType,
) -> Result<StateType, Box<dyn std::error::Error>> {
    // TODO handle path not found
    let block_state_path = get_block_state_path(block_number)?;
    let file_path: PathBuf = get_state_file_path(&block_state_path, block_number, state_file_type);

    let inner = File::open(file_path)?;
    let mut reader = BufReader::new(inner);
    let mut buf = String::new();

    reader.read_to_string(&mut buf)?;

    let state = match state_file_type {
        StateFileType::Proof => {
            let state: (risc0_zkvm::Receipt, Vec<u32>) = serde_json::from_str(&buf)?;
            StateType::Proof(Box::new(state))
        }
        StateFileType::AccountUpdates => {
            let state: Vec<AccountUpdate> = serde_json::from_str(&buf)?;
            StateType::AccountUpdates(state)
        }
    };

    Ok(state)
}

/// READ
pub fn read_latest_state_file(
    state_file_type: StateFileType,
) -> Result<StateType, Box<dyn std::error::Error>> {
    let (latest_block_state_number, _) = get_latest_block_number_and_path_from_state_path()?;
    let state = read_state_file_for_block_number(latest_block_state_number, state_file_type)?;
    Ok(state)
}

/// DELETE
pub fn delete_state_file_for_block_number(
    block_number: u64,
    state_file_type: StateFileType,
) -> Result<(), Box<dyn std::error::Error>> {
    let block_state_path = get_block_state_path(block_number)?;
    let file_path: PathBuf = get_state_file_path(&block_state_path, block_number, state_file_type);
    std::fs::remove_file(file_path)?;

    Ok(())
}

pub fn delete_latest_state_file(
    state_file_type: StateFileType,
) -> Result<(), Box<dyn std::error::Error>> {
    let (latest_block_state_number, _) = get_latest_block_number_and_path_from_state_path()?;
    let latest_block_state_path = get_block_state_path(latest_block_state_number)?;
    let file_path: PathBuf = get_state_file_path(
        &latest_block_state_path,
        latest_block_state_number,
        state_file_type,
    );
    std::fs::remove_file(file_path)?;

    Ok(())
}

pub fn delete_state_path_for_block_number(
    block_number: u64,
) -> Result<(), Box<dyn std::error::Error>> {
    let block_state_path = get_block_state_path(block_number)?;
    std::fs::remove_dir_all(block_state_path)?;
    Ok(())
}

pub fn delete_latest_state_path() -> Result<(), Box<dyn std::error::Error>> {
    let (latest_block_state_number, _) = get_latest_block_number_and_path_from_state_path()?;
    let latest_block_state_path = get_block_state_path(latest_block_state_number)?;
    std::fs::remove_dir_all(latest_block_state_path)?;
    Ok(())
}

pub fn path_has_state_file(
    state_file_type: StateFileType,
    path_buf: &Path,
) -> Result<bool, Box<dyn std::error::Error>> {
    let file_prefix = match state_file_type {
        StateFileType::AccountUpdates => "account_updates",
        StateFileType::Proof => "proof",
    };

    for entry in std::fs::read_dir(path_buf)? {
        let entry = entry?;
        let file_name = entry.file_name();
        let lossy_string = file_name.to_string_lossy();

        let matches_prefix = lossy_string.starts_with(file_prefix);
        let matches_suffix = lossy_string.ends_with(".json");

        if matches_prefix && matches_suffix {
            return Ok(true);
        }
    }

    Ok(false)
}

pub fn block_number_has_state_file(
    state_file_type: StateFileType,
    block_number: u64,
) -> Result<bool, Box<dyn std::error::Error>> {
    let block_state_path = get_block_state_path(block_number)?;
    let file_path: PathBuf = get_state_file_path(&block_state_path, block_number, state_file_type);

    let mut path_buf = file_path.clone();
    path_buf.pop();

    let file_prefix = match state_file_type {
        StateFileType::AccountUpdates => "account_updates",
        StateFileType::Proof => "proof",
    };

    for entry in std::fs::read_dir(path_buf)? {
        let entry = entry?;
        let file_name = entry.file_name();
        let lossy_string = file_name.to_string_lossy();

        let matches_prefix = lossy_string.starts_with(file_prefix);
        let matches_suffix = lossy_string.ends_with(".json");

        if matches_prefix && matches_suffix {
            return Ok(true);
        }
    }

    Ok(false)
}

#[cfg(test)]
mod tests {
    use ethrex_blockchain::add_block;
    use ethrex_storage::{EngineType, Store};
    use ethrex_vm::execution_db::ExecutionDB;
    use risc0_zkvm::sha::Digest;

    use super::*;
    use crate::utils::test_data_io;
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

        // Generic Receipt
        let receipt = risc0_zkvm::Receipt::new(
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
        );

        // Write all
        for block in &blocks {
            let account_updates =
                ExecutionDB::get_account_updates(blocks.last().unwrap(), &store).unwrap();

            account_updates_vec.push(account_updates.clone());

            write_state_in_block_state_path(
                block.header.number,
                StateType::AccountUpdates(account_updates),
                StateFileType::AccountUpdates,
            )?;

            write_state_in_block_state_path(
                block.header.number,
                StateType::Proof(Box::new((receipt.clone(), vec![5u32; 8]))),
                StateFileType::Proof,
            )?;
        }

        let (latest_block_state_number, _) = get_latest_block_number_and_path_from_state_path()?;

        assert_eq!(
            latest_block_state_number,
            blocks.last().unwrap().header.number
        );

        // Delete account_updates file
        let (_, latest_path) = get_latest_block_number_and_path_from_state_path()?;

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
        delete_latest_state_path()?;
        let (latest_block_state_number, _) = get_latest_block_number_and_path_from_state_path()?;
        assert_eq!(
            latest_block_state_number,
            blocks.last().unwrap().header.number - 1
        );

        // Read account_updates back
        let read_account_updates_blk2 =
            match read_state_file_for_block_number(2, StateFileType::AccountUpdates)? {
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

        // Read Proof back
        let read_proof_updates_blk2 =
            match read_state_file_for_block_number(2, StateFileType::Proof)? {
                StateType::Proof(p) => p,
                StateType::AccountUpdates(_) => unimplemented!(),
            };

        assert_eq!(
            read_proof_updates_blk2.0.journal.bytes,
            receipt.journal.bytes
        );
        assert_eq!(read_proof_updates_blk2.1, vec![5u32; 8]);

        fs::remove_dir_all(default_datadir()?)?;

        Ok(())
    }
}
