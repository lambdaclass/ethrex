use std::{
    io::{BufRead, Write},
    path::PathBuf,
};

use tracing::{debug, info};
pub mod block_producer;
pub mod committer;
pub mod eth;
pub mod l1_watcher;
pub mod prover_client;
pub mod prover_server;

pub mod errors;

pub const CARGO_MANIFEST_DIR: &str = std::env!("CARGO_MANIFEST_DIR");

pub fn read_env_file() -> Result<(), errors::ConfigError> {
    let env_file = open_readable()?;
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

pub fn read_env_as_lines(
) -> Result<std::io::Lines<std::io::BufReader<std::fs::File>>, errors::ConfigError> {
    let env_file = open_readable()?;
    let reader = std::io::BufReader::new(env_file);

    Ok(reader.lines())
}

fn open_readable() -> std::io::Result<std::fs::File> {
    let path = get_env_file_path();
    match std::fs::File::open(path) {
        Ok(file) => Ok(file),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            info!(".env file not found, create one by copying .env.example");
            Err(err)
        }
        Err(err) => Err(err),
    }
}

pub fn get_env_file_path() -> PathBuf {
    match std::env::var("ENV_FILE") {
        Ok(env_file_path) => PathBuf::from(env_file_path),
        Err(_) => PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(".env"),
    }
}

pub fn write_env(lines: Vec<String>) -> Result<(), errors::ConfigError> {
    let env_file_name = std::env::var("ENV_FILE").unwrap_or(".env".to_string());
    let env_file = match std::fs::OpenOptions::new()
        .write(true)
        .truncate(true)
        .open(&env_file_name)
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
