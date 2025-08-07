use std::{
    fs::{File, OpenOptions, read_to_string},
    io::{BufReader, BufWriter, Write},
    path::PathBuf,
    process::{Command, Stdio},
    str::FromStr,
};

use bytes::Bytes;
use clap::Parser;
use ethrex_common::{Address, U256, types::Genesis};
use ethrex_l2::sequencer::utils::DEV_MODE_ADDRESS;
use ethrex_l2_common::calldata::Value;
use ethrex_l2_rpc::{
    clients::send_eip1559_transaction,
    signer::{LocalSigner, Signer},
};
use ethrex_l2_sdk::{
    calldata::encode_calldata, deploy_contract_from_bytecode, deploy_with_proxy_from_bytecode,
    initialize_contract,
};
use ethrex_rpc::{
    EthClient,
    clients::{Overrides, eth::get_address_from_secret_key},
    types::block_identifier::{BlockIdentifier, BlockTag},
};
use keccak_hash::H256;
use tracing::{debug, error, info, trace, warn};

use ethrex_l2_sdk::DeployError;
use ethrex_rpc::clients::{EthClientError, eth::errors::CalldataEncodeError};

use clap::ArgAction;
use ethrex_common::H160;
use secp256k1::SecretKey;

use crate::{
    l2::EthOptions,
    networks::{LOCAL_DEVNET_GENESIS_CONTENTS, LOCAL_DEVNETL2_GENESIS_CONTENTS},
    utils::parse_private_key,
};

#[derive(Parser)]
pub struct DeployerOptions {
    #[command(flatten)]
    pub eth_options: EthOptions,
    // Deployer options
    #[arg(
        long = "deployer.private-key",
        value_name = "PRIVATE_KEY",
        value_parser = parse_private_key,
        env = "ETHREX_DEPLOYER_L1_PRIVATE_KEY",
        help_heading = "Deployer options",
        help = "Private key from which the contracts will be deployed from.",
    )]
    pub private_key: SecretKey,
    #[arg(
        long = "deployer.env-file",
        value_name = "PATH",
        env = "ETHREX_DEPLOYER_ENV_FILE_PATH",
        help_heading = "Deployer options",
        help = "Path to the .env file where addresses of deployed contracts will be written to."
    )]
    pub env_file_path: Option<PathBuf>,
    #[arg(
        long = "deployer.deposit-rich-accounts",
        default_value = "false",
        value_name = "BOOLEAN",
        env = "ETHREX_DEPLOYER_DEPLOY_RICH",
        action = ArgAction::SetTrue,
        help_heading = "Deployer options",
        help = "If set, it will deposit ETH from L1 rich wallets to L2 accounts."
    )]
    pub deposit_rich: bool,
    #[arg(
        long = "deployer.rich-accounts-pk-file",
        value_name = "PATH",
        env = "ETHREX_DEPLOYER_PRIVATE_KEYS_FILE_PATH",
        required_if_eq("deposit_rich", "true"),
        requires = "deposit_rich",
        help_heading = "Deployer options",
        help = "Path to the file containing the private keys of the L1 rich accounts."
    )]
    pub private_keys_file_path: Option<PathBuf>,
    #[arg(
        long = "deployer.deterministic-salt",
        value_name = "H256",
        env = "ETHREX_DEPLOYER_DETERMINISTIC_SALT",
        help_heading = "Deployer options",
        help = "Salt to use with CREATE2 deterministic deployer. If used, the contracts will be always deployed under the same addresses."
    )]
    pub create2_salt: Option<H256>,
    #[arg(
        long,
        value_name = "PATH",
        env = "ETHREX_DEPLOYER_GENESIS_L1_PATH",
        required_if_eq("deposit_rich", "true"),
        requires = "deposit_rich",
        help_heading = "Deployer options",
        help = "Path to the genesis file. The default is ../../fixtures/genesis/l1-dev.json"
    )]
    pub genesis_l1_path: Option<PathBuf>,
    // L2 options
    #[arg(
        long = "l2.genesis",
        value_name = "PATH",
        env = "ETHREX_DEPLOYER_GENESIS_L2_PATH",
        required_if_eq("use_compiled_genesis", "false"),
        help_heading = "L2 options",
        help = "Path to the L2 genesis file"
    )]
    pub genesis_l2_path: Option<PathBuf>,
    #[arg(
        long = "l2.l1-committer",
        default_value = "0x3d1e15a1a55578f7c920884a9943b3b35d0d885b",
        value_name = "ADDRESS",
        env = "ETHREX_DEPLOYER_COMMITTER_L1_ADDRESS",
        help_heading = "L2 options",
        help = "Address of the account that commits the batches in L1."
    )]
    pub committer_l1_address: Address,
    #[arg(
        long = "l2.l1-proof-sender",
        default_value = "0xE25583099BA105D9ec0A67f5Ae86D90e50036425",
        value_name = "ADDRESS",
        env = "ETHREX_DEPLOYER_PROOF_SENDER_L1_ADDRESS",
        help_heading = "L2 options",
        help = "Address of the account that sends the proofs to be verified in L1."
    )]
    pub proof_sender_l1_address: Address,
    #[arg(
        long = "l2.validium",
        default_value = "false",
        value_name = "BOOLEAN",
        env = "ETHREX_L2_VALIDIUM",
        help_heading = "L2 options",
        help = "If true, L2 will run on validium mode as opposed to the default rollup mode, meaning it will not publish state diffs to the L1."
    )]
    pub validium: bool,
    // Verifiers options
    // TODO: This should work side by side with a risc0_deploy_verifier flag.
    #[arg(
        long = "verifier.risc0",
        value_name = "ADDRESS",
        env = "ETHREX_DEPLOYER_RISC0_CONTRACT_VERIFIER",
        help_heading = "Verifiers options",
        help = "L1 address of the RISC0 verifier. If not set, RISC0 verification will be disabled and not required by the contract."
    )]
    pub risc0_verifier_address: Option<Address>,
    #[arg(
        long = "verifier.sp1",
        value_name = "ADDRESS",
        env = "ETHREX_DEPLOYER_SP1_CONTRACT_VERIFIER",
        conflicts_with = "sp1_deploy_verifier",
        help_heading = "Verifiers options",
        help = "L1 address of the SP1 verifier. If not set, SP1 verification will be disabled and not required by the contract."
    )]
    pub sp1_verifier_address: Option<Address>,
    #[arg(
        long = "verifier.deploy-sp1",
        default_value = "false",
        value_name = "BOOLEAN",
        action = ArgAction::SetTrue,
        env = "ETHREX_DEPLOYER_SP1_DEPLOY_VERIFIER",
        help_heading = "Verifiers options",
        help = "If set to true, it will deploy the SP1 verifier contract and use its address.",
    )]
    pub sp1_deploy_verifier: bool,
    #[arg(
        long = "verifier.tdx",
        value_name = "ADDRESS",
        env = "ETHREX_DEPLOYER_TDX_CONTRACT_VERIFIER",
        conflicts_with = "tdx_deploy_verifier",
        help_heading = "Verifiers options",
        help = "L1 address of the TDX verifier. If not set, TDX verification will be disabled and not required by the contract."
    )]
    pub tdx_verifier_address: Option<Address>,
    #[arg(
        long = "verifier.deploy-tdx",
        default_value = "false",
        value_name = "BOOLEAN",
        action = ArgAction::SetTrue,
        env = "ETHREX_DEPLOYER_TDX_DEPLOY_VERIFIER",
        help_heading = "Verifiers options",
        help = "If set to true, it will deploy the SP1 verifier contract and use its address.",
    )]
    pub tdx_deploy_verifier: bool,
    #[arg(
        long = "verifier.aligned-aggregator",
        value_name = "ADDRESS",
        env = "ETHREX_DEPLOYER_ALIGNED_AGGREGATOR_ADDRESS",
        help_heading = "Verifiers options",
        help = "L1 address of the Aligned Aggregator. If not set, Aligned verification will be disabled and not required by the contract."
    )]
    pub aligned_aggregator_address: Option<Address>,
    #[arg(
        long,
        value_name = "ADDRESS",
        env = "ETHREX_ON_CHAIN_PROPOSER_OWNER",
        help_heading = "Deployer options",
        help = "Address of the owner of the OnChainProposer contract, who can upgrade the contract."
    )]
    pub on_chain_proposer_owner: Address,
    #[arg(
        long,
        value_name = "ADDRESS",
        env = "ETHREX_BRIDGE_OWNER",
        help_heading = "Deployer options",
        help = "Address of the owner of the CommonBridge contract, who can upgrade the contract."
    )]
    pub bridge_owner: Address,
    #[arg(
        long,
        value_name = "PRIVATE_KEY",
        env = "ETHREX_ON_CHAIN_PROPOSER_OWNER_PK",
        help_heading = "Deployer options",
        help = "Private key of the owner of the OnChainProposer contract. If set, the deployer will send a transaction to accept the ownership.",
        requires = "on_chain_proposer_owner"
    )]
    pub on_chain_proposer_owner_pk: Option<SecretKey>,
    #[arg(
        long,
        default_value_t = format!("{}/../prover/zkvm/interface/sp1/out/riscv32im-succinct-zkvm-vk", env!("CARGO_MANIFEST_DIR")),
        value_name = "PATH",
        env = "ETHREX_SP1_VERIFICATION_KEY_PATH",
        help_heading = "Deployer options",
        help = "Path to the SP1 verification key. This is used for proof verification."
    )]
    pub sp1_vk_path: String,
    #[arg(
        long,
        default_value_t = format!("{}/../prover/zkvm/interface/risc0/out/riscv32im-risc0-vk", env!("CARGO_MANIFEST_DIR")),
        value_name = "PATH",
        env = "ETHREX_RISC0_VERIFICATION_KEY_PATH",
        help_heading = "Deployer options",
        help = "Path to the Risc0 image id / verification key. This is used for proof verification."
    )]
    pub risc0_vk_path: String,
    #[arg(
        long,
        default_value = "false",
        value_name = "BOOLEAN",
        env = "ETHREX_DEPLOYER_DEPLOY_BASED_CONTRACTS",
        action = ArgAction::SetTrue,
        help_heading = "Deployer options",
        help = "If set to true, it will deploy the SequencerRegistry contract and a modified OnChainProposer contract."
    )]
    pub deploy_based_contracts: bool,
    #[arg(
        long,
        value_name = "ADDRESS",
        env = "ETHREX_DEPLOYER_SEQUENCER_REGISTRY_OWNER",
        required_if_eq("deploy_based_contracts", "true"),
        help_heading = "Deployer options",
        help = "Address of the owner of the SequencerRegistry contract, who can upgrade the contract."
    )]
    pub sequencer_registry_owner: Option<Address>,
    #[arg(
        long,
        default_value = "3000",
        env = "ETHREX_ON_CHAIN_PROPOSER_INCUSION_MAX_WAIT",
        help_heading = "Deployer options",
        help = "Deadline in seconds for the sequencer to process a privileged transaction."
    )]
    pub inclusion_max_wait: u64,
    #[arg(
        long,
        default_value = "false",
        env = "ETHREX_USE_COMPILED_GENESIS",
        help_heading = "Deployer options",
        help = "Genesis data is extracted at compile time, used for development"
    )]
    pub use_compiled_genesis: bool,
}

impl Default for DeployerOptions {
    fn default() -> Self {
        Self {
            eth_options: EthOptions::default(),
            #[allow(clippy::unwrap_used)]
            private_key: SecretKey::from_slice(
                H256([
                    0x38, 0x5c, 0x54, 0x64, 0x56, 0xb6, 0xa6, 0x03, 0xa1, 0xcf, 0xca, 0xa9, 0xec,
                    0x94, 0x94, 0xba, 0x48, 0x32, 0xda, 0x08, 0xdd, 0x6b, 0xcf, 0x4d, 0xe9, 0xa7,
                    0x1e, 0x4a, 0x01, 0xb7, 0x49, 0x24,
                ])
                .as_bytes(),
            )
            .unwrap(),
            env_file_path: None,
            deposit_rich: true,
            private_keys_file_path: Some("../../fixtures/keys/private_keys_l1.txt".into()),
            genesis_l1_path: Some("../../fixtures/genesis/l1-dev.json".into()),
            genesis_l2_path: Some("../../fixtures/genesis/l2.json".into()),
            // 0x3d1e15a1a55578f7c920884a9943b3b35d0d885b
            committer_l1_address: H160([
                0x3d, 0x1e, 0x15, 0xa1, 0xa5, 0x55, 0x78, 0xf7, 0xc9, 0x20, 0x88, 0x4a, 0x99, 0x43,
                0xb3, 0xb3, 0x5d, 0x0d, 0x88, 0x5b,
            ]),
            // 0xE25583099BA105D9ec0A67f5Ae86D90e50036425
            proof_sender_l1_address: H160([
                0xe2, 0x55, 0x83, 0x09, 0x9b, 0xa1, 0x05, 0xd9, 0xec, 0x0a, 0x67, 0xf5, 0xae, 0x86,
                0xd9, 0x0e, 0x50, 0x03, 0x64, 0x25,
            ]),
            risc0_verifier_address: Some(H160([
                0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
                0x00, 0x00, 0x00, 0x00, 0x00, 0xaa,
            ])),
            sp1_verifier_address: Some(H160([
                0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
                0x00, 0x00, 0x00, 0x00, 0x00, 0xaa,
            ])),
            sp1_deploy_verifier: false,
            tdx_verifier_address: Some(H160([
                0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
                0x00, 0x00, 0x00, 0x00, 0x00, 0xaa,
            ])),
            tdx_deploy_verifier: false,
            aligned_aggregator_address: Some(H160([
                0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
                0x00, 0x00, 0x00, 0x00, 0x00, 0xaa,
            ])),
            create2_salt: None,
            validium: false,
            // 0x4417092b70a3e5f10dc504d0947dd256b965fc62
            // Private Key: 0x941e103320615d394a55708be13e45994c7d93b932b064dbcb2b511fe3254e2e
            // (also found on fixtures/keys/private_keys_l1.txt)
            on_chain_proposer_owner: H160([
                0x44, 0x17, 0x09, 0x2b, 0x70, 0xa3, 0xe5, 0xf1, 0x0d, 0xc5, 0x04, 0xd0, 0x94, 0x7d,
                0xd2, 0x56, 0xb9, 0x65, 0xfc, 0x62,
            ]),
            // 0x4417092b70a3e5f10dc504d0947dd256b965fc62
            bridge_owner: H160([
                0x44, 0x17, 0x09, 0x2b, 0x70, 0xa3, 0xe5, 0xf1, 0x0d, 0xc5, 0x04, 0xd0, 0x94, 0x7d,
                0xd2, 0x56, 0xb9, 0x65, 0xfc, 0x62,
            ]),
            on_chain_proposer_owner_pk: None,
            sp1_vk_path: format!(
                "{}/../prover/zkvm/interface/sp1/out/riscv32im-succinct-zkvm-vk",
                env!("CARGO_MANIFEST_DIR")
            ),
            risc0_vk_path: format!(
                "{}/../prover/zkvm/interface/risc0/out/riscv32im-risc0-vk",
                env!("CARGO_MANIFEST_DIR")
            ),
            deploy_based_contracts: false,
            sequencer_registry_owner: None,
            inclusion_max_wait: 3000,
            use_compiled_genesis: true,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum DeployerError {
    #[error("The path is not a valid utf-8 string")]
    FailedToGetStringFromPath,
    #[error("Deployer setup error: {0} not set")]
    ConfigValueNotSet(String),
    #[error("Deployer EthClient error: {0}")]
    EthClientError(#[from] EthClientError),
    #[error("Deployer decoding error: {0}")]
    DecodingError(String),
    #[error("Failed to encode calldata: {0}")]
    CalldataEncodeError(#[from] CalldataEncodeError),
    #[error("Failed to deploy contract: {0}")]
    FailedToDeployContract(#[from] DeployError),
    #[error("Deployment subtask failed: {0}")]
    DeploymentSubtaskFailed(String),
    #[error("Internal error: {0}")]
    InternalError(String),
    #[error("IO error: {0}")]
    IO(#[from] std::io::Error),
    #[error(
        "Contract bytecode not found. Make sure to compile the deployer with `COMPILE_CONTRACTS` set."
    )]
    BytecodeNotFound,
    #[error("Failed to read or parse genesis: {0}")]
    GenesisError(String),
}

/// Bytecode of the OnChainProposer contract.
/// This is generated by the [build script](./build.rs).
const ON_CHAIN_PROPOSER_BYTECODE: &[u8] = include_bytes!(concat!(
    env!("OUT_DIR"),
    "/contracts/solc_out/OnChainProposer.bytecode"
));

/// Bytecode of the CommonBridge contract.
/// This is generated by the [build script](./build.rs).
const COMMON_BRIDGE_BYTECODE: &[u8] = include_bytes!(concat!(
    env!("OUT_DIR"),
    "/contracts/solc_out/CommonBridge.bytecode"
));

/// Bytecode of the based OnChainProposer contract.
/// This is generated by the [build script](./build.rs).
const ON_CHAIN_PROPOSER_BASED_BYTECODE: &[u8] = include_bytes!(concat!(
    env!("OUT_DIR"),
    "/contracts/solc_out/OnChainProposerBased.bytecode"
));

/// Bytecode of the SequencerRegistry contract.
/// This is generated by the [build script](./build.rs).
const SEQUENCER_REGISTRY_BYTECODE: &[u8] = include_bytes!(concat!(
    env!("OUT_DIR"),
    "/contracts/solc_out/SequencerRegistry.bytecode"
));

/// Bytecode of the SP1Verifier contract.
/// This is generated by the [build script](./build.rs).
const SP1_VERIFIER_BYTECODE: &[u8] = include_bytes!(concat!(
    env!("OUT_DIR"),
    "/contracts/solc_out/SP1Verifier.bytecode"
));

const INITIALIZE_ON_CHAIN_PROPOSER_SIGNATURE_BASED: &str = "initialize(bool,address,address,address,address,address,bytes32,bytes32,bytes32,address,uint256)";
const INITIALIZE_ON_CHAIN_PROPOSER_SIGNATURE: &str = "initialize(bool,address,address,address,address,address,bytes32,bytes32,bytes32,address[],uint256)";

const INITIALIZE_BRIDGE_ADDRESS_SIGNATURE: &str = "initializeBridgeAddress(address)";
const TRANSFER_OWNERSHIP_SIGNATURE: &str = "transferOwnership(address)";
const ACCEPT_OWNERSHIP_SIGNATURE: &str = "acceptOwnership()";
const BRIDGE_INITIALIZER_SIGNATURE: &str = "initialize(address,address,uint256)";

#[derive(Clone, Copy)]
pub struct ContractAddresses {
    pub on_chain_proposer_address: Address,
    pub bridge_address: Address,
    pub sp1_verifier_address: Address,
    pub risc0_verifier_address: Address,
    pub tdx_verifier_address: Address,
    pub sequencer_registry_address: Address,
    pub aligned_aggregator_address: Address,
}

pub async fn deploy_l1_contracts(
    opts: DeployerOptions,
) -> Result<ContractAddresses, DeployerError> {
    info!("Starting deployer binary");
    let signer: Signer = LocalSigner::new(opts.private_key).into();

    let eth_client = EthClient::new_with_config(
        opts.eth_options
            .rpc_url
            .iter()
            .map(|url| url.as_str())
            .collect(),
        opts.eth_options.max_number_of_retries,
        opts.eth_options.backoff_factor,
        opts.eth_options.min_retry_delay,
        opts.eth_options.max_retry_delay,
        Some(opts.eth_options.maximum_allowed_max_fee_per_gas),
        Some(opts.eth_options.maximum_allowed_max_fee_per_blob_gas),
    )?;

    let contract_addresses = deploy_contracts(&eth_client, &opts, &signer).await?;

    initialize_contracts(contract_addresses, &eth_client, &opts, &signer).await?;

    if opts.deposit_rich {
        let _ = make_deposits(contract_addresses.bridge_address, &eth_client, &opts)
            .await
            .inspect_err(|err| {
                warn!("Failed to make deposits: {err}");
            });
    }

    write_contract_addresses_to_env(contract_addresses, opts.env_file_path)?;
    info!("Deployer binary finished successfully");
    Ok(contract_addresses)
}

async fn deploy_contracts(
    eth_client: &EthClient,
    opts: &DeployerOptions,
    deployer: &Signer,
) -> Result<ContractAddresses, DeployerError> {
    info!("Deploying OnChainProposer");

    let salt = opts.create2_salt.unwrap_or_else(|| H256::random());
    debug!("Salt used: {salt:#x}");

    let bytecode = if opts.deploy_based_contracts {
        ON_CHAIN_PROPOSER_BASED_BYTECODE.to_vec()
    } else {
        ON_CHAIN_PROPOSER_BYTECODE.to_vec()
    };

    if bytecode.is_empty() {
        return Err(DeployerError::BytecodeNotFound);
    }

    let on_chain_proposer_deployment =
        deploy_with_proxy_from_bytecode(deployer, eth_client, &bytecode, salt.as_bytes()).await?;
    info!(
        address =? on_chain_proposer_deployment.implementation_address,
        tx_hash =? on_chain_proposer_deployment.implementation_tx_hash,
        "OnChainProposer implementation deployed"
    );
    info!(
        address =? on_chain_proposer_deployment.proxy_address,
        tx_hash =? on_chain_proposer_deployment.proxy_tx_hash,
        "OnChainProposer proxy deployed"
    );

    info!("Deploying CommonBridge");

    let bridge_deployment = deploy_with_proxy_from_bytecode(
        deployer,
        eth_client,
        COMMON_BRIDGE_BYTECODE,
        salt.as_bytes(),
    )
    .await?;

    info!(
        address =? bridge_deployment.implementation_address,
        tx_hash =? bridge_deployment.implementation_tx_hash,
        "CommonBridge implementation deployed"
    );
    info!(
        address =? bridge_deployment.proxy_address,
        tx_hash =? bridge_deployment.proxy_tx_hash,
        "CommonBridge proxy deployed"
    );

    let sequencer_registry_deployment = if opts.deploy_based_contracts {
        info!("Deploying SequencerRegistry");

        let sequencer_registry_deployment = deploy_with_proxy_from_bytecode(
            deployer,
            eth_client,
            SEQUENCER_REGISTRY_BYTECODE,
            salt.as_bytes(),
        )
        .await?;

        info!(
            "SequencerRegistry deployed:\n  Proxy -> address={:#x}, tx_hash={:#x}\n  Impl  -> address={:#x}, tx_hash={:#x}",
            sequencer_registry_deployment.proxy_address,
            sequencer_registry_deployment.proxy_tx_hash,
            sequencer_registry_deployment.implementation_address,
            sequencer_registry_deployment.implementation_tx_hash,
        );
        sequencer_registry_deployment
    } else {
        Default::default()
    };

    // TODO: Add Risc0Verifier deployment
    let risc0_verifier_address = opts.risc0_verifier_address.unwrap_or(DEV_MODE_ADDRESS);

    let sp1_verifier_address = if opts.sp1_deploy_verifier {
        info!("Deploying SP1 verifier");
        deploy_contract_from_bytecode(
            &[],
            SP1_VERIFIER_BYTECODE,
            deployer,
            salt.as_bytes(),
            eth_client,
        )
        .await
        .map(|(tx_hash, address)| {
            info!(?address, ?tx_hash, "SP1 verifier deployed");
            address
        })?
    } else {
        opts.sp1_verifier_address.unwrap_or(DEV_MODE_ADDRESS)
    };

    let tdx_verifier_address = if opts.tdx_deploy_verifier {
        info!("Deploying TDX verifier");
        deploy_tdx_contracts(opts, on_chain_proposer_deployment.proxy_address)
            .inspect(|address| info!(?address, "TDX verifier deployed"))?
    } else {
        opts.tdx_verifier_address.unwrap_or(DEV_MODE_ADDRESS)
    };

    let aligned_aggregator_address = opts.aligned_aggregator_address.unwrap_or(DEV_MODE_ADDRESS);

    debug!(
        on_chain_proposer_proxy_address = ?on_chain_proposer_deployment.proxy_address,
        bridge_proxy_address = ?bridge_deployment.proxy_address,
        on_chain_proposer_implementation_address = ?on_chain_proposer_deployment.implementation_address,
        bridge_implementation_address = ?bridge_deployment.implementation_address,
        sp1_verifier_address = ?sp1_verifier_address,
        risc0_verifier_address = ?risc0_verifier_address,
        tdx_verifier_address = ?tdx_verifier_address,
        "Contracts deployed"
    );

    Ok(ContractAddresses {
        on_chain_proposer_address: on_chain_proposer_deployment.proxy_address,
        bridge_address: bridge_deployment.proxy_address,
        sp1_verifier_address,
        risc0_verifier_address,
        tdx_verifier_address,
        sequencer_registry_address: sequencer_registry_deployment.proxy_address,
        aligned_aggregator_address,
    })
}

fn deploy_tdx_contracts(
    opts: &DeployerOptions,
    on_chain_proposer: Address,
) -> Result<Address, DeployerError> {
    Command::new("make")
        .arg("deploy-all")
        .env("PRIVATE_KEY", hex::encode(opts.private_key.as_ref()))
        // TODO: This can panic
        .env("RPC_URL", &opts.eth_options.rpc_url[0])
        .env("ON_CHAIN_PROPOSER", format!("{on_chain_proposer:#x}"))
        .current_dir("tee/contracts")
        .stdout(Stdio::null())
        .spawn()
        .map_err(|err| {
            DeployerError::DeploymentSubtaskFailed(format!("Failed to spawn make: {err}"))
        })?
        .wait()
        .map_err(|err| {
            DeployerError::DeploymentSubtaskFailed(format!("Failed to wait for make: {err}"))
        })?;

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

fn read_vk(path: &str) -> Bytes {
    let Ok(str) = std::fs::read_to_string(path) else {
        warn!(
            ?path,
            "Failed to read verification key file, will use 0x00..00, this is expected in dev mode"
        );
        return Bytes::from(vec![0u8; 32]);
    };

    let cleaned = str.trim().strip_prefix("0x").unwrap_or(&str);

    hex::decode(cleaned).map(Bytes::from).unwrap_or_else(|e| {
        warn!(
            ?path,
            "Failed to decode hex string, will use 0x00..00, this is expected in dev mode: {}", e
        );
        Bytes::from(vec![0u8; 32])
    })
}

async fn initialize_contracts(
    contract_addresses: ContractAddresses,
    eth_client: &EthClient,
    opts: &DeployerOptions,
    initializer: &Signer,
) -> Result<(), DeployerError> {
    trace!("Initializing contracts");

    trace!(committer_l1_address = %opts.committer_l1_address, "Using committer L1 address for OnChainProposer initialization");

    let genesis: Genesis = if opts.use_compiled_genesis {
        serde_json::from_str(LOCAL_DEVNETL2_GENESIS_CONTENTS)
            .map_err(|e| DeployerError::GenesisError(e.to_string()))?
    } else {
        read_genesis_file(
            &opts
                .genesis_l2_path
                .clone()
                .expect("L2 genesis file required but not provided."),
        )?
    };

    let sp1_vk = read_vk(&opts.sp1_vk_path);
    let risc0_vk = read_vk(&opts.risc0_vk_path);

    let deployer_address = get_address_from_secret_key(&opts.private_key)?;

    info!("Initializing OnChainProposer");

    if opts.deploy_based_contracts {
        // Initialize OnChainProposer with Based config and SequencerRegistry
        let calldata_values = vec![
            Value::Bool(opts.validium),
            Value::Address(deployer_address),
            Value::Address(contract_addresses.risc0_verifier_address),
            Value::Address(contract_addresses.sp1_verifier_address),
            Value::Address(contract_addresses.tdx_verifier_address),
            Value::Address(contract_addresses.aligned_aggregator_address),
            Value::FixedBytes(sp1_vk),
            Value::FixedBytes(risc0_vk),
            Value::FixedBytes(genesis.compute_state_root().0.to_vec().into()),
            Value::Address(contract_addresses.sequencer_registry_address),
            Value::Uint(genesis.config.chain_id.into()),
        ];

        trace!(calldata_values = ?calldata_values, "OnChainProposer initialization calldata values");
        let on_chain_proposer_initialization_calldata = encode_calldata(
            INITIALIZE_ON_CHAIN_PROPOSER_SIGNATURE_BASED,
            &calldata_values,
        )?;

        let deployer = Signer::Local(LocalSigner::new(opts.private_key));

        let initialize_tx_hash = initialize_contract(
            contract_addresses.on_chain_proposer_address,
            on_chain_proposer_initialization_calldata,
            &deployer,
            eth_client,
        )
        .await?;

        info!(tx_hash = %format!("{initialize_tx_hash:#x}"), "OnChainProposer initialized");

        info!("Initializing SequencerRegistry");
        let initialize_tx_hash = {
            let calldata_values = vec![
                Value::Address(opts.sequencer_registry_owner.ok_or(
                    DeployerError::ConfigValueNotSet("--sequencer-registry-owner".to_string()),
                )?),
                Value::Address(contract_addresses.on_chain_proposer_address),
            ];
            let sequencer_registry_initialization_calldata =
                encode_calldata("initialize(address,address)", &calldata_values)?;

            initialize_contract(
                contract_addresses.sequencer_registry_address,
                sequencer_registry_initialization_calldata,
                &deployer,
                eth_client,
            )
            .await?
        };
        info!(tx_hash = %format!("{initialize_tx_hash:#x}"), "SequencerRegistry initialized");
    } else {
        // Initialize only OnChainProposer without Based config
        let calldata_values = vec![
            Value::Bool(opts.validium),
            Value::Address(deployer_address),
            Value::Address(contract_addresses.risc0_verifier_address),
            Value::Address(contract_addresses.sp1_verifier_address),
            Value::Address(contract_addresses.tdx_verifier_address),
            Value::Address(contract_addresses.aligned_aggregator_address),
            Value::FixedBytes(sp1_vk),
            Value::FixedBytes(risc0_vk),
            Value::FixedBytes(genesis.compute_state_root().0.to_vec().into()),
            Value::Array(vec![
                Value::Address(opts.committer_l1_address),
                Value::Address(opts.proof_sender_l1_address),
            ]),
            Value::Uint(genesis.config.chain_id.into()),
        ];
        trace!(calldata_values = ?calldata_values, "OnChainProposer initialization calldata values");
        let on_chain_proposer_initialization_calldata =
            encode_calldata(INITIALIZE_ON_CHAIN_PROPOSER_SIGNATURE, &calldata_values)?;

        let initialize_tx_hash = initialize_contract(
            contract_addresses.on_chain_proposer_address,
            on_chain_proposer_initialization_calldata,
            initializer,
            eth_client,
        )
        .await?;
        info!(tx_hash = %format!("{initialize_tx_hash:#x}"), "OnChainProposer initialized");
    }

    let initialize_bridge_address_tx_hash = {
        let calldata_values = vec![Value::Address(contract_addresses.bridge_address)];
        let on_chain_proposer_initialization_calldata =
            encode_calldata(INITIALIZE_BRIDGE_ADDRESS_SIGNATURE, &calldata_values)?;

        initialize_contract(
            contract_addresses.on_chain_proposer_address,
            on_chain_proposer_initialization_calldata,
            initializer,
            eth_client,
        )
        .await?
    };

    info!(
        tx_hash = %format!("{initialize_bridge_address_tx_hash:#x}"),
        "OnChainProposer bridge address initialized"
    );

    if opts.on_chain_proposer_owner != initializer.address() {
        let transfer_ownership_tx_hash = {
            let owener_transfer_calldata = encode_calldata(
                TRANSFER_OWNERSHIP_SIGNATURE,
                &[Value::Address(opts.on_chain_proposer_owner)],
            )?;

            initialize_contract(
                contract_addresses.on_chain_proposer_address,
                owener_transfer_calldata,
                initializer,
                eth_client,
            )
            .await?
        };

        if let Some(owner_pk) = opts.on_chain_proposer_owner_pk {
            let signer = Signer::Local(LocalSigner::new(owner_pk));
            let accept_ownership_calldata = encode_calldata(ACCEPT_OWNERSHIP_SIGNATURE, &[])?;
            let accept_tx = eth_client
                .build_eip1559_transaction(
                    contract_addresses.on_chain_proposer_address,
                    opts.on_chain_proposer_owner,
                    accept_ownership_calldata.into(),
                    Overrides::default(),
                )
                .await?;
            let accept_tx_hash = send_eip1559_transaction(eth_client, &accept_tx, &signer).await?;

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
            Value::Address(contract_addresses.on_chain_proposer_address),
            Value::Uint(opts.inclusion_max_wait.into()),
        ];
        let bridge_initialization_calldata =
            encode_calldata(BRIDGE_INITIALIZER_SIGNATURE, &calldata_values)?;

        initialize_contract(
            contract_addresses.bridge_address,
            bridge_initialization_calldata,
            initializer,
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

    let genesis: Genesis = if opts.use_compiled_genesis {
        serde_json::from_str(LOCAL_DEVNET_GENESIS_CONTENTS)
            .map_err(|e| DeployerError::GenesisError(e.to_string()))?
    } else {
        read_genesis_file(
            &opts
                .genesis_l1_path
                .clone()
                .expect("L1 genesis file required but not provided."),
        )?
    };

    let pks = read_to_string(opts.private_keys_file_path.clone().ok_or(
        DeployerError::ConfigValueNotSet("--private-keys-file-path".to_string()),
    )?)
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
        let signer = Signer::Local(LocalSigner::new(secret_key));

        let Some(_) = genesis.alloc.get(&signer.address()) else {
            debug!(
                address =? signer.address(),
                "Skipping deposit for address as it is not in the genesis file"
            );
            continue;
        };

        let get_balance = eth_client
            .get_balance(signer.address(), BlockIdentifier::Tag(BlockTag::Latest))
            .await?;
        let value_to_deposit = get_balance
            .checked_div(U256::from_str("2").unwrap_or(U256::zero()))
            .unwrap_or(U256::zero());

        let overrides = Overrides {
            value: Some(value_to_deposit),
            from: Some(signer.address()),
            ..Overrides::default()
        };

        let build = eth_client
            .build_eip1559_transaction(bridge, signer.address(), Bytes::new(), overrides)
            .await?;

        match send_eip1559_transaction(eth_client, &build, &signer).await {
            Ok(hash) => {
                info!(
                    address =? signer.address(),
                    ?value_to_deposit,
                    ?hash,
                    "Deposit transaction sent to L1"
                );
            }
            Err(e) => {
                error!(address =? signer.address(), ?value_to_deposit, "Failed to deposit");
                return Err(DeployerError::EthClientError(e));
            }
        }
    }
    trace!("Deposits finished");
    Ok(())
}

fn write_contract_addresses_to_env(
    contract_addresses: ContractAddresses,
    env_file_path: Option<PathBuf>,
) -> Result<(), DeployerError> {
    trace!("Writing contract addresses to .env file");
    let env_file_path =
        env_file_path.unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.env")); // ethrex/cmd/.env

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
        "ETHREX_COMMITTER_ON_CHAIN_PROPOSER_ADDRESS={:#x}",
        contract_addresses.on_chain_proposer_address
    )?;
    writeln!(
        writer,
        "ETHREX_WATCHER_BRIDGE_ADDRESS={:#x}",
        contract_addresses.bridge_address
    )?;
    writeln!(
        writer,
        "ETHREX_DEPLOYER_SP1_CONTRACT_VERIFIER={:#x}",
        contract_addresses.sp1_verifier_address
    )?;

    writeln!(
        writer,
        "ETHREX_DEPLOYER_RISC0_CONTRACT_VERIFIER={:#x}",
        contract_addresses.risc0_verifier_address
    )?;
    writeln!(
        writer,
        "ETHREX_DEPLOYER_ALIGNED_AGGREGATOR_ADDRESS={:#x}",
        contract_addresses.aligned_aggregator_address
    )?;
    writeln!(
        writer,
        "ETHREX_DEPLOYER_TDX_CONTRACT_VERIFIER={:#x}",
        contract_addresses.tdx_verifier_address
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
    writeln!(
        writer,
        "ETHREX_DEPLOYER_SEQUENCER_REGISTRY_ADDRESS={:#x}",
        contract_addresses.sequencer_registry_address
    )?;
    trace!(?env_file_path, "Contract addresses written to .env");
    Ok(())
}

pub fn read_genesis_file(path: &PathBuf) -> Result<Genesis, DeployerError> {
    let file = File::open(path).map_err(|e| DeployerError::GenesisError(e.to_string()))?;
    let genesis_reader = BufReader::new(file);
    serde_json::from_reader(genesis_reader).map_err(|e| DeployerError::GenesisError(e.to_string()))
}
