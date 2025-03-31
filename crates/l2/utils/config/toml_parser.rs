use crate::utils::config::{
    errors::{ConfigError, TomlParserError},
    ConfigMode,
};
use serde::Deserialize;
use std::fs::OpenOptions;
use std::io::Write;

#[derive(Deserialize, Debug)]
struct Deployer {
    address: String,
    private_key: String,
    risc0_contract_verifier: String,
    sp1_contract_verifier: String,
    pico_contract_verifier: String,
    sp1_deploy_verifier: bool,
    pico_deploy_verifier: bool,
    salt_is_zero: bool,
}

impl Deployer {
    fn to_env(&self) -> String {
        let prefix = "DEPLOYER";
        format!(
            "{prefix}_ADDRESS={}
{prefix}_PRIVATE_KEY={}
{prefix}_RISC0_CONTRACT_VERIFIER={}
{prefix}_SP1_CONTRACT_VERIFIER={}
{prefix}_PICO_CONTRACT_VERIFIER={}
{prefix}_SP1_DEPLOY_VERIFIER={}
{prefix}_PICO_DEPLOY_VERIFIER={}
{prefix}_SALT_IS_ZERO={}
",
            self.address,
            self.private_key,
            self.risc0_contract_verifier,
            self.sp1_contract_verifier,
            self.pico_contract_verifier,
            self.sp1_deploy_verifier,
            self.pico_deploy_verifier,
            self.salt_is_zero
        )
    }
}

#[derive(Deserialize, Debug)]
struct Eth {
    rpc_url: String,
}

impl Eth {
    fn to_env(&self) -> String {
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
    fn to_env(&self) -> String {
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
    block_time_ms: u64,
    coinbase_address: String,
}

impl Proposer {
    fn to_env(&self) -> String {
        let prefix = "PROPOSER";
        format!(
            "
{prefix}_BLOCK_TIME_MS={}
{prefix}_COINBASE_ADDRESS={}
",
            self.block_time_ms, self.coinbase_address,
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
    validium: bool,
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
{prefix}_VALIDIUM={}
",
            self.on_chain_proposer_address,
            self.l1_address,
            self.l1_private_key,
            self.interval_ms,
            self.arbitrary_base_blob_gas_price,
            self.validium,
        )
    }
}

#[derive(Deserialize, Debug)]
struct ProverClient {
    prover_server_endpoint: String,
    sp1_prover: String,
    risc0_dev_mode: u64,
    proving_time_ms: u64,
}

impl ProverClient {
    fn to_env(&self) -> String {
        let prefix = "PROVER_CLIENT";
        format!(
            "{prefix}_PROVER_SERVER_ENDPOINT={}
{prefix}_PROVING_TIME_MS={}
RISC0_DEV_MODE={}
SP1_PROVER={}
",
            self.prover_server_endpoint, self.proving_time_ms, self.risc0_dev_mode, self.sp1_prover
        )
    }
}

#[derive(Deserialize, Debug)]
struct ProverServer {
    listen_ip: String,
    listen_port: u64,
    verifier_address: String,
    verifier_private_key: String,
    dev_mode: bool,
    dev_interval_ms: u64,
}

impl ProverServer {
    fn to_env(&self) -> String {
        let prefix = "PROVER_SERVER";
        format!(
            "
{prefix}_LISTEN_IP={}
{prefix}_LISTEN_PORT={}
{prefix}_VERIFIER_ADDRESS={}
{prefix}_VERIFIER_PRIVATE_KEY={}
{prefix}_DEV_MODE={}
{prefix}_DEV_INTERVAL_MS={}
",
            self.listen_ip,
            self.listen_port,
            self.verifier_address,
            self.verifier_private_key,
            self.dev_mode,
            self.dev_interval_ms
        )
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
    prover_server: ProverServer,
}

impl L2Config {
    fn to_env(&self) -> String {
        let mut env_representation = String::new();

        env_representation.push_str(&self.deployer.to_env());
        env_representation.push_str(&self.eth.to_env());
        env_representation.push_str(&self.engine.to_env());
        env_representation.push_str(&self.watcher.to_env());
        env_representation.push_str(&self.proposer.to_env());
        env_representation.push_str(&self.committer.to_env());
        env_representation.push_str(&self.prover_server.to_env());

        env_representation
    }
}

fn write_to_env(config: String, mode: ConfigMode) -> Result<(), TomlParserError> {
    let env_file_path = mode.get_env_path_or_default();
    let env_file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(env_file_path);
    match env_file {
        Ok(mut file) => {
            file.write_all(&config.into_bytes()).map_err(|_| {
                TomlParserError::EnvWriteError(format!(
                    "Couldn't write file in {}, line: {}",
                    file!(),
                    line!()
                ))
            })?;
        }
        Err(err) => {
            return Err(TomlParserError::EnvWriteError(format!(
                "Error: {}. Couldn't write file in {}, line: {}",
                err,
                file!(),
                line!()
            )));
        }
    };
    Ok(())
}

fn read_config(config_path: String, mode: ConfigMode) -> Result<(), ConfigError> {
    let toml_path = mode.get_config_file_path(&config_path);
    let toml_file_name = toml_path
        .file_name()
        .ok_or(ConfigError::Custom("Invalid CONFIGS_PATH".to_string()))?
        .to_str()
        .ok_or(ConfigError::Custom("Couldn't convert to_str()".to_string()))?
        .to_owned();
    let file = std::fs::read_to_string(toml_path).map_err(|err| {
        TomlParserError::TomlFileNotFound(format!("{err}: {}", toml_file_name.clone()), mode)
    })?;
    match mode {
        ConfigMode::Sequencer => {
            let config: L2Config = toml::from_str(&file).map_err(|err| {
                TomlParserError::TomlFormat(format!("{err}: {}", toml_file_name.clone()), mode)
            })?;
            write_to_env(config.to_env(), mode)?;
        }
        ConfigMode::ProverClient => {
            let config: ProverClient = toml::from_str(&file).map_err(|err| {
                TomlParserError::TomlFormat(format!("{err}: {}", toml_file_name.clone()), mode)
            })?;
            write_to_env(config.to_env(), mode)?;
        }
    }

    Ok(())
}

pub fn parse_configs(mode: ConfigMode) -> Result<(), ConfigError> {
    #[allow(clippy::expect_fun_call, clippy::expect_used)]
    let config_path = std::env::var("CONFIGS_PATH").expect(
        format!(
            "CONFIGS_PATH environment variable not defined. Expected in {}, line: {}
If running locally, a reasonable value would be CONFIGS_PATH=./configs",
            file!(),
            line!()
        )
        .as_str(),
    );

    read_config(config_path, mode).map_err(From::from)
}
