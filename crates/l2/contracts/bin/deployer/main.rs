use std::{
    fs::{read_to_string, File, OpenOptions},
    io::{BufWriter, Write},
    path::PathBuf,
    process::{Command, ExitStatus, Stdio},
    str::FromStr,
};

use bytes::Bytes;
use clap::Parser;
use cli::{parse_private_key, DeployerOptions};
use error::DeployerError;
use ethrex_common::{Address, U256};
use ethrex_l2::utils::test_data_io::read_genesis_file;
use ethrex_l2_sdk::{
    calldata::{encode_calldata, Value},
    compile_contract, deploy_contract, deploy_with_proxy, get_address_from_secret_key,
    initialize_contract,
};
use ethrex_rpc::{
    clients::{eth::BlockByNumber, Overrides},
    EthClient,
};
use keccak_hash::H256;
use tracing::{debug, error, info, trace, warn, Level};

mod cli;
mod error;

const INITIALIZE_ON_CHAIN_PROPOSER_SIGNATURE: &str =
    "initialize(bool,address,address,address,address,address,bytes32,bytes32,address[])";
const INITIALIZE_BRIDGE_ADDRESS_SIGNATURE: &str = "initializeBridgeAddress(address)";
const TRANSFER_OWNERSHIP_SIGNATURE: &str = "transferOwnership(address)";
const ACCEPT_OWNERSHIP_SIGNATURE: &str = "acceptOwnership()";
const BRIDGE_INITIALIZER_SIGNATURE: &str = "initialize(address,address)";

#[tokio::main]
async fn main() -> Result<(), DeployerError> {
    tracing_subscriber::fmt().with_max_level(Level::INFO).init();

    trace!("Starting deployer binary");
    let opts = DeployerOptions::parse();

    let eth_client = EthClient::new_with_config(
        vec![&opts.rpc_url],
        opts.max_number_of_retries,
        opts.backoff_factor,
        opts.min_retry_delay,
        opts.max_retry_delay,
        Some(opts.maximum_allowed_max_fee_per_gas),
        Some(opts.maximum_allowed_max_fee_per_blob_gas),
    )?;

    download_contract_deps(&opts)?;

    compile_contracts(&opts)?;

    let (
        on_chain_proposer_address,
        bridge_address,
        sp1_verifier_address,
        pico_verifier_address,
        risc0_verifier_address,
        tdx_verifier_address,
    ) = deploy_contracts(&eth_client, &opts).await?;

    initialize_contracts(
        on_chain_proposer_address,
        bridge_address,
        risc0_verifier_address,
        sp1_verifier_address,
        pico_verifier_address,
        tdx_verifier_address,
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
        tdx_verifier_address,
        opts.env_file_path,
    )?;
    trace!("Deployer binary finished successfully");
    Ok(())
}

fn download_contract_deps(opts: &DeployerOptions) -> Result<(), DeployerError> {
    trace!("Downloading contract dependencies");
    std::fs::create_dir_all(opts.contracts_path.join("lib")).map_err(|err| {
        DeployerError::DependencyError(format!("Failed to create contracts/lib: {err}"))
    })?;

    git_clone(
        "https://github.com/OpenZeppelin/openzeppelin-contracts-upgradeable.git",
        opts.contracts_path
            .join("lib/openzeppelin-contracts-upgradeable")
            .to_str()
            .ok_or(DeployerError::FailedToGetStringFromPath)?,
        None,
        true,
    )?;

    git_clone(
        "https://github.com/succinctlabs/sp1-contracts.git",
        opts.contracts_path
            .join("lib/sp1-contracts")
            .to_str()
            .ok_or(DeployerError::FailedToGetStringFromPath)?,
        None,
        false,
    )?;

    git_clone(
        "https://github.com/brevis-network/pico-zkapp-template.git",
        opts.contracts_path
            .join("lib/pico-zkapp-template")
            .to_str()
            .ok_or(DeployerError::FailedToGetStringFromPath)?,
        Some("evm"),
        false,
    )?;

    trace!("Contract dependencies downloaded");
    Ok(())
}

pub fn git_clone(
    repository_url: &str,
    outdir: &str,
    branch: Option<&str>,
    submodules: bool,
) -> Result<ExitStatus, DeployerError> {
    trace!(repository_url = %repository_url, outdir = %outdir, branch = ?branch, "Cloning git repository");
    let mut git_cmd = Command::new("git");

    let git_clone_cmd = git_cmd.arg("clone").arg(repository_url);

    if let Some(branch) = branch {
        git_clone_cmd.arg("--branch").arg(branch);
    }

    if submodules {
        git_clone_cmd.arg("--recurse-submodules");
    }

    git_clone_cmd
        .arg(outdir)
        .spawn()
        .map_err(|err| DeployerError::DependencyError(format!("Failed to spawn git: {err}")))?
        .wait()
        .map_err(|err| DeployerError::DependencyError(format!("Failed to wait for git: {err}")))
}

fn compile_contracts(opts: &DeployerOptions) -> Result<(), DeployerError> {
    trace!("Compiling contracts");
    compile_contract(&opts.contracts_path, "lib/openzeppelin-contracts-upgradeable/lib/openzeppelin-contracts/contracts/proxy/ERC1967/ERC1967Proxy.sol", false)?;
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
    trace!("Contracts compiled");
    Ok(())
}

lazy_static::lazy_static! {
    static ref SALT: std::sync::Mutex<H256>  = std::sync::Mutex::new(H256::zero());
}

async fn deploy_contracts(
    eth_client: &EthClient,
    opts: &DeployerOptions,
) -> Result<(Address, Address, Address, Address, Address, Address), DeployerError> {
    trace!("Deploying contracts");

    info!("Deploying OnChainProposer");

    let salt = if opts.randomize_contract_deployment {
        H256::random().as_bytes().to_vec()
    } else {
        SALT.lock()
            .map_err(|_| DeployerError::InternalError("failed unwrapping salt lock".to_string()))?
            .as_bytes()
            .to_vec()
    };

    trace!("Attempting to deploy OnChainProposer contract");
    let on_chain_proposer_deployment = deploy_with_proxy(
        opts.private_key,
        eth_client,
        &opts.contracts_path.join("solc_out"),
        "OnChainProposer.bin",
        &salt,
    )
    .await?;

    info!(
        "OnChainProposer deployed:\n  Proxy -> address={:#x}, tx_hash={:#x}\n  Impl  -> address={:#x}, tx_hash={:#x}",
        on_chain_proposer_deployment.proxy_address,
        on_chain_proposer_deployment.proxy_tx_hash,
        on_chain_proposer_deployment.implementation_address,
        on_chain_proposer_deployment.implementation_tx_hash,
    );

    info!("Deploying CommonBridge");

    let bridge_deployment = deploy_with_proxy(
        opts.private_key,
        eth_client,
        &opts.contracts_path.join("solc_out"),
        "CommonBridge.bin",
        &salt,
    )
    .await?;

    info!(
        "CommonBridge deployed:\n  Proxy -> address={:#x}, tx_hash={:#x}\n  Impl  -> address={:#x}, tx_hash={:#x}",
        bridge_deployment.proxy_address,
        bridge_deployment.proxy_tx_hash,
        bridge_deployment.implementation_address,
        bridge_deployment.implementation_tx_hash,
    );

    let sp1_verifier_address = if opts.sp1_deploy_verifier {
        info!("Deploying SP1Verifier (if sp1_deploy_verifier is true)");
        let (verifier_deployment_tx_hash, sp1_verifier_address) = deploy_contract(
            &[],
            &opts.contracts_path.join("solc_out/SP1Verifier.bin"),
            &opts.private_key,
            &salt,
            eth_client,
        )
        .await?;

        info!(address = %format!("{sp1_verifier_address:#x}"), tx_hash = %format!("{verifier_deployment_tx_hash:#x}"), "SP1Verifier deployed");
        sp1_verifier_address
    } else {
        opts.sp1_verifier_address
            .ok_or(DeployerError::InternalError(
                "SP1Verifier address is not set and sp1_deploy_verifier is false".to_string(),
            ))?
    };

    let pico_verifier_address = if opts.pico_deploy_verifier {
        info!("Deploying PicoVerifier (if pico_deploy_verifier is true)");
        let (verifier_deployment_tx_hash, pico_verifier_address) = deploy_contract(
            &[],
            &opts.contracts_path.join("solc_out/PicoVerifier.bin"),
            &opts.private_key,
            &salt,
            eth_client,
        )
        .await?;

        info!(address = %format!("{pico_verifier_address:#x}"), tx_hash = %format!("{verifier_deployment_tx_hash:#x}"), "PicoVerifier deployed");

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

    let tdx_verifier_address = if opts.tdx_deploy_verifier {
        info!("Deploying TDXVerifier (if tdx_deploy_verifier is true)");
        let tdx_verifier_address =
            deploy_tdx_contracts(opts, on_chain_proposer_deployment.proxy_address)?;

        info!(address = %format!("{tdx_verifier_address:#x}"), "TDXVerifier deployed");
        tdx_verifier_address
    } else {
        opts.tdx_verifier_address
            .ok_or(DeployerError::InternalError(
                "TDXVerifier address is not set and tdx_deploy_verifier is false".to_string(),
            ))?
    };

    trace!(
        on_chain_proposer_proxy_address = ?on_chain_proposer_deployment.proxy_address,
        bridge_proxy_address = ?bridge_deployment.proxy_address,
        on_chain_proposer_implementation_address = ?on_chain_proposer_deployment.implementation_address,
        bridge_implementation_address = ?bridge_deployment.implementation_address,
        sp1_verifier_address = ?sp1_verifier_address,
        pico_verifier_address = ?pico_verifier_address,
        risc0_verifier_address = ?risc0_verifier_address,
        tdx_verifier_address = ?tdx_verifier_address,
        "Contracts deployed"
    );
    Ok((
        on_chain_proposer_deployment.proxy_address,
        bridge_deployment.proxy_address,
        sp1_verifier_address,
        pico_verifier_address,
        risc0_verifier_address,
        tdx_verifier_address,
    ))
}

fn deploy_tdx_contracts(
    opts: &DeployerOptions,
    on_chain_proposer: Address,
) -> Result<Address, DeployerError> {
    Command::new("make")
        .arg("deploy-all")
        .env("PRIVATE_KEY", hex::encode(opts.private_key.as_ref()))
        .env("RPC_URL", &opts.rpc_url)
        .env("ON_CHAIN_PROPOSER", format!("{:#x}", on_chain_proposer))
        .current_dir("tee/contracts")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|err| DeployerError::DependencyError(format!("Failed to spawn make: {err}")))?
        .wait()
        .map_err(|err| DeployerError::DependencyError(format!("Failed to wait for make: {err}")))?;

    let address = read_tdx_deployment_address("TDXVerifier");
    Ok(address)
}

fn read_tdx_deployment_address(name: &str) -> Address {
    let path = format!("tee/contracts/deploydeps/automata-dcap-attestation/evm/deployment/{name}");
    let Ok(contents) = read_to_string(path) else {
        return Address::zero();
    };
    Address::from_str(&contents).unwrap_or(Address::zero())
}

#[allow(clippy::too_many_arguments)]
async fn initialize_contracts(
    on_chain_proposer_address: Address,
    bridge_address: Address,
    risc0_verifier_address: Address,
    sp1_verifier_address: Address,
    pico_verifier_address: Address,
    tdx_verifier_address: Address,
    eth_client: &EthClient,
    opts: &DeployerOptions,
) -> Result<(), DeployerError> {
    trace!("Initializing contracts");

    info!("Initializing OnChainProposer");

    trace!(committer_l1_address = %opts.committer_l1_address, "Using committer L1 address for OnChainProposer initialization");

    let sp1_vk_string = read_to_string(&opts.sp1_vk_path).unwrap_or_else(|_| {
        warn!(
            path = opts.sp1_vk_path,
            "Failed to read SP1 verification key file, will use 0x00..00, this is expected in dev mode"
        );
        "0x00".to_string()
    });
    let sp1_vk = hex::decode(sp1_vk_string.trim_start_matches("0x"))
        .map_err(|err| {
            DeployerError::DecodingError(format!(
                "failed to parse sp1_vk ({sp1_vk_string}) from hex: {err}"
            ))
        })?
        .into();

    let genesis = read_genesis_file(&opts.genesis_l2_path);
    let deployer_address = get_address_from_secret_key(&opts.private_key)?;

    let initialize_tx_hash = {
        let calldata_values = vec![
            Value::Bool(opts.validium),
            Value::Address(deployer_address),
            Value::Address(risc0_verifier_address),
            Value::Address(sp1_verifier_address),
            Value::Address(pico_verifier_address),
            Value::Address(tdx_verifier_address),
            Value::FixedBytes(sp1_vk),
            Value::FixedBytes(genesis.compute_state_root().0.to_vec().into()),
            Value::Array(vec![
                Value::Address(opts.committer_l1_address),
                Value::Address(opts.proof_sender_l1_address),
            ]),
        ];
        trace!(calldata_values = ?calldata_values, "OnChainProposer initialization calldata values");
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

    info!(tx_hash = %format!("{initialize_tx_hash:#x}"), "OnChainProposer initialized");

    let initialize_bridge_address_tx_hash = {
        let calldata_values = vec![Value::Address(bridge_address)];
        let on_chain_proposer_initialization_calldata =
            encode_calldata(INITIALIZE_BRIDGE_ADDRESS_SIGNATURE, &calldata_values)?;

        initialize_contract(
            on_chain_proposer_address,
            on_chain_proposer_initialization_calldata,
            &opts.private_key,
            eth_client,
        )
        .await?
    };

    info!(
        tx_hash = %format!("{initialize_bridge_address_tx_hash:#x}"),
        "OnChainProposer bridge address initialized"
    );

    if opts.on_chain_proposer_owner != deployer_address {
        let transfer_ownership_tx_hash = {
            let owener_transfer_calldata = encode_calldata(
                TRANSFER_OWNERSHIP_SIGNATURE,
                &[Value::Address(opts.on_chain_proposer_owner)],
            )?;

            initialize_contract(
                on_chain_proposer_address,
                owener_transfer_calldata,
                &opts.private_key,
                eth_client,
            )
            .await?
        };

        if let Some(owner_pk) = opts.on_chain_proposer_owner_pk {
            let accept_ownership_calldata = encode_calldata(ACCEPT_OWNERSHIP_SIGNATURE, &[])?;
            let accept_tx = eth_client
                .build_eip1559_transaction(
                    on_chain_proposer_address,
                    opts.on_chain_proposer_owner,
                    accept_ownership_calldata.into(),
                    Overrides::default(),
                )
                .await?;
            let accept_tx_hash = eth_client
                .send_eip1559_transaction(&accept_tx, &owner_pk)
                .await?;

            eth_client
                .wait_for_transaction_receipt(accept_tx_hash, 100)
                .await?;

            info!(
                transfer_tx_hash = %format!("{transfer_ownership_tx_hash:#x}"),
                accept_tx_hash = %format!("{accept_tx_hash:#x}"),
                "OnChainProposer ownership transfered"
            );
        } else {
            info!(
                transfer_tx_hash = %format!("{transfer_ownership_tx_hash:#x}"),
                "OnChainProposer ownership transfered but not accepted yet"
            );
        }
    }

    info!("Initializing CommonBridge");
    let initialize_tx_hash = {
        let calldata_values = vec![
            Value::Address(opts.bridge_owner),
            Value::Address(on_chain_proposer_address),
        ];
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

    info!(tx_hash = %format!("{initialize_tx_hash:#x}"), "CommonBridge initialized");

    trace!("Contracts initialized");
    Ok(())
}

async fn make_deposits(
    bridge: Address,
    eth_client: &EthClient,
    opts: &DeployerOptions,
) -> Result<(), DeployerError> {
    trace!("Making deposits");
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
            debug!(
                ?address,
                "Skipping deposit for address as it is not in the genesis file"
            );
            continue;
        };

        let get_balance = eth_client
            .get_balance(address, BlockByNumber::Latest)
            .await?;
        let value_to_deposit = get_balance
            .checked_div(U256::from_str("2").unwrap_or(U256::zero()))
            .unwrap_or(U256::zero());

        let overrides = Overrides {
            value: Some(value_to_deposit),
            from: Some(address),
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
                info!(
                    ?address,
                    ?value_to_deposit,
                    ?hash,
                    "Deposit transaction sent to L1"
                );
            }
            Err(e) => {
                error!(?address, ?value_to_deposit, "Failed to deposit");
                return Err(DeployerError::EthClientError(e));
            }
        }
    }
    trace!("Deposits finished");
    Ok(())
}

fn write_contract_addresses_to_env(
    on_chain_proposer_address: Address,
    bridge_address: Address,
    sp1_verifier_address: Address,
    pico_verifier_address: Address,
    risc0_verifier_address: Address,
    tdx_verifier_address: Address,
    env_file_path: Option<PathBuf>,
) -> Result<(), DeployerError> {
    trace!("Writing contract addresses to .env file");
    let env_file_path =
        env_file_path.unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.env")); // ethrex/crates/l2/.env

    if !env_file_path.exists() {
        File::create(&env_file_path).map_err(|err| {
            DeployerError::InternalError(format!(
                "Failed to create .env file at {}: {err}",
                env_file_path.display()
            ))
        })?;
    }

    let env_file = OpenOptions::new()
        .write(true)
        .truncate(true)
        .open(&env_file_path)?; // ethrex/crates/l2/.env
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
    writeln!(
        writer,
        "ETHREX_DEPLOYER_TDX_CONTRACT_VERIFIER={tdx_verifier_address:#x}"
    )?;
    // TDX aux contracts, qpl-tool depends on exact env var naming
    writeln!(
        writer,
        "ENCLAVE_ID_DAO={:#x}",
        read_tdx_deployment_address("AutomataEnclaveIdentityDao")
    )?;
    writeln!(
        writer,
        "FMSPC_TCB_DAO={:#x}",
        read_tdx_deployment_address("AutomataFmspcTcbDao")
    )?;
    writeln!(
        writer,
        "PCK_DAO={:#x}",
        read_tdx_deployment_address("AutomataPckDao")
    )?;
    writeln!(
        writer,
        "PCS_DAO={:#x}",
        read_tdx_deployment_address("AutomataPcsDao")
    )?;
    trace!(?env_file_path, "Contract addresses written to .env");
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
