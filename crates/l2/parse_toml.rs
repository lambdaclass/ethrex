use crate::errors::*;
use serde::Deserialize;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;
use tracing::info;

#[derive(Deserialize, Debug)]
struct Deployer {
    address: String,
    private_key: String,
    risc0_contract_verifier: String,
    sp1_contract_verifier: String,
    sp1_deploy_verifier: bool,
    salt_is_zero: bool,
}

impl Deployer {
    pub fn to_env(&self) -> String {
        let prefix = "DEPLOYER";
        format!(
            "
{prefix}_ADDRESS={}
{prefix}_PRIVATE_KEY={}
{prefix}_RISC0_CONTRACT_VERIFIER={}
{prefix}_SP1_CONTRACT_VERIFIER={}
{prefix}_SP1_DEPLOY_VERIFIER={}
{prefix}_SALT_IS_ZERO={}
",
            self.address,
            self.private_key,
            self.risc0_contract_verifier,
            self.sp1_contract_verifier,
            self.sp1_deploy_verifier,
            self.salt_is_zero
        )
    }
}

#[derive(Deserialize, Debug)]
struct Eth {
    rpc_url: String,
}

impl Eth {
    pub fn to_env(&self) -> String {
        let prefix = "ETH";
        format!(
            "
{prefix}_RPC_URL={}
",
            self.rpc_url,
        )
    }
}

#[derive(Deserialize, Debug)]
struct Engine {
    rpc_url: String,
    jwt_path: String,
}

impl Engine {
    pub fn to_env(&self) -> String {
        let prefix = "ENGINE_API";
        format!(
            "
{prefix}_RPC_URL={}
{prefix}_JWT_PATH={}
",
            self.rpc_url, self.jwt_path,
        )
    }
}

#[derive(Deserialize, Debug)]
struct Watcher {
    bridge_address: String,
    check_interval_ms: u64,
    max_block_step: u64,
    l2_proposer_private_key: String,
}

impl Watcher {
    pub fn to_env(&self) -> String {
        let prefix = "L1_WATCHER";
        format!(
            "
{prefix}_BRIDGE_ADDRESS={}
{prefix}_CHECK_INTERVAL_MS={}
{prefix}_MAX_BLOCK_STEP={}
{prefix}_L2_PROPOSER_PRIVATE_KEY={}
",
            self.bridge_address,
            self.check_interval_ms,
            self.max_block_step,
            self.l2_proposer_private_key
        )
    }
}

#[derive(Deserialize, Debug)]
struct Proposer {
    interval_ms: u64,
    coinbase_address: String,
}

impl Proposer {
    pub fn to_env(&self) -> String {
        let prefix = "PROPOSER";
        format!(
            "
{prefix}_INTERVAL_MS={}
{prefix}_COINBASE_ADDRESS={}
",
            self.interval_ms, self.coinbase_address,
        )
    }
}

#[derive(Deserialize, Debug)]
struct Committer {
    on_chain_proposer_address: String,
    l1_address: String,
    l1_private_key: String,
    interval_ms: u64,
    arbitrary_base_blob_gas_price: u64,
}

impl Committer {
    pub fn to_env(&self) -> String {
        let prefix = "COMMITTER";
        format!(
            "
{prefix}_ON_CHAIN_PROPOSER_ADDRESS={}
{prefix}_L1_ADDRESS={}
{prefix}_L1_PRIVATE_KEY={}
{prefix}_INTERVAL_MS={}
{prefix}_ARBITRARY_BASE_BLOB_GAS_PRICE={}
",
            self.on_chain_proposer_address,
            self.l1_address,
            self.l1_private_key,
            self.interval_ms,
            self.arbitrary_base_blob_gas_price,
        )
    }
}

#[derive(Deserialize, Debug)]
struct Client {
    prover_server_endpoint: String,
    interval_ms: u64,
}

impl Client {
    pub fn to_env(&self) -> String {
        let prefix = "PROVER_CLIENT";
        format!(
            "
{prefix}_PROVER_SERVER_ENDPOINT={}
{prefix}_INTERVAL_MS={}
",
            self.prover_server_endpoint, self.interval_ms
        )
    }
}

#[derive(Deserialize, Debug)]
struct Server {
    listen_ip: String,
    listen_port: u64,
    verifier_address: String,
    verifier_private_key: String,
    dev_mode: bool,
}

impl Server {
    pub fn to_env(&self) -> String {
        let prefix = "PROVER_SERVER";
        format!(
            "
{prefix}_LISTEN_IP={}
{prefix}_LISTEN_PORT={}
{prefix}_VERIFIER_ADDRESS={}
{prefix}_VERIFIER_PRIVATE_KEY={}
{prefix}_DEV_MODE={}
",
            self.listen_ip,
            self.listen_port,
            self.verifier_address,
            self.verifier_private_key,
            self.dev_mode
        )
    }
}

#[derive(Deserialize, Debug)]
struct Prover {
    sp1_prover: String,
    risc0_dev_mode: u64,
    client: Client,
    server: Server,
}

impl Prover {
    pub fn to_env(&self) -> String {
        let mut env = format!(
            "
SP1_PROVER={}
RISC0_DEV_MODE={}
",
            self.sp1_prover, self.risc0_dev_mode,
        );
        env.push_str(&self.client.to_env());
        env.push_str(&self.server.to_env());
        env
    }
}

#[derive(Deserialize, Debug)]
struct L2Config {
    deployer: Deployer,
    eth: Eth,
    engine: Engine,
    watcher: Watcher,
    proposer: Proposer,
    committer: Committer,
    prover: Prover,
}

impl L2Config {
    pub fn to_env(&self) -> String {
        let mut env_representation = String::new();
        env_representation.push_str(&self.deployer.to_env());
        env_representation.push_str(&self.eth.to_env());
        env_representation.push_str(&self.engine.to_env());
        env_representation.push_str(&self.watcher.to_env());
        env_representation.push_str(&self.proposer.to_env());
        env_representation.push_str(&self.committer.to_env());
        env_representation.push_str(&self.prover.to_env());
        env_representation
    }
}

pub fn write_to_env(config: String) -> Result<(), ConfigError> {
    let env_file_name = std::env::var("ENV_FILE").unwrap_or("/usr/local/bin/.env".to_string());
    let env_file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(env_file_name);
    match env_file {
        Ok(mut file) => {
            dbg!("LO PUDE ABRIR");
            file.write_all(&config.into_bytes()).unwrap();
        }
        Err(err) => {
            dbg!(err);
            return Err(ConfigError::EnvWriteError);
        }
    };
    // {
    //         Ok(file) => file,
    //         Err(err) => {
    //             dbg!(err);
    //             return Err(ConfigError::EnvWriteError)
    //         },
    //     };
    //     env_file
    //         .write_all(&config.into_bytes())
    //         .map_err(|_| ConfigError::EnvWriteError)?;
    // };
    Ok(())
}

pub fn read_toml(toml_path: String) -> Result<(), ConfigError> {
    let file = std::fs::read_to_string(toml_path).map_err(|_| ConfigError::TomlFileNotFound)?;
    let config: L2Config = toml::from_str(&file).map_err(|_| ConfigError::TomlFormat)?;
    write_to_env(config.to_env())?;
    Ok(())
}
