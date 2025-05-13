use std::{
    io::{BufRead, Write},
    path::{Path, PathBuf},
};
use tracing::{debug, info};

pub mod prover;

pub mod errors;
pub mod toml_parser;

/// Reads the desired .env* file
/// .env        if running the sequencer/L2 node
/// .env.prover if running the prover_client
pub fn read_env_file_by_config() -> Result<(), errors::ConfigError> {
    let env_file_path = {
        let cargo_manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        std::env::var("PROVER_ENV_FILE")
            .map(Into::into)
            .unwrap_or(cargo_manifest_dir.join(".env.prover"))
    };
    let env_file = open_env_file(&env_file_path)?;
    let reader = std::io::BufReader::new(env_file);

    for line in reader.lines() {
        let line = line?;

        if line.starts_with("#") {
            // Skip comments
            continue;
        };

        match line.split_once('=') {
            Some((key, value)) => {
                if std::env::vars().any(|(k, _)| k == key) {
                    debug!("Env var {key} already set, skipping");
                    continue;
                }
                debug!("Setting env var from .env: {key}={value}");
                std::env::set_var(key, value)
            }
            None => continue,
        };
    }

    Ok(())
}

pub fn read_env_as_lines_by_config(
) -> Result<std::io::Lines<std::io::BufReader<std::fs::File>>, errors::ConfigError> {
    let env_file_path = {
        let cargo_manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        std::env::var("PROVER_ENV_FILE")
            .map(Into::into)
            .unwrap_or(cargo_manifest_dir.join(".env.prover"))
    };
    let env_file = open_env_file(&env_file_path)?;
    let reader = std::io::BufReader::new(env_file);

    Ok(reader.lines())
}

fn open_env_file(path: &Path) -> std::io::Result<std::fs::File> {
    match std::fs::File::open(path) {
        Ok(file) => Ok(file),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            info!(".env file not found, create one by copying .env.example");
            Err(err)
        }
        Err(err) => Err(err),
    }
}

pub fn write_env_file_by_config(lines: Vec<String>) -> Result<(), errors::ConfigError> {
    let env_file_path = {
        let cargo_manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        std::env::var("PROVER_ENV_FILE")
            .map(Into::into)
            .unwrap_or(cargo_manifest_dir.join(".env.prover"))
    };

    let env_file = match std::fs::OpenOptions::new()
        .write(true)
        .truncate(true)
        .open(env_file_path)
    {
        Ok(file) => file,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            info!(".env file not found, create one by copying .env.example");
            return Err(err.into());
        }
        Err(err) => return Err(err.into()),
    };

    let mut writer = std::io::BufWriter::new(env_file);
    for line in lines {
        writeln!(writer, "{line}")?;
    }

    Ok(())
}
