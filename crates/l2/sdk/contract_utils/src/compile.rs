use std::{path::Path, process::Command};

#[derive(Debug, thiserror::Error)]
pub enum ContractCompilationError {
    #[error("The path is not a valid utf-8 string")]
    FailedToGetStringFromPath,
    #[error("Deployer compilation error: {0}")]
    CompilationError(String),
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

    let mut cmd = Command::new("solc");
    cmd.arg(bin_flag)
        .arg(
            "@openzeppelin/contracts=".to_string()
                + general_contracts_path
                    .join("lib")
                    .join("openzeppelin-contracts-upgradeable")
                    .join("lib")
                    .join("openzeppelin-contracts")
                    .join("contracts")
                    .to_str()
                    .ok_or(ContractCompilationError::FailedToGetStringFromPath)?,
        )
        .arg(
            "@openzeppelin/contracts-upgradeable=".to_string()
                + general_contracts_path
                    .join("lib")
                    .join("openzeppelin-contracts-upgradeable")
                    .join("contracts")
                    .to_str()
                    .ok_or(ContractCompilationError::FailedToGetStringFromPath)?,
        )
        .arg(
            general_contracts_path
                .join(contract_path)
                .to_str()
                .ok_or(ContractCompilationError::FailedToGetStringFromPath)?,
        )
        .arg("--via-ir")
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
        );

    let cmd_succeeded = cmd
        .spawn()
        .map_err(|err| {
            ContractCompilationError::CompilationError(format!("Failed to spawn solc: {err}"))
        })?
        .wait()
        .map_err(|err| {
            ContractCompilationError::CompilationError(format!("Failed to wait for solc: {err}"))
        })?
        .success();

    // Both the contract path and the output path are relative to where the Makefile is.
    if !cmd_succeeded {
        return Err(ContractCompilationError::CompilationError(
            format!("Failed to compile {contract_path}").to_owned(),
        ));
    }

    Ok(())
}
