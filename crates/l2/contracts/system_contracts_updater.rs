mod utils;
use std::collections::HashMap;
use std::io::BufWriter;
use std::io::Write;
use std::path::Path;
use std::str::FromStr;

use bytes::Bytes;
use ethrex_common::types::Genesis;
use ethrex_common::types::GenesisAccount;
use ethrex_common::H160;
use ethrex_common::U256;
use ethrex_l2::utils::config::read_env_file;
use utils::compile_contract;
use utils::ContractCompilationError;

const COMMON_BRIDGE_L2_ADDRESS : &str = "0x000000000000000000000000000000000000FFFF";

fn main() -> Result<(), ContractCompilationError> {
    read_env_file()?;
    let contracts_path = Path::new(
        std::env::var("DEPLOYER_CONTRACTS_PATH")
            .unwrap_or(".".to_string())
            .as_str(),
    )
    .to_path_buf();

    compile_contract(&contracts_path, "src/l2/CommonBridgeL2.sol", true)?;

    let mut args = std::env::args();
    if args.len() < 2 {
        println!("Error when updating system contracts: Missing genesis file path argument");
        std::process::exit(1);
    }

    args.next();
    let genesis_path = args.next().ok_or(ContractCompilationError::FailedToGetStringFromPath)?;

    let file = std::fs::File::open(&genesis_path)?;
    let reader = std::io::BufReader::new(file);
    let mut genesis: Genesis =
        serde_json::from_reader(reader).expect("Failed to deserialize genesis file");

    let runtime_code = std::fs::read("contracts/solc_out/CommonBridgeL2.bin-runtime")?;

    genesis.alloc.insert(H160::from_str(COMMON_BRIDGE_L2_ADDRESS).unwrap(), GenesisAccount{
        code: Bytes::from(hex::decode(runtime_code).unwrap()),
        storage: HashMap::new(),
        balance: U256::zero(),
        nonce: 1,
    });

    let modified_genesis = serde_json::to_string(&genesis).unwrap();
    std::fs::write(&genesis_path, modified_genesis).unwrap();

    Ok(())
}
