use std::io::{BufRead, Write};

use tracing::debug;

pub mod committer;
pub mod eth;
pub mod l1_watcher;
pub mod proposer;
pub mod prover_client;
pub mod prover_server;

pub mod errors;

pub fn read_env_file() -> Result<(), errors::ConfigError> {
    let env_file_name = std::env::var("ENV_FILE").unwrap_or(".env".to_string());
    let env_file = std::fs::File::open(env_file_name)?;
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
    let env_file_name = std::env::var("ENV_FILE").unwrap_or(".env".to_owned());
    let env_file = std::fs::File::open(env_file_name)?;
    let reader = std::io::BufReader::new(env_file);

    Ok(reader.lines())
}

pub fn write_env(lines: Vec<String>) -> Result<(), errors::ConfigError> {
    let env_file_name = std::env::var("ENV_FILE").unwrap_or(".env".to_string());

    let file = std::fs::OpenOptions::new()
        .write(true)
        .truncate(true)
        .open(&env_file_name)?;

    let mut writer = std::io::BufWriter::new(file);
    for line in lines {
        writeln!(writer, "{line}")?;
    }

    Ok(())
}
