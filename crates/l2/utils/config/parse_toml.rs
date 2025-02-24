use serde::Deserialize;
use std::fs;

#[derive(Deserialize, Debug)]
struct Deployer {
    address: String,
    private_key: String,
    risc0_contract_verifier: String,
    sp1_contract_verifier: String,
    sp1_deploy_verifier: bool,
}

impl Deployer {
    pub fn to_env(&self) -> String {
        let prefix = "DEPLOYER";
        format!(
            "
{prefix}_ADDRESS={},
{prefix}_PRIVATE_KEY={},
{prefix}_RISC0_CONTRACT_VERIFIER={},
{prefix}_SP1_CONTRACT_VERIFIER={},
{prefix}_SP1_DEPLOY_VERIFIER={},
",
            self.address,
            self.private_key,
            self.risc0_contract_verifier,
            self.sp1_contract_verifier,
            self.sp1_deploy_verifier
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
{prefix}_RPC_URL={},
",
            self.rpc_url,
        )
    }
}

#[derive(Deserialize, Debug)]
struct Auth {
    rpc_url: String,
    jwt_path: String,
}

impl Auth {
    pub fn to_env(&self) -> String {
        let prefix = "AUTH";
        format!(
            "
{prefix}_RPC_URL={},
{prefix}_JWT_PATH={},
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
        let prefix = "WATCHER";
        format!(
            "
{prefix}_BRIDGE_ADDRESS={},
{prefix}_CHECK_INTERVAL_MS={},
{prefix}_MAX_BLOCK_STEP={},
{prefix}_L2_PROPOSER_PRIVATE_KEY={},
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
{prefix}_INTERVAL_MS={},
{prefix}_COINBASE_ADDRESS={},
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
{prefix}_ON_CHAIN_PROPOSER_ADDRESS={},
{prefix}_L1_ADDRESS={},
{prefix}_L1_PRIVATE_KEY={},
{prefix}_INTERVAL_MS={},
{prefix}_ARBITRARY_BASE_BLOB_GAS_PRICE={},
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
{prefix}_PROVER_SERVER_ENDPOINT={},
{prefix}_INTERVAL_MS={},
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
{prefix}_LISTEN_IP={},
{prefix}_LISTEN_PORT={},
{prefix}_VERIFIER_ADDRESS={},
{prefix}_VERIFIER_PRIVATE_KEY={},
{prefix}_DEV_MODE={},
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
        let prefix = "PROVER";
        let mut env = format!(
            "
{prefix}_SP1_PROVER={},
{prefix}_RISC0_DEV_MODE={},
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
    auth: Auth,
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
        env_representation.push_str(&self.auth.to_env());
        env_representation.push_str(&self.watcher.to_env());
        env_representation.push_str(&self.proposer.to_env());
        env_representation.push_str(&self.committer.to_env());
        env_representation.push_str(&self.prover.to_env());
        env_representation
    }
}

pub fn write_to_env(config: String) {
    // let env_file_name = std::env::var("ENV_FILE").unwrap_or(".env".to_string());
    let env_file_name = ".env_new";
    // let mut env_file = std::fs::File::create(env_file_name).unwrap();
    // let mut writer = std::io::BufWriter::new(env_file);
    // for line in lines {
    //     writeln!(writer, "{line}")?;
    // }

    // Ok(())
    // env_file.write(config)
    fs::write(env_file_name, config).unwrap();
}

pub fn read_toml() {
    println!("Hello ARGENTINA");
    let file = std::fs::read_to_string("config.toml").unwrap();
    println!("{}\n", &file);
    let config: L2Config = toml::from_str(&file).unwrap();
    write_to_env(config.to_env());
}
