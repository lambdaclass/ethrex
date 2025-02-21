use std::io::{BufRead, Write};

use tracing::{debug, info};

pub mod committer;
pub mod eth;
pub mod l1_watcher;
pub mod proposer;
pub mod prover_client;
pub mod prover_server;

pub mod errors;

use serde::Deserialize;

pub fn read_env_file() -> Result<(), errors::ConfigError> {
    let env_file_name = std::env::var("ENV_FILE").unwrap_or(".env".to_string());
    let env_file_path = open_readable(env_file_name)?;
    let reader = std::io::BufReader::new(env_file_path);

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
    let env_file_path = std::env::var("ENV_FILE").unwrap_or(".env".to_owned());
    let env_file = open_readable(env_file_path)?;
    let reader = std::io::BufReader::new(env_file);

    Ok(reader.lines())
}

fn open_readable(path: String) -> std::io::Result<std::fs::File> {
    match std::fs::File::open(path) {
        Ok(file) => Ok(file),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            info!(".env file not found, create one by copying .env.example");
            Err(err)
        }
        Err(err) => Err(err),
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

#[derive(Deserialize, Debug)]
struct Deployer {
    address: String,
    private_key: String,
    risc0_contract_verifier: String,
    sp1_contract_verifier: String,
    sp1_deploy_verifier: bool,
}

#[derive(Deserialize, Debug)]
struct Eth {
    rpc_url: String,
}

#[derive(Deserialize, Debug)]
struct Auth {
    rpc_url: String,
    jwt_path: String,
}

#[derive(Deserialize, Debug)]
struct Watcher {
    bridge_address: String,
    check_interval_ms: u64,
    max_block_step: u64,
    l2_proposer_private_key: String,
}

#[derive(Deserialize, Debug)]
struct Proposer {
    interval_ms: u64,
    coinbase_address: String,
}

#[derive(Deserialize, Debug)]
struct Committer {
    on_chain_proposer_address: String,
    l1_address: String,
    l1_private_key: String,
    interval_ms: u64,
    arbitrary_base_blob_gas_price: u64,
}

#[derive(Deserialize, Debug)]
struct Prover {
    sp1_prover: String,
    risc0_dev_mode: u64,
}

#[derive(Deserialize, Debug)]
struct L2Config {
    deployer: Deployer,
    eth: Eth,
    auth: Auth,
    watcher: Watcher,
    proposer: Proposer,
    committer: Committer,
    prover: Prover,
}
pub fn read_toml() {
    println!("Hello ARGENTINA");
    let file = std::fs::read_to_string("config.toml").unwrap();
    // let file = file.replace("\n", "");
    println!("{}\n", &file);
    let config: L2Config = toml::from_str(&file).unwrap();
    dbg!(config);
}
