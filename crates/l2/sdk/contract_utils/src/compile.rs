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
    abi_json: bool,
    remappings: Option<&[(&str, PathBuf)]>,
    allow_paths: &[&Path],
    optimize_runs: Option<u64>,
) -> Result<(), ContractCompilationError> {
    let bin_flag = if runtime_bin {
        "--bin-runtime"
    } else {
        "--bin"
    };

    let mut cmd = Command::new("solc");
    cmd.arg(bin_flag);

    if abi_json {
        cmd.arg("--abi");
    }

    apply_remappings(&mut cmd, remappings)?;

    if let Some(optimize_runs) = optimize_runs {
        cmd.arg("--optimize")
            .arg("--optimize-runs")
            .arg(format!("{optimize_runs}"));
    }

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
    .arg("--no-cbor-metadata");

    if !allow_paths.is_empty() {
        apply_allow_paths(&mut cmd, allow_paths)?;
    }

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
        return Err(ContractCompilationError::CompilationError(format!(
            "Failed to compile {contract_path:?}"
        )));
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

fn apply_allow_paths(
    cmd: &mut Command,
    allow_paths: &[&Path],
) -> Result<(), ContractCompilationError> {
    cmd.arg("--allow-paths");
    let joined_paths = allow_paths
        .iter()
        .map(|p| {
            p.to_str()
                .ok_or(ContractCompilationError::FailedToGetStringFromPath)
        })
        .collect::<Result<Vec<_>, _>>()?
        .join(",");
    cmd.arg(joined_paths);
    Ok(())
}
