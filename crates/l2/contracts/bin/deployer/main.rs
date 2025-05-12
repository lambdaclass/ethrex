use std::{
    fs::{read_to_string, OpenOptions},
    io::{BufWriter, Write},
    path::PathBuf,
    process::{Command, ExitStatus},
    str::FromStr,
};

use bytes::Bytes;
use clap::Parser;
use cli::{parse_private_key, DeployerOptions};
use colored::Colorize;
use error::DeployerError;
use ethrex_common::{Address, U256};
use ethrex_l2::utils::test_data_io::read_genesis_file;
use ethrex_l2_sdk::{
    calldata::{encode_calldata, Value},
    compile_contract, deploy_contract, get_address_from_secret_key, initialize_contract,
};
use ethrex_rpc::{
    clients::{eth::BlockByNumber, EthClientError, Overrides},
    EthClient,
};
use keccak_hash::H256;
use spinoff::{spinner, spinners, Color, Spinner};

mod cli;
mod error;

const INITIALIZE_ON_CHAIN_PROPOSER_SIGNATURE: &str =
    "initialize(address,address,address,address,address[])";
const BRIDGE_INITIALIZER_SIGNATURE: &str = "initialize(address)";

#[tokio::main]
async fn main() -> Result<(), DeployerError> {
    let opts = DeployerOptions::parse();

    let eth_client = EthClient::new_with_maximum_fees(
        &opts.rpc_url,
        opts.maximum_allowed_max_fee_per_gas,
        opts.maximum_allowed_max_fee_per_blob_gas,
    );

    download_contract_deps(&opts)?;

    compile_contracts(&opts)?;

    let (
        on_chain_proposer_address,
        bridge_address,
        sp1_verifier_address,
        pico_verifier_address,
        risc0_verifier_address,
    ) = deploy_contracts(&eth_client, &opts).await?;

    initialize_contracts(
        on_chain_proposer_address,
        bridge_address,
        risc0_verifier_address,
        sp1_verifier_address,
        pico_verifier_address,
        &eth_client,
        &opts,
    )
    .await?;

    if opts.deposit_rich {
        make_deposits(bridge_address, &eth_client, &opts).await?;
    }

    write_contract_addresses_to_env(
        on_chain_proposer_address,
        bridge_address,
        sp1_verifier_address,
        pico_verifier_address,
        risc0_verifier_address,
        opts.env_file_path,
    )
}

fn download_contract_deps(opts: &DeployerOptions) -> Result<(), DeployerError> {
    std::fs::create_dir_all(opts.contracts_path.join("lib")).map_err(|err| {
        DeployerError::DependencyError(format!("Failed to create contracts/lib: {err}"))
    })?;

    git_clone(
        "https://github.com/OpenZeppelin/openzeppelin-contracts.git",
        opts.contracts_path
            .join("lib/openzeppelin-contracts")
            .to_str()
            .ok_or(DeployerError::FailedToGetStringFromPath)?,
        None,
    )?;

    git_clone(
        "https://github.com/succinctlabs/sp1-contracts.git",
        opts.contracts_path
            .join("lib/sp1-contracts")
            .to_str()
            .ok_or(DeployerError::FailedToGetStringFromPath)?,
        None,
    )?;

    git_clone(
        "https://github.com/brevis-network/pico-zkapp-template.git",
        opts.contracts_path
            .join("lib/pico-zkapp-template")
            .to_str()
            .ok_or(DeployerError::FailedToGetStringFromPath)?,
        Some("evm"),
    )?;

    Ok(())
}

pub fn git_clone(
    repository_url: &str,
    outdir: &str,
    branch: Option<&str>,
) -> Result<ExitStatus, DeployerError> {
    let mut git_cmd = Command::new("git");

    let git_clone_cmd = git_cmd.arg("clone").arg(repository_url);

    if let Some(branch) = branch {
        git_clone_cmd.arg("--branch").arg(branch);
    }

    git_clone_cmd
        .arg(outdir)
        .spawn()
        .map_err(|err| DeployerError::DependencyError(format!("Failed to spawn git: {err}")))?
        .wait()
        .map_err(|err| DeployerError::DependencyError(format!("Failed to wait for git: {err}")))
}

fn compile_contracts(opts: &DeployerOptions) -> Result<(), DeployerError> {
    compile_contract(&opts.contracts_path, "src/l1/OnChainProposer.sol", false)?;
    compile_contract(&opts.contracts_path, "src/l1/CommonBridge.sol", false)?;
    compile_contract(
        &opts.contracts_path,
        "lib/sp1-contracts/contracts/src/v4.0.0-rc.3/SP1VerifierGroth16.sol",
        false,
    )?;
    compile_contract(
        &opts.contracts_path,
        "lib/pico-zkapp-template/contracts/src/PicoVerifier.sol",
        false,
    )?;
    Ok(())
}

lazy_static::lazy_static! {
    static ref SALT: std::sync::Mutex<H256>  = std::sync::Mutex::new(H256::zero());
}

async fn deploy_contracts(
    eth_client: &EthClient,
    opts: &DeployerOptions,
) -> Result<(Address, Address, Address, Address, Address), DeployerError> {
    let deploy_frames = spinner!(["📭❱❱", "❱📬❱", "❱❱📫"], 220);

    let mut spinner = Spinner::new(
        deploy_frames.clone(),
        "Deploying OnChainProposer",
        Color::Cyan,
    );

    let salt = if opts.randomize_contract_deployment {
        H256::random().as_bytes().to_vec()
    } else {
        SALT.lock()
            .map_err(|_| DeployerError::InternalError("failed unwrapping salt lock".to_string()))?
            .as_bytes()
            .to_vec()
    };

    let (on_chain_proposer_deployment_tx_hash, on_chain_proposer_address) = deploy_contract(
        &[&[0u8; 31][..], &[u8::from(opts.validium)]].concat(),
        &opts.contracts_path.join("solc_out/OnChainProposer.bin"),
        &opts.private_key,
        &salt,
        eth_client,
    )
    .await?;

    spinner.success(&format!(
        "OnChainProposer:\n\tDeployed at address {}\n\tWith tx hash {}",
        format!("{on_chain_proposer_address:#x}").bright_green(),
        format!("{on_chain_proposer_deployment_tx_hash:#x}").bright_cyan()
    ));

    let mut spinner = Spinner::new(deploy_frames.clone(), "Deploying CommonBridge", Color::Cyan);
    let (bridge_deployment_tx_hash, bridge_address) = deploy_contract(
        &{
            let deployer_address = get_address_from_secret_key(&opts.private_key)?;
            let offset = 32 - deployer_address.as_bytes().len() % 32;
            let mut encoded_owner = vec![0; offset];
            encoded_owner.extend_from_slice(deployer_address.as_bytes());
            encoded_owner
        },
        &opts.contracts_path.join("solc_out/CommonBridge.bin"),
        &opts.private_key,
        &salt,
        eth_client,
    )
    .await?;

    spinner.success(&format!(
        "CommonBridge:\n\tDeployed at address {}\n\tWith tx hash {}",
        format!("{bridge_address:#x}").bright_green(),
        format!("{bridge_deployment_tx_hash:#x}").bright_cyan(),
    ));

    let sp1_verifier_address = if opts.sp1_deploy_verifier {
        let mut spinner = Spinner::new(deploy_frames.clone(), "Deploying SP1Verifier", Color::Cyan);
        let (verifier_deployment_tx_hash, sp1_verifier_address) = deploy_contract(
            &[],
            &opts.contracts_path.join("solc_out/SP1Verifier.bin"),
            &opts.private_key,
            &salt,
            eth_client,
        )
        .await?;

        spinner.success(&format!(
            "SP1Groth16Verifier:\n\tDeployed at address {}\n\tWith tx hash {}",
            format!("{sp1_verifier_address:#x}").bright_green(),
            format!("{verifier_deployment_tx_hash:#x}").bright_cyan(),
        ));
        sp1_verifier_address
    } else {
        opts.sp1_verifier_address
            .ok_or(DeployerError::InternalError(
                "SP1Verifier address is not set and sp1_deploy_verifier is false".to_string(),
            ))?
    };

    let pico_verifier_address = if opts.pico_deploy_verifier {
        let mut spinner = Spinner::new(deploy_frames, "Deploying PicoVerifier", Color::Cyan);
        let (verifier_deployment_tx_hash, pico_verifier_address) = deploy_contract(
            &[],
            &opts.contracts_path.join("solc_out/PicoVerifier.bin"),
            &opts.private_key,
            &salt,
            eth_client,
        )
        .await?;

        spinner.success(&format!(
            "PicoGroth16Verifier:\n\tDeployed at address {}\n\tWith tx hash {}",
            format!("{pico_verifier_address:#x}").bright_green(),
            format!("{verifier_deployment_tx_hash:#x}").bright_cyan(),
        ));

        pico_verifier_address
    } else {
        opts.pico_verifier_address
            .ok_or(DeployerError::InternalError(
                "PicoVerifier address is not set and pico_deploy_verifier is false".to_string(),
            ))?
    };

    // TODO: Add Risc0Verifier deployment
    let risc0_verifier_address =
        opts.risc0_verifier_address
            .ok_or(DeployerError::InternalError(
                "Risc0Verifier address is not set and risc0_deploy_verifier is false".to_string(),
            ))?;

    Ok((
        on_chain_proposer_address,
        bridge_address,
        sp1_verifier_address,
        pico_verifier_address,
        risc0_verifier_address,
    ))
}

#[allow(clippy::too_many_arguments)]
async fn initialize_contracts(
    on_chain_proposer_address: Address,
    bridge_address: Address,
    risc0_verifier_address: Address,
    sp1_verifier_address: Address,
    pico_verifier_address: Address,
    eth_client: &EthClient,
    opts: &DeployerOptions,
) -> Result<(), DeployerError> {
    let initialize_frames = spinner!(["🪄❱❱", "❱🪄❱", "❱❱🪄"], 200);

    let mut spinner = Spinner::new(
        initialize_frames.clone(),
        "Initializing OnChainProposer",
        Color::Cyan,
    );

    let initialize_tx_hash = {
        let calldata_values = vec![
            Value::Address(bridge_address),
            Value::Address(risc0_verifier_address),
            Value::Address(sp1_verifier_address),
            Value::Address(pico_verifier_address),
            Value::Array(vec![
                Value::Address(opts.committer_l1_address),
                Value::Address(opts.proof_sender_l1_address),
            ]),
        ];
        let on_chain_proposer_initialization_calldata =
            encode_calldata(INITIALIZE_ON_CHAIN_PROPOSER_SIGNATURE, &calldata_values)?;

        initialize_contract(
            on_chain_proposer_address,
            on_chain_proposer_initialization_calldata,
            &opts.private_key,
            eth_client,
        )
        .await?
    };

    spinner.success(&format!(
        "OnChainProposer:\n\tInitialized with tx hash {}",
        format!("{initialize_tx_hash:#x}").bright_cyan()
    ));

    let mut spinner = Spinner::new(
        initialize_frames.clone(),
        "Initializing CommonBridge",
        Color::Cyan,
    );
    let initialize_tx_hash = {
        let calldata_values = vec![Value::Address(on_chain_proposer_address)];
        let bridge_initialization_calldata =
            encode_calldata(BRIDGE_INITIALIZER_SIGNATURE, &calldata_values)?;

        initialize_contract(
            bridge_address,
            bridge_initialization_calldata,
            &opts.private_key,
            eth_client,
        )
        .await?
    };

    spinner.success(&format!(
        "CommonBridge:\n\tInitialized with tx hash {}",
        format!("{initialize_tx_hash:#x}").bright_cyan()
    ));
    Ok(())
}

async fn make_deposits(
    bridge: Address,
    eth_client: &EthClient,
    opts: &DeployerOptions,
) -> Result<(), DeployerError> {
    let genesis = read_genesis_file(&opts.genesis_l1_path);
    let pks = read_to_string(&opts.private_keys_file_path)
        .map_err(|_| DeployerError::FailedToGetStringFromPath)?;
    let private_keys: Vec<String> = pks
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| line.trim().to_string())
        .collect();

    for pk in private_keys.iter() {
        let secret_key = parse_private_key(pk).map_err(|_| {
            DeployerError::DecodingError("Error while parsing private key".to_string())
        })?;
        let address = get_address_from_secret_key(&secret_key)?;
        let values = vec![Value::Tuple(vec![
            Value::Address(address),
            Value::Address(address),
            Value::Uint(U256::from(21000 * 5)),
            Value::Bytes(Bytes::from_static(b"")),
        ])];

        let calldata = encode_calldata("deposit((address,address,uint256,bytes))", &values)?;

        let Some(_) = genesis.alloc.get(&address) else {
            println!(
                "Skipping deposit for address {:?} as it is not in the genesis file",
                address
            );
            continue;
        };

        let get_balance = eth_client
            .get_balance(address, BlockByNumber::Latest)
            .await?;
        let value_to_deposit = get_balance
            .checked_div(U256::from_str("2").unwrap_or(U256::zero()))
            .unwrap_or(U256::zero());

        let gas_price = eth_client.get_gas_price().await?.try_into().map_err(|_| {
            EthClientError::InternalError("Failed to convert gas_price to a u64".to_owned())
        })?;

        let overrides = Overrides {
            value: Some(value_to_deposit),
            from: Some(address),
            gas_limit: Some(21000 * 5),
            max_fee_per_gas: Some(gas_price),
            max_priority_fee_per_gas: Some(gas_price),
            ..Overrides::default()
        };

        let build = eth_client
            .build_eip1559_transaction(bridge, address, Bytes::from(calldata), overrides)
            .await?;

        match eth_client
            .send_eip1559_transaction(&build, &secret_key)
            .await
        {
            Ok(hash) => {
                println!(
                    "Deposit transaction sent to L1 from {:?} with value {:?} and hash {:?}",
                    address, value_to_deposit, hash
                );
            }
            Err(e) => {
                println!(
                    "Failed to deposit to {:?} with value {:?}",
                    address, value_to_deposit
                );
                return Err(DeployerError::EthClientError(e));
            }
        }
    }
    Ok(())
}

fn write_contract_addresses_to_env(
    on_chain_proposer_address: Address,
    bridge_address: Address,
    sp1_verifier_address: Address,
    pico_verifier_address: Address,
    risc0_verifier_address: Address,
    env_file_path: Option<PathBuf>,
) -> Result<(), DeployerError> {
    let env_file = OpenOptions::new()
        .write(true)
        .truncate(true)
        .open(env_file_path.unwrap_or(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.env")))?; // ethrex/crates/l2/.env
    let mut writer = BufWriter::new(env_file);
    writeln!(
        writer,
        "ETHREX_COMMITTER_ON_CHAIN_PROPOSER_ADDRESS={on_chain_proposer_address:#x}"
    )?;
    writeln!(writer, "ETHREX_WATCHER_BRIDGE_ADDRESS={bridge_address:#x}")?;
    writeln!(
        writer,
        "ETHREX_DEPLOYER_SP1_CONTRACT_VERIFIER={sp1_verifier_address:#x}"
    )?;

    writeln!(
        writer,
        "ETHREX_DEPLOYER_PICO_CONTRACT_VERIFIER={pico_verifier_address:#x}"
    )?;
    writeln!(
        writer,
        "ETHREX_DEPLOYER_RISC0_CONTRACT_VERIFIER={risc0_verifier_address:#x}"
    )?;
    Ok(())
}

#[allow(clippy::unwrap_used)]
#[allow(clippy::expect_used)]
#[allow(clippy::panic)]
#[cfg(test)]
mod test {
    use crate::{compile_contracts, download_contract_deps, DeployerError, DeployerOptions};
    use std::{env, path::Path};

    #[test]
    fn test_contract_compilation() -> Result<(), DeployerError> {
        let binding = env::current_dir().unwrap();
        let parent_dir = binding.parent().unwrap();

        env::set_current_dir(parent_dir).expect("Failed to change directory");

        let solc_out = parent_dir.join("contracts/solc_out");
        let lib = parent_dir.join("contracts/lib");

        if let Err(e) = std::fs::remove_dir_all(&solc_out) {
            if e.kind() != std::io::ErrorKind::NotFound {
                panic!("Failed to remove directory solc_out");
            }
        }
        if let Err(e) = std::fs::remove_dir_all(&lib) {
            if e.kind() != std::io::ErrorKind::NotFound {
                panic!("failed to remove directory lib");
            }
        }

        let opts = DeployerOptions {
            contracts_path: Path::new("contracts").to_path_buf(),
            ..Default::default()
        };

        download_contract_deps(&opts)?;
        compile_contracts(&opts)?;

        std::fs::remove_dir_all(solc_out).unwrap();
        std::fs::remove_dir_all(lib).unwrap();
        Ok(())
    }
}
