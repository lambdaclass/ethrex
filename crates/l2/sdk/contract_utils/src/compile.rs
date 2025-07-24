use std::{
    path::{Path, PathBuf},
    process::Command,
};

#[derive(Debug, thiserror::Error)]
pub enum ContractCompilationError {
    #[error("The path is not a valid utf-8 string")]
    FailedToGetStringFromPath,
    #[error("Deployer compilation error: {0}")]
    CompilationError(String),
}

pub fn compile_contract(
    output_dir: &Path,
    contract_path: &Path,
    runtime_bin: bool,
    remappings: Option<&[(&str, PathBuf)]>,
) -> Result<(), ContractCompilationError> {
    let bin_flag = if runtime_bin {
        "--bin-runtime"
    } else {
        "--bin"
    };

    let mut cmd = Command::new("solc");
    cmd.arg(bin_flag);

    apply_remappings(&mut cmd, remappings)?;

    cmd.arg(
        contract_path
            .to_str()
            .ok_or(ContractCompilationError::FailedToGetStringFromPath)?,
    )
    .arg("--via-ir")
    .arg("-o")
    .arg(
        output_dir
            .join("solc_out")
            .to_str()
            .ok_or(ContractCompilationError::FailedToGetStringFromPath)?,
    )
    .arg("--overwrite")
    .arg("--allow-paths")
    .arg(
        output_dir
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

    if !cmd_succeeded {
        return Err(ContractCompilationError::CompilationError(
            format!("Failed to compile {contract_path:?}").to_owned(),
        ));
    }

    Ok(())
}

fn apply_remappings(
    cmd: &mut Command,
    remappings: Option<&[(&str, PathBuf)]>,
) -> Result<(), ContractCompilationError> {
    if let Some(remaps) = remappings {
        for (prefix, path) in remaps {
            let path_str = path
                .to_str()
                .ok_or(ContractCompilationError::FailedToGetStringFromPath)?;
            cmd.arg(format!("{prefix}={path_str}"));
        }
    }
    Ok(())
}
