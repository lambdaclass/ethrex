use std::{path::Path, process::Command};

use ethrex_l2::utils::config::errors;

#[derive(Debug, thiserror::Error)]
pub enum ContractCompilationError {
    #[error("The path is not a valid utf-8 string")]
    FailedToGetStringFromPath,
    #[error("Deployer compilation error: {0}")]
    CompilationError(String),
    #[error("Failed to interact with .env file, error: {0}")]
    EnvFileError(#[from] errors::ConfigError),
    #[error("Could not read file")]
    FailedToReadFile(#[from] std::io::Error),
}

pub fn compile_contract(
    general_contracts_path: &Path,
    contract_path: &str,
    runtime_bin: bool,
) -> Result<(), ContractCompilationError> {
    let bin_flag = if runtime_bin {
        "--bin-runtime"
    } else {
        "--bin"
    };

    // Both the contract path and the output path are relative to where the Makefile is.
    if !Command::new("solc")
        .arg(bin_flag)
        .arg(
            general_contracts_path
                .join(contract_path)
                .to_str()
                .ok_or(ContractCompilationError::FailedToGetStringFromPath)?,
        )
        .arg("-o")
        .arg(
            general_contracts_path
                .join("solc_out")
                .to_str()
                .ok_or(ContractCompilationError::FailedToGetStringFromPath)?,
        )
        .arg("--overwrite")
        .arg("--allow-paths")
        .arg(
            general_contracts_path
                .to_str()
                .ok_or(ContractCompilationError::FailedToGetStringFromPath)?,
        )
        .spawn()
        .map_err(|err| {
            ContractCompilationError::CompilationError(format!("Failed to spawn solc: {err}"))
        })?
        .wait()
        .map_err(|err| {
            ContractCompilationError::CompilationError(format!("Failed to wait for solc: {err}"))
        })?
        .success()
    {
        return Err(ContractCompilationError::CompilationError(
            format!("Failed to compile {}", contract_path).to_owned(),
        ));
    }

    Ok(())
}
