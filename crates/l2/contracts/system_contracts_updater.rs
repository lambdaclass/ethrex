mod utils;
use std::collections::HashMap;
use std::path::Path;

use bytes::Bytes;
use ethrex_common::types::Genesis;
use ethrex_common::types::GenesisAccount;
use ethrex_common::U256;
use ethrex_l2::utils::config::read_env_file;
use ethrex_l2_sdk::COMMON_BRIDGE_L2_ADDRESS;
use utils::compile_contract;
use utils::ContractCompilationError;

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
    let genesis_path = args
        .next()
        .ok_or(ContractCompilationError::FailedToGetStringFromPath)?;

    let file = std::fs::File::open(&genesis_path)?;
    let reader = std::io::BufReader::new(file);
    let mut genesis: Genesis = serde_json::from_reader(reader)?;

    let runtime_code = std::fs::read("contracts/solc_out/CommonBridgeL2.bin-runtime")?;

    genesis.alloc.insert(
        COMMON_BRIDGE_L2_ADDRESS,
        GenesisAccount {
            code: Bytes::from(hex::decode(runtime_code).map_err(|_| {
                ContractCompilationError::InternalError(
                    "Failed to decode runtime code as a hexstring".to_owned(),
                )
            })?),
            storage: HashMap::new(),
            balance: U256::zero(),
            nonce: 1,
        },
    );

    let modified_genesis = serde_json::to_string(&genesis)?;
    std::fs::write(&genesis_path, modified_genesis)?;

    Ok(())
}
