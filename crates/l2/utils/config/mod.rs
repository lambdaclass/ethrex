use std::{
    fmt::{Debug, Display},
    io::{BufRead, Write},
    path::{Path, PathBuf},
};

use tracing::{debug, info};

pub mod block_producer;
pub mod committer;
pub mod eth;
pub mod l1_watcher;
pub mod prover_client;
pub mod prover_server;

pub mod errors;
pub mod toml_parser;

#[derive(Clone, Copy)]
pub enum ConfigMode {
    /// Parses the entire the sequencer_config.toml
    /// And generates the .env file.
    Sequencer,
    /// Parses the prover_config.toml
    /// And generates the .env.prover file only with the prover_client config variables.
    ProverClient,
}

impl ConfigMode {
    /// Gets the .*config.toml file from the environment, or sets a default value
    /// sequencer_config.toml         for the sequencer/L2 node
    /// prover_config.toml  for the the prover_client
    fn get_config_file_path(&self, config_path: &str) -> PathBuf {
        match self {
            ConfigMode::Sequencer => {
                let sequencer_config_file_name = std::env::var("SEQUENCER_CONFIG_FILE")
                    .unwrap_or("sequencer_config.toml".to_owned());
                Path::new(&config_path).join(sequencer_config_file_name)
            }
            ConfigMode::ProverClient => {
                let prover_client_config_file_name = std::env::var("PROVER_CLIENT_CONFIG_FILE")
                    .unwrap_or("prover_client_config.toml".to_owned());
                Path::new(&config_path).join(prover_client_config_file_name)
            }
        }
    }

    /// Gets the .env* file from the environment, or sets a default value
    /// .env        for the sequencer/L2 node
    /// .env.prover for the the prover_client
    pub fn get_env_path_or_default(&self) -> PathBuf {
        let cargo_manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));

        match self {
            ConfigMode::Sequencer => std::env::var("ENV_FILE")
                .map(Into::into)
                .unwrap_or(cargo_manifest_dir.join(".env")),
            ConfigMode::ProverClient => std::env::var("PROVER_ENV_FILE")
                .map(Into::into)
                .unwrap_or(cargo_manifest_dir.join(".env.prover")),
        }
    }
}

impl Debug for ConfigMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Sequencer => write!(f, "sequencer"),
            Self::ProverClient => write!(f, "prover_client"),
        }
    }
}

impl Display for ConfigMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Sequencer => write!(f, "sequencer"),
            Self::ProverClient => write!(f, "prover_client"),
        }
    }
}

/// Reads the desired .env* file
/// .env        if running the sequencer/L2 node
/// .env.prover if running the prover_client
pub fn read_env_file_by_config(config_mode: ConfigMode) -> Result<(), errors::ConfigError> {
    let env_file_path = config_mode.get_env_path_or_default();
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
    config_mode: ConfigMode,
) -> Result<std::io::Lines<std::io::BufReader<std::fs::File>>, errors::ConfigError> {
    let env_file_path = config_mode.get_env_path_or_default();
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

pub fn write_env_file_by_config(
    lines: Vec<String>,
    config_mode: ConfigMode,
) -> Result<(), errors::ConfigError> {
    let env_file_path = config_mode.get_env_path_or_default();

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
