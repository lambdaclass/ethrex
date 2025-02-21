use serde::Deserialize;

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

impl L2Config {
    pub fn to_env(&self) {
        println!("{}", self.deployer.to_env());
    }
}

pub fn read_toml() {
    println!("Hello ARGENTINA");
    let file = std::fs::read_to_string("config.toml").unwrap();
    // let file = file.replace("\n", "");
    println!("{}\n", &file);
    let config: L2Config = toml::from_str(&file).unwrap();
    dbg!(config);
}
