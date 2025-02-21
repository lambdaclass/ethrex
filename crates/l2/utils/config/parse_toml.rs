use serde::Deserialize;

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
