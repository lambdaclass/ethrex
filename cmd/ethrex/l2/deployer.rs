use std::{
    fs::{File, OpenOptions, read_to_string},
    io::{BufWriter, Write},
    path::PathBuf,
    process::{Command, Stdio},
    str::FromStr,
};

use bytes::Bytes;
use clap::Parser;
use ethrex_common::H256;
use ethrex_common::utils::keccak;
use ethrex_common::{
    Address, U256,
    types::{Genesis, TxType},
};
use ethrex_l2::{sequencer::utils::get_git_commit_hash, utils::test_data_io::read_genesis_file};
use ethrex_l2_common::{calldata::Value, prover::ProverType, utils::get_address_from_secret_key};
use ethrex_l2_rpc::signer::{LocalSigner, Signer};
use ethrex_l2_sdk::{
    build_generic_tx, calldata::encode_calldata, create2_deploy_from_bytecode_no_wait,
    initialize_contract_no_wait, send_generic_transaction, wait_for_transaction_receipt,
};
use ethrex_l2_sdk::{deploy_with_proxy_from_bytecode_no_wait, register_fee_token_no_wait};
use ethrex_rpc::{
    EthClient,
    clients::Overrides,
    types::block_identifier::{BlockIdentifier, BlockTag},
};
use tracing::{debug, error, info, trace, warn};

use ethrex_l2_sdk::DeployError;
use ethrex_rpc::clients::{EthClientError, eth::errors::CalldataEncodeError};

use clap::ArgAction;
use ethrex_common::H160;
use hex::FromHexError;
use secp256k1::SecretKey;
use url::Url;

use ethrex_config::networks::{
    LOCAL_DEVNET_GENESIS_CONTENTS, LOCAL_DEVNET_PRIVATE_KEYS, LOCAL_DEVNETL2_GENESIS_CONTENTS,
};

#[derive(Parser)]
pub struct DeployerOptions {
    #[arg(
        long = "eth-rpc-url",
        value_name = "RPC_URL",
        env = "ETHREX_ETH_RPC_URL",
        help_heading = "Eth options"
    )]
    pub rpc_url: Url,
    #[arg(
        long,
        default_value = "10000000000",
        value_name = "UINT64",
        env = "ETHREX_MAXIMUM_ALLOWED_MAX_FEE_PER_GAS",
        help_heading = "Eth options"
    )]
    pub maximum_allowed_max_fee_per_gas: u64,
    #[arg(
        long,
        default_value = "10000000000",
        value_name = "UINT64",
        env = "ETHREX_MAXIMUM_ALLOWED_MAX_FEE_PER_BLOB_GAS",
        help_heading = "Eth options"
    )]
    pub maximum_allowed_max_fee_per_blob_gas: u64,
    #[arg(
        long,
        value_name = "PRIVATE_KEY",
        value_parser = parse_private_key,
        env = "ETHREX_DEPLOYER_L1_PRIVATE_KEY",
        help_heading = "Deployer options",
        help = "Private key corresponding of a funded account that will be used for L1 contract deployment.",
    )]
    pub private_key: SecretKey,
    #[arg(
        long,
        default_value = "10",
        value_name = "UINT64",
        env = "ETHREX_ETH_MAX_NUMBER_OF_RETRIES",
        help_heading = "Eth options"
    )]
    pub max_number_of_retries: u64,
    #[arg(
        long,
        default_value = "2",
        value_name = "UINT64",
        env = "ETHREX_ETH_BACKOFF_FACTOR",
        help_heading = "Eth options"
    )]
    pub backoff_factor: u64,
    #[arg(
        long,
        default_value = "96",
        value_name = "UINT64",
        env = "ETHREX_ETH_MIN_RETRY_DELAY",
        help_heading = "Eth options"
    )]
    pub min_retry_delay: u64,
    #[arg(
        long,
        default_value = "1800",
        value_name = "UINT64",
        env = "ETHREX_ETH_MAX_RETRY_DELAY",
        help_heading = "Eth options"
    )]
    pub max_retry_delay: u64,
    #[arg(
        long,
        value_name = "PATH",
        env = "ETHREX_DEPLOYER_ENV_FILE_PATH",
        help_heading = "Deployer options",
        help = "Path to the .env file."
    )]
    pub env_file_path: Option<PathBuf>,
    #[arg(
        long,
        default_value = "false",
        value_name = "BOOLEAN",
        env = "ETHREX_DEPLOYER_DEPLOY_RICH",
        action = ArgAction::SetTrue,
        help_heading = "Deployer options",
        help = "If set to true, it will deposit ETH from L1 rich wallets to L2 accounts."
    )]
    pub deposit_rich: bool,
    #[arg(
        long,
        value_name = "PATH",
        env = "ETHREX_DEPLOYER_PRIVATE_KEYS_FILE_PATH",
        required_if_eq("deposit_rich", "true"),
        help_heading = "Deployer options",
        help = "Path to the file containing the private keys of the rich accounts. The default is ../../fixtures/keys/private_keys_l1.txt"
    )]
    pub private_keys_file_path: Option<PathBuf>,
    #[arg(
        long,
        value_name = "PATH",
        env = "ETHREX_DEPLOYER_GENESIS_L1_PATH",
        required_if_eq("deposit_rich", "true"),
        help_heading = "Deployer options",
        help = "Path to the genesis file. The default is ../../fixtures/genesis/l1-dev.json"
    )]
    pub genesis_l1_path: Option<PathBuf>,
    #[arg(
        long,
        value_name = "PATH",
        env = "ETHREX_DEPLOYER_GENESIS_L2_PATH",
        help_heading = "Deployer options",
        help = "Path to the l2 genesis file. The default is ../../fixtures/genesis/l2.json"
    )]
    pub genesis_l2_path: PathBuf,
    #[arg(
        long = "committer.l1-address",
        default_value = "0x3d1e15a1a55578f7c920884a9943b3b35d0d885b",
        value_name = "ADDRESS",
        env = "ETHREX_DEPLOYER_COMMITTER_L1_ADDRESS",
        help_heading = "Deployer options",
        help = "Address of the L1 committer account. This is the address of the account that commits the batches in L1."
    )]
    pub committer_l1_address: Address,
    #[arg(
        long = "proof-sender.l1-address",
        default_value = "0xE25583099BA105D9ec0A67f5Ae86D90e50036425",
        value_name = "ADDRESS",
        env = "ETHREX_DEPLOYER_PROOF_SENDER_L1_ADDRESS",
        help_heading = "Deployer options",
        help = "Address of the L1 proof sender account. This is the address of the account that sends the proofs to be verified in L1."
    )]
    pub proof_sender_l1_address: Address,
    #[arg(
        long,
        default_value = "false",
        value_name = "BOOLEAN",
        env = "ETHREX_L2_RISC0",
        action = ArgAction::Set,
        help_heading = "Deployer options",
        help = "If true, L2 will require Risc0 proofs to validate batch proofs and settle state."
    )]
    pub risc0: bool,
    #[arg(
        long = "risc0.verifier-address",
        value_name = "ADDRESS",
        env = "ETHREX_DEPLOYER_RISC0_VERIFIER_ADDRESS",
        help_heading = "Deployer options"
    )]
    pub risc0_verifier_address: Option<Address>,
    #[arg(
        long,
        default_value = "false",
        value_name = "BOOLEAN",
        env = "ETHREX_L2_SP1",
        action = ArgAction::Set,
        help_heading = "Deployer options",
        help = "If true, L2 will require SP1 proofs to validate batch proofs and settle state."
    )]
    pub sp1: bool,
    #[arg(
        long = "sp1.verifier-address",
        value_name = "ADDRESS",
        env = "ETHREX_DEPLOYER_SP1_VERIFIER_ADDRESS",
        help_heading = "Deployer options",
        help = "If no verifier address is provided, contract deployer will deploy the SP1 verifier"
    )]
    pub sp1_verifier_address: Option<Address>,
    #[arg(
        long,
        default_value = "false",
        value_name = "BOOLEAN",
        env = "ETHREX_L2_TDX",
        action = ArgAction::Set,
        help_heading = "Deployer options",
        help = "If true, L2 will require TDX proofs to validate batch proofs and settle state."
    )]
    pub tdx: bool,
    #[arg(
        long = "tdx.verifier-address",
        value_name = "ADDRESS",
        env = "ETHREX_DEPLOYER_TDX_VERIFIER_ADDRESS",
        help_heading = "Deployer options",
        help = "If no verifier address is provided, contract deployer will deploy the TDX verifier"
    )]
    pub tdx_verifier_address: Option<Address>,
    #[arg(
        long,
        default_value = "false",
        value_name = "BOOLEAN",
        env = "ETHREX_L2_ALIGNED",
        action = ArgAction::Set,
        help_heading = "Deployer options",
        help = "If true, L2 will verify proofs using Aligned Layer instead of smart contract verifiers."
    )]
    pub aligned: bool,
    #[arg(
        long = "aligned.aggregator-address",
        value_name = "ADDRESS",
        env = "ETHREX_DEPLOYER_ALIGNED_AGGREGATOR_ADDRESS",
        help_heading = "Deployer options"
    )]
    pub aligned_aggregator_address: Option<Address>,
    #[arg(
        long,
        default_value = "false",
        value_name = "BOOLEAN",
        action = ArgAction::SetTrue,
        env = "ETHREX_DEPLOYER_RANDOMIZE_CONTRACT_DEPLOYMENT",
        help_heading = "Deployer options",
        help = "If set to false, the deployed contract addresses will be deterministic."
    )]
    pub randomize_contract_deployment: bool,
    #[arg(
        long,
        default_value = "false",
        value_name = "BOOLEAN",
        env = "ETHREX_L2_VALIDIUM",
        action = ArgAction::Set,
        help_heading = "Deployer options",
        help = "If true, L2 will run on validium mode as opposed to the default rollup mode, meaning it will not publish blobs to the L1."
    )]
    pub validium: bool,
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
        value_parser = parse_private_key,
        env = "ETHREX_BRIDGE_OWNER_PK",
        help_heading = "Deployer options",
        help = "Private key of the owner of the CommonBridge contract. If set, the deployer will send a transaction to accept the ownership.",
        requires = "bridge_owner"
    )]
    pub bridge_owner_pk: Option<SecretKey>,
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
        value_name = "PATH",
        env = "ETHREX_SP1_VERIFICATION_KEY_PATH",
        help_heading = "Deployer options",
        help = "Path to the SP1 verification key. This is used for proof verification."
    )]
    pub sp1_vk_path: Option<String>,
    #[arg(
        long,
        value_name = "PATH",
        env = "ETHREX_RISC0_VERIFICATION_KEY_PATH",
        help_heading = "Deployer options",
        help = "Path to the Risc0 image id / verification key. This is used for proof verification."
    )]
    pub risc0_vk_path: Option<String>,
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
        env = "ETHREX_ON_CHAIN_PROPOSER_INCLUSION_MAX_WAIT",
        help_heading = "Deployer options",
        help = "Deadline in seconds for the sequencer to process a privileged transaction."
    )]
    pub inclusion_max_wait: u64,
    #[arg(
        long,
        default_value = "false",
        env = "ETHREX_USE_COMPILED_GENESIS",
        action = ArgAction::Set,
        help_heading = "Deployer options",
        help = "Genesis data is extracted at compile time, used for development"
    )]
    pub use_compiled_genesis: bool,
    #[arg(
        long = "router.deploy",
        default_value = "false",
        env = "ETHREX_SHARED_BRIDGE_DEPLOY_ROUTER",
        help_heading = "Deployer options",
        help = "If set, the deployer will deploy the shared bridge router contract. Default to false",
        conflicts_with = "router"
    )]
    pub deploy_router: bool,
    #[arg(
        long = "router.address",
        value_name = "ADDRESS",
        env = "ETHREX_SHARED_BRIDGE_ROUTER_ADDRESS",
        help_heading = "Deployer options",
        help = "The address of the shared bridge router"
    )]
    pub router: Option<Address>,
    #[arg(
        long,
        value_name = "ADDRESS",
        env = "ETHREX_DEPLOYER_INITIAL_FEE_TOKEN",
        help_heading = "Deployer options",
        help = "This address will be registered as an initial fee token"
    )]
    pub initial_fee_token: Option<Address>,
}

impl Default for DeployerOptions {
    fn default() -> Self {
        Self {
            rpc_url: Url::parse("http://localhost:8545")
                .expect("Unreachable error. URL is hardcoded"),
            maximum_allowed_max_fee_per_gas: 10_000_000_000,
            maximum_allowed_max_fee_per_blob_gas: 10_000_000_000,
            max_number_of_retries: 10,
            backoff_factor: 2,
            min_retry_delay: 96,
            max_retry_delay: 1800,
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
            env_file_path: Some(PathBuf::from(".env")),
            deposit_rich: true,
            private_keys_file_path: None,
            genesis_l1_path: Some("../../fixtures/genesis/l1-dev.json".into()),
            genesis_l2_path: "../../fixtures/genesis/l2.json".into(),
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
            risc0: false,
            risc0_verifier_address: None,
            sp1: false,
            sp1_verifier_address: None,
            tdx: false,
            tdx_verifier_address: None,
            aligned: false,
            aligned_aggregator_address: None,
            randomize_contract_deployment: false,
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
            // Private Key: 0x941e103320615d394a55708be13e45994c7d93b932b064dbcb2b511fe3254e2e
            bridge_owner_pk: Some(
                SecretKey::from_slice(
                    H256::from_str(
                        "941e103320615d394a55708be13e45994c7d93b932b064dbcb2b511fe3254e2e",
                    )
                    .expect("Bridge owner private key is a valid hex string")
                    .as_bytes(),
                )
                .expect("Bridge owner private key is valid"),
            ),
            on_chain_proposer_owner_pk: None,
            sp1_vk_path: None,
            risc0_vk_path: None,
            deploy_based_contracts: false,
            sequencer_registry_owner: None,
            inclusion_max_wait: 3000,
            use_compiled_genesis: true,
            router: None,
            deploy_router: false,
            initial_fee_token: None,
        }
    }
}

pub fn parse_private_key(s: &str) -> eyre::Result<SecretKey> {
    Ok(SecretKey::from_slice(&parse_hex(s)?)?)
}

pub fn parse_hex(s: &str) -> eyre::Result<Bytes, FromHexError> {
    match s.strip_prefix("0x") {
        Some(s) => hex::decode(s).map(Into::into),
        None => hex::decode(s).map(Into::into),
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
    #[error("Failed to parse genesis")]
    Genesis,
    #[error("Transaction receipt error")]
    TransactionReceiptError,
}

/// Bytecode of the Router contract.
/// This is generated by the [build script](./build.rs).
const ROUTER_BYTECODE: &[u8] = include_bytes!(concat!(
    env!("OUT_DIR"),
    "/contracts/solc_out/Router.bytecode"
));

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

const INITIALIZE_ON_CHAIN_PROPOSER_SIGNATURE_BASED: &str = "initialize(bool,address,bool,bool,bool,bool,address,address,address,address,bytes32,bytes32,bytes32,bytes32,address,uint256)";
const INITIALIZE_ON_CHAIN_PROPOSER_SIGNATURE: &str = "initialize(bool,address,bool,bool,bool,bool,address,address,address,address,bytes32,bytes32,bytes32,bytes32,address[],uint256)";

const INITIALIZE_BRIDGE_ADDRESS_SIGNATURE: &str = "initializeBridgeAddress(address)";
const TRANSFER_OWNERSHIP_SIGNATURE: &str = "transferOwnership(address)";
const ACCEPT_OWNERSHIP_SIGNATURE: &str = "acceptOwnership()";
const BRIDGE_INITIALIZER_SIGNATURE: &str = "initialize(address,address,uint256,address, uint256)";
const ROUTER_INITIALIZER_SIGNATURE: &str = "initialize(address)";
const ROUTER_REGISTER_SIGNATURE: &str = "register(uint256,address)";

// Gas limit for deploying and initializing contracts
// Needed to avoid estimating gas of initializations when the
// deploy transaction is still pending
const TRANSACTION_GAS_LIMIT: u64 = 10_000_000;

#[derive(Clone)]
pub struct ContractAddresses {
    pub on_chain_proposer_address: Address,
    pub bridge_address: Address,
    pub sp1_verifier_address: Address,
    pub risc0_verifier_address: Address,
    pub tdx_verifier_address: Address,
    pub sequencer_registry_address: Address,
    pub aligned_aggregator_address: Address,
    pub router: Option<Address>,
}

pub async fn deploy_l1_contracts(
    opts: DeployerOptions,
) -> Result<ContractAddresses, DeployerError> {
    info!("Starting deployer binary");
    let signer: Signer = LocalSigner::new(opts.private_key).into();

    let eth_client = EthClient::new_with_config(
        vec![opts.rpc_url.clone()],
        opts.max_number_of_retries,
        opts.backoff_factor,
        opts.min_retry_delay,
        opts.max_retry_delay,
        Some(opts.maximum_allowed_max_fee_per_gas),
        Some(opts.maximum_allowed_max_fee_per_blob_gas),
    )?;

    info!("Deploying contracts");

    let genesis: Genesis = if opts.use_compiled_genesis {
        serde_json::from_str(LOCAL_DEVNETL2_GENESIS_CONTENTS).map_err(|_| DeployerError::Genesis)?
    } else {
        read_genesis_file(
            opts.genesis_l2_path
                .to_str()
                .ok_or(DeployerError::FailedToGetStringFromPath)?,
        )
    };

    let (contract_addresses, deploy_tx_hashes) =
        deploy_contracts(&eth_client, &opts, &signer).await?;

    info!("Initializing contracts");

    let initialize_tx_hashes = initialize_contracts(
        contract_addresses.clone(),
        &eth_client,
        &opts,
        &genesis,
        &signer,
    )
    .await?;

    info!("Waiting for transactions receipts");

    for tx_hash in deploy_tx_hashes.iter().chain(initialize_tx_hashes.iter()) {
        if *tx_hash == H256::default() {
            continue;
        }
        let receipt = wait_for_transaction_receipt(*tx_hash, &eth_client, 100).await?;
        if !receipt.receipt.status {
            error!("Receipt status is false for tx_hash: {tx_hash:#x}");
            return Err(DeployerError::TransactionReceiptError);
        }
    }

    if contract_addresses.router.is_some() {
        let _ = register_chain(
            &eth_client,
            contract_addresses.clone(),
            genesis.config.chain_id,
            &signer,
        )
        .await
        .inspect_err(|err| {
            warn!(%err, "Could not register chain in shared bridge router");
        });
    }

    if opts.deposit_rich {
        let _ = make_deposits(contract_addresses.bridge_address, &eth_client, &opts)
            .await
            .inspect_err(|err| {
                warn!("Failed to make deposits: {err}");
            });
    }

    write_contract_addresses_to_env(contract_addresses.clone(), opts.env_file_path)?;
    info!("Deployer binary finished successfully");
    Ok(contract_addresses)
}

lazy_static::lazy_static! {
    static ref SALT: std::sync::Mutex<H256>  = std::sync::Mutex::new(H256::zero());
}

async fn deploy_contracts(
    eth_client: &EthClient,
    opts: &DeployerOptions,
    deployer: &Signer,
) -> Result<(ContractAddresses, Vec<H256>), DeployerError> {
    trace!("Deploying contracts");

    let gas_price = eth_client
        .get_gas_price_with_extra(20)
        .await?
        .try_into()
        .map_err(|_| {
            EthClientError::InternalError("Failed to convert gas_price to a u64".to_owned())
        })?;

    let mut nonce = eth_client
        .get_nonce(deployer.address(), BlockIdentifier::Tag(BlockTag::Latest))
        .await?;

    let salt = if opts.randomize_contract_deployment {
        H256::random().as_bytes().to_vec()
    } else {
        SALT.lock()
            .map_err(|_| DeployerError::InternalError("failed unwrapping salt lock".to_string()))?
            .as_bytes()
            .to_vec()
    };

    let router_deployment = if opts.deploy_router {
        info!("Deploying Router");

        let bytecode = ROUTER_BYTECODE.to_vec();
        if bytecode.is_empty() {
            return Err(DeployerError::BytecodeNotFound);
        }

        let router_deployment = deploy_with_proxy_from_bytecode_no_wait(
            deployer,
            eth_client,
            &bytecode,
            &salt,
            Overrides {
                nonce: Some(nonce),
                gas_limit: Some(TRANSACTION_GAS_LIMIT),
                max_fee_per_gas: Some(gas_price),
                max_priority_fee_per_gas: Some(gas_price),
                ..Default::default()
            },
        )
        .await?;
        info!(
            "Router deployed:\n  Proxy -> address={:#x}, tx_hash={:#x}\n  Impl  -> address={:#x}, tx_hash={:#x}",
            router_deployment.proxy_address,
            router_deployment.proxy_tx_hash,
            router_deployment.implementation_address,
            router_deployment.implementation_tx_hash,
        );
        nonce += 2;

        router_deployment
    } else {
        Default::default()
    };

    info!("Deploying OnChainProposer");

    trace!("Attempting to deploy OnChainProposer contract");
    let bytecode = if opts.deploy_based_contracts {
        ON_CHAIN_PROPOSER_BASED_BYTECODE.to_vec()
    } else {
        ON_CHAIN_PROPOSER_BYTECODE.to_vec()
    };

    if bytecode.is_empty() {
        return Err(DeployerError::BytecodeNotFound);
    }

    let on_chain_proposer_deployment = deploy_with_proxy_from_bytecode_no_wait(
        deployer,
        eth_client,
        &bytecode,
        &salt,
        Overrides {
            nonce: Some(nonce),
            gas_limit: Some(TRANSACTION_GAS_LIMIT),
            max_fee_per_gas: Some(gas_price),
            max_priority_fee_per_gas: Some(gas_price),
            ..Default::default()
        },
    )
    .await?;

    // We can increase the nonce after each deployment since the deployer is the same
    nonce += 2;

    info!(
        "OnChainProposer deployed:\n  Proxy -> address={:#x}, tx_hash={:#x}\n  Impl  -> address={:#x}, tx_hash={:#x}",
        on_chain_proposer_deployment.proxy_address,
        on_chain_proposer_deployment.proxy_tx_hash,
        on_chain_proposer_deployment.implementation_address,
        on_chain_proposer_deployment.implementation_tx_hash,
    );

    info!("Deploying CommonBridge");

    let bridge_deployment = deploy_with_proxy_from_bytecode_no_wait(
        deployer,
        eth_client,
        COMMON_BRIDGE_BYTECODE,
        &salt,
        Overrides {
            nonce: Some(nonce),
            gas_limit: Some(TRANSACTION_GAS_LIMIT),
            max_fee_per_gas: Some(gas_price),
            max_priority_fee_per_gas: Some(gas_price),
            ..Default::default()
        },
    )
    .await?;

    info!(
        "CommonBridge deployed:\n  Proxy -> address={:#x}, tx_hash={:#x}\n  Impl  -> address={:#x}, tx_hash={:#x}",
        bridge_deployment.proxy_address,
        bridge_deployment.proxy_tx_hash,
        bridge_deployment.implementation_address,
        bridge_deployment.implementation_tx_hash,
    );

    nonce += 2;

    let sequencer_registry_deployment = if opts.deploy_based_contracts {
        info!("Deploying SequencerRegistry");

        let sequencer_registry_deployment = deploy_with_proxy_from_bytecode_no_wait(
            deployer,
            eth_client,
            SEQUENCER_REGISTRY_BYTECODE,
            &salt,
            Overrides {
                nonce: Some(nonce),
                gas_limit: Some(TRANSACTION_GAS_LIMIT),
                max_fee_per_gas: Some(gas_price),
                max_priority_fee_per_gas: Some(gas_price),
                ..Default::default()
            },
        )
        .await?;

        nonce += 2;

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

    // if it's a required proof type, but no address has been specified, deploy it.
    let (sp1_verifier_deployment_tx_hash, sp1_verifier_address) = match opts.sp1_verifier_address {
        _ if opts.aligned => (H256::default(), Address::zero()),
        Some(addr) if opts.sp1 => (H256::default(), addr),
        None if opts.sp1 => {
            info!("Deploying SP1Verifier");
            let (verifier_deployment_tx_hash, sp1_verifier_address) =
                create2_deploy_from_bytecode_no_wait(
                    &[],
                    SP1_VERIFIER_BYTECODE,
                    deployer,
                    &salt,
                    eth_client,
                    Overrides {
                        nonce: Some(nonce),
                        gas_limit: Some(TRANSACTION_GAS_LIMIT),
                        max_fee_per_gas: Some(gas_price),
                        max_priority_fee_per_gas: Some(gas_price),
                        ..Default::default()
                    },
                )
                .await?;
            info!(address = %format!("{sp1_verifier_address:#x}"), tx_hash = %format!("{verifier_deployment_tx_hash:#x}"), "SP1Verifier deployed");
            (verifier_deployment_tx_hash, sp1_verifier_address)
        }
        _ => (H256::default(), Address::zero()),
    };

    // we can't deploy the risc0 contract because of uncompatible licenses
    let risc0_verifier_address = match opts.risc0_verifier_address {
        _ if opts.aligned => Address::zero(),
        Some(addr) if opts.risc0 => addr,
        None if opts.risc0 => {
            return Err(DeployerError::InternalError(
                "Risc0Verifier address is not set and risc0 is a required prover".to_string(),
            ));
        }
        _ => Address::zero(),
    };

    // if it's a required proof type, but no address has been specified, deploy it.
    let tdx_verifier_address = match opts.tdx_verifier_address {
        Some(addr) if opts.tdx => addr,
        None if opts.tdx => {
            info!("Deploying TDXVerifier (if tdx_deploy_verifier is true)");
            let tdx_verifier_address =
                deploy_tdx_contracts(opts, on_chain_proposer_deployment.proxy_address)?;

            info!(address = %format!("{tdx_verifier_address:#x}"), "TDXVerifier deployed");
            tdx_verifier_address
        }
        _ => Address::zero(),
    };

    // return error if no address was specified but verification with aligned is required.
    let aligned_aggregator_address = match opts.aligned_aggregator_address {
        Some(addr) if opts.aligned => addr,
        None if opts.aligned => return Err(DeployerError::InternalError(
            "Verification with Aligned Layer is required but no aggregator address was provided"
                .to_string(),
        )),
        _ => Address::zero(),
    };

    trace!(
        on_chain_proposer_proxy_address = ?on_chain_proposer_deployment.proxy_address,
        bridge_proxy_address = ?bridge_deployment.proxy_address,
        on_chain_proposer_implementation_address = ?on_chain_proposer_deployment.implementation_address,
        bridge_implementation_address = ?bridge_deployment.implementation_address,
        sp1_verifier_address = ?sp1_verifier_address,
        risc0_verifier_address = ?risc0_verifier_address,
        tdx_verifier_address = ?tdx_verifier_address,
        "Contracts deployed"
    );
    let receipts = vec![
        on_chain_proposer_deployment.implementation_tx_hash,
        on_chain_proposer_deployment.proxy_tx_hash,
        bridge_deployment.implementation_tx_hash,
        bridge_deployment.proxy_tx_hash,
        sequencer_registry_deployment.implementation_tx_hash,
        sequencer_registry_deployment.proxy_tx_hash,
        sp1_verifier_deployment_tx_hash,
        router_deployment.implementation_tx_hash,
        router_deployment.proxy_tx_hash,
    ];

    Ok((
        ContractAddresses {
            on_chain_proposer_address: on_chain_proposer_deployment.proxy_address,
            bridge_address: bridge_deployment.proxy_address,
            sp1_verifier_address,
            risc0_verifier_address,
            tdx_verifier_address,
            sequencer_registry_address: sequencer_registry_deployment.proxy_address,
            aligned_aggregator_address,
            router: opts.router.or(Some(router_deployment.proxy_address)),
        },
        receipts,
    ))
}

fn deploy_tdx_contracts(
    opts: &DeployerOptions,
    on_chain_proposer: Address,
) -> Result<Address, DeployerError> {
    Command::new("make")
        .arg("deploy-all")
        .env("PRIVATE_KEY", hex::encode(opts.private_key.as_ref()))
        .env("RPC_URL", opts.rpc_url.as_str())
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

fn get_vk(prover_type: ProverType, opts: &DeployerOptions) -> Result<Bytes, DeployerError> {
    let (required_type, vk_path) = match prover_type {
        ProverType::SP1 => (opts.sp1, &opts.sp1_vk_path),
        ProverType::RISC0 => (opts.risc0, &opts.risc0_vk_path),
        _ => unimplemented!("{prover_type}"),
    };

    info!("Reading vk in path {vk_path:?}");
    if !required_type {
        Ok(Bytes::new())
    } else if let Some(vk_path) = vk_path {
        read_vk(vk_path)
    } else {
        info!(?prover_type, "Using vk from local repo");
        let vk_path = {
            let path = match &prover_type {
                ProverType::RISC0 => format!(
                    "{}/../../crates/l2/prover/src/guest_program/src/risc0/out/riscv32im-risc0-vk",
                    env!("CARGO_MANIFEST_DIR")
                ),
                // Aligned requires the vk's 32 bytes hash, while the L1 verifier requires
                // the hash as a bn254 F_r element.
                ProverType::SP1 if opts.aligned => format!(
                    "{}/../../crates/l2/prover/src/guest_program/src/sp1/out/riscv32im-succinct-zkvm-vk-u32",
                    env!("CARGO_MANIFEST_DIR")
                ),
                ProverType::SP1 if !opts.aligned => format!(
                    "{}/../../crates/l2/prover/src/guest_program/src/sp1/out/riscv32im-succinct-zkvm-vk-bn254",
                    env!("CARGO_MANIFEST_DIR")
                ),
                // other types don't have a verification key
                _other => {
                    return Err(DeployerError::InternalError(format!(
                        "missing {prover_type} vk"
                    )));
                }
            };
            std::fs::canonicalize(path)?
        };
        read_vk(
            vk_path
                .to_str()
                .ok_or(DeployerError::FailedToGetStringFromPath)?,
        )
    }
}

fn read_vk(path: &str) -> Result<Bytes, DeployerError> {
    let string = std::fs::read_to_string(path)?;
    let trimmed = string.trim_start_matches("0x").trim();
    let decoded = hex::decode(trimmed)
        .map_err(|_| DeployerError::InternalError("failed to decode vk".to_string()))?;
    Ok(Bytes::from(decoded))
}

async fn initialize_contracts(
    contract_addresses: ContractAddresses,
    eth_client: &EthClient,
    opts: &DeployerOptions,
    genesis: &Genesis,
    initializer: &Signer,
) -> Result<Vec<H256>, DeployerError> {
    trace!("Initializing contracts");
    let mut tx_hashes = vec![];
    let gas_price = eth_client
        .get_gas_price_with_extra(20)
        .await?
        .try_into()
        .map_err(|_| {
            EthClientError::InternalError("Failed to convert gas_price to a u64".to_owned())
        })?;

    trace!(committer_l1_address = %opts.committer_l1_address, "Using committer L1 address for OnChainProposer initialization");

    info!("Reading verification keys for OnChainProposer initialization");

    let sp1_vk = get_vk(ProverType::SP1, opts)?;
    info!("SP1 VK read");
    let risc0_vk = get_vk(ProverType::RISC0, opts)?;

    info!("Risc0 vk read");
    let commit_hash = keccak(get_git_commit_hash());

    let deployer_address = get_address_from_secret_key(&opts.private_key.secret_bytes())
        .map_err(DeployerError::InternalError)?;

    info!("Initializing OnChainProposer");

    if opts.deploy_based_contracts {
        // Initialize OnChainProposer with Based config and SequencerRegistry
        let calldata_values = vec![
            Value::Bool(opts.validium),
            Value::Address(deployer_address),
            Value::Bool(opts.risc0),
            Value::Bool(opts.sp1),
            Value::Bool(opts.tdx),
            Value::Bool(opts.aligned),
            Value::Address(contract_addresses.risc0_verifier_address),
            Value::Address(contract_addresses.sp1_verifier_address),
            Value::Address(contract_addresses.tdx_verifier_address),
            Value::Address(contract_addresses.aligned_aggregator_address),
            Value::FixedBytes(sp1_vk),
            Value::FixedBytes(risc0_vk),
            Value::FixedBytes(commit_hash.0.to_vec().into()),
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
        let deployer_nonce = eth_client
            .get_nonce(deployer.address(), BlockIdentifier::Tag(BlockTag::Pending))
            .await?;

        let initialize_tx_hash = initialize_contract_no_wait(
            contract_addresses.on_chain_proposer_address,
            on_chain_proposer_initialization_calldata,
            &deployer,
            eth_client,
            Overrides {
                nonce: Some(deployer_nonce),
                gas_limit: Some(TRANSACTION_GAS_LIMIT),
                max_fee_per_gas: Some(gas_price),
                max_priority_fee_per_gas: Some(gas_price),
                ..Default::default()
            },
        )
        .await?;

        info!(tx_hash = %format!("{initialize_tx_hash:#x}"), "OnChainProposer initialized");

        tx_hashes.push(initialize_tx_hash);

        info!("Initializing SequencerRegistry");
        let initialize_tx_hash = {
            let deployer_nonce = eth_client
                .get_nonce(deployer.address(), BlockIdentifier::Tag(BlockTag::Pending))
                .await?;
            let calldata_values = vec![
                Value::Address(opts.sequencer_registry_owner.ok_or(
                    DeployerError::ConfigValueNotSet("--sequencer-registry-owner".to_string()),
                )?),
                Value::Address(contract_addresses.on_chain_proposer_address),
            ];
            let sequencer_registry_initialization_calldata =
                encode_calldata("initialize(address,address)", &calldata_values)?;

            initialize_contract_no_wait(
                contract_addresses.sequencer_registry_address,
                sequencer_registry_initialization_calldata,
                &deployer,
                eth_client,
                Overrides {
                    nonce: Some(deployer_nonce),
                    gas_limit: Some(TRANSACTION_GAS_LIMIT),
                    max_fee_per_gas: Some(gas_price),
                    max_priority_fee_per_gas: Some(gas_price),
                    ..Default::default()
                },
            )
            .await?
        };
        info!(tx_hash = %format!("{initialize_tx_hash:#x}"), "SequencerRegistry initialized");
        tx_hashes.push(initialize_tx_hash);
    } else {
        if let Some(router) = contract_addresses.router
            && opts.deploy_router
        {
            let initializer_nonce = eth_client
                .get_nonce(
                    initializer.address(),
                    BlockIdentifier::Tag(BlockTag::Pending),
                )
                .await?;
            let calldata_values = vec![Value::Address(deployer_address)];
            let router_initialization_calldata =
                encode_calldata(ROUTER_INITIALIZER_SIGNATURE, &calldata_values)?;
            let initialize_tx_hash = initialize_contract_no_wait(
                router,
                router_initialization_calldata,
                initializer,
                eth_client,
                Overrides {
                    nonce: Some(initializer_nonce),
                    gas_limit: Some(TRANSACTION_GAS_LIMIT),
                    max_fee_per_gas: Some(gas_price),
                    max_priority_fee_per_gas: Some(gas_price),
                    ..Default::default()
                },
            )
            .await?;
            info!(tx_hash = %format!("{initialize_tx_hash:#x}"), "Router initialized");
        }
        // Initialize only OnChainProposer without Based config
        let calldata_values = vec![
            Value::Bool(opts.validium),
            Value::Address(deployer_address),
            Value::Bool(opts.risc0),
            Value::Bool(opts.sp1),
            Value::Bool(opts.tdx),
            Value::Bool(opts.aligned),
            Value::Address(contract_addresses.risc0_verifier_address),
            Value::Address(contract_addresses.sp1_verifier_address),
            Value::Address(contract_addresses.tdx_verifier_address),
            Value::Address(contract_addresses.aligned_aggregator_address),
            Value::FixedBytes(sp1_vk),
            Value::FixedBytes(risc0_vk),
            Value::FixedBytes(commit_hash.0.to_vec().into()),
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
        let initializer_nonce = eth_client
            .get_nonce(
                initializer.address(),
                BlockIdentifier::Tag(BlockTag::Pending),
            )
            .await?;

        let initialize_tx_hash = initialize_contract_no_wait(
            contract_addresses.on_chain_proposer_address,
            on_chain_proposer_initialization_calldata,
            initializer,
            eth_client,
            Overrides {
                nonce: Some(initializer_nonce),
                gas_limit: Some(TRANSACTION_GAS_LIMIT),
                max_fee_per_gas: Some(gas_price),
                max_priority_fee_per_gas: Some(gas_price),
                ..Default::default()
            },
        )
        .await?;
        info!(tx_hash = %format!("{initialize_tx_hash:#x}"), "OnChainProposer initialized");
        tx_hashes.push(initialize_tx_hash);
    }

    let initialize_bridge_address_tx_hash = {
        let initializer_nonce = eth_client
            .get_nonce(
                initializer.address(),
                BlockIdentifier::Tag(BlockTag::Pending),
            )
            .await?;
        let calldata_values = vec![Value::Address(contract_addresses.bridge_address)];
        let on_chain_proposer_initialization_calldata =
            encode_calldata(INITIALIZE_BRIDGE_ADDRESS_SIGNATURE, &calldata_values)?;

        initialize_contract_no_wait(
            contract_addresses.on_chain_proposer_address,
            on_chain_proposer_initialization_calldata,
            initializer,
            eth_client,
            Overrides {
                nonce: Some(initializer_nonce),
                gas_limit: Some(TRANSACTION_GAS_LIMIT),
                max_fee_per_gas: Some(gas_price),
                max_priority_fee_per_gas: Some(gas_price),
                ..Default::default()
            },
        )
        .await?
    };

    info!(
        tx_hash = %format!("{initialize_bridge_address_tx_hash:#x}"),
        "OnChainProposer bridge address initialized"
    );

    tx_hashes.push(initialize_bridge_address_tx_hash);

    if opts.on_chain_proposer_owner != initializer.address() {
        let initializer_nonce = eth_client
            .get_nonce(
                initializer.address(),
                BlockIdentifier::Tag(BlockTag::Pending),
            )
            .await?;
        let transfer_ownership_tx_hash = {
            let owner_transfer_calldata = encode_calldata(
                TRANSFER_OWNERSHIP_SIGNATURE,
                &[Value::Address(opts.on_chain_proposer_owner)],
            )?;

            initialize_contract_no_wait(
                contract_addresses.on_chain_proposer_address,
                owner_transfer_calldata,
                initializer,
                eth_client,
                Overrides {
                    nonce: Some(initializer_nonce),
                    gas_limit: Some(TRANSACTION_GAS_LIMIT),
                    max_fee_per_gas: Some(gas_price),
                    max_priority_fee_per_gas: Some(gas_price),
                    ..Default::default()
                },
            )
            .await?
        };

        tx_hashes.push(transfer_ownership_tx_hash);

        if let Some(owner_pk) = opts.on_chain_proposer_owner_pk {
            let signer = Signer::Local(LocalSigner::new(owner_pk));
            let owner_nonce = eth_client
                .get_nonce(signer.address(), BlockIdentifier::Tag(BlockTag::Pending))
                .await?;
            let accept_ownership_calldata = encode_calldata(ACCEPT_OWNERSHIP_SIGNATURE, &[])?;
            let accept_tx = build_generic_tx(
                eth_client,
                TxType::EIP1559,
                contract_addresses.on_chain_proposer_address,
                opts.on_chain_proposer_owner,
                accept_ownership_calldata.into(),
                Overrides {
                    nonce: Some(owner_nonce),
                    gas_limit: Some(TRANSACTION_GAS_LIMIT),
                    max_fee_per_gas: Some(gas_price),
                    max_priority_fee_per_gas: Some(gas_price),
                    ..Default::default()
                },
            )
            .await?;
            let accept_tx_hash = send_generic_transaction(eth_client, accept_tx, &signer).await?;
            tx_hashes.push(accept_tx_hash);

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
        let initializer_nonce = eth_client
            .get_nonce(
                initializer.address(),
                BlockIdentifier::Tag(BlockTag::Pending),
            )
            .await?;
        let calldata_values = vec![
            Value::Address(initializer.address()),
            Value::Address(contract_addresses.on_chain_proposer_address),
            Value::Uint(opts.inclusion_max_wait.into()),
            Value::Address(contract_addresses.router.unwrap_or_default()),
            Value::Uint(genesis.config.chain_id.into()),
        ];
        let bridge_initialization_calldata =
            encode_calldata(BRIDGE_INITIALIZER_SIGNATURE, &calldata_values)?;

        initialize_contract_no_wait(
            contract_addresses.bridge_address,
            bridge_initialization_calldata,
            initializer,
            eth_client,
            Overrides {
                nonce: Some(initializer_nonce),
                gas_limit: Some(TRANSACTION_GAS_LIMIT),
                max_fee_per_gas: Some(gas_price),
                max_priority_fee_per_gas: Some(gas_price),
                ..Default::default()
            },
        )
        .await?
    };
    info!(tx_hash = %format!("{initialize_tx_hash:#x}"), "CommonBridge initialized");
    tx_hashes.push(initialize_tx_hash);

    if let Some(fee_token) = opts.initial_fee_token {
        let initializer_nonce = eth_client
            .get_nonce(
                initializer.address(),
                BlockIdentifier::Tag(BlockTag::Pending),
            )
            .await?;
        let register_tx_hash = register_fee_token_no_wait(
            eth_client,
            contract_addresses.bridge_address,
            fee_token,
            initializer,
            Overrides {
                nonce: Some(initializer_nonce),
                gas_limit: Some(TRANSACTION_GAS_LIMIT),
                max_fee_per_gas: Some(gas_price),
                max_priority_fee_per_gas: Some(gas_price),
                ..Default::default()
            },
        )
        .await?;
        info!(?fee_token, "CommonBridge initial fee token registered");
        info!(tx_hash = %format!("{register_tx_hash:#x}"), "Initial fee token registration transaction sent");
        tx_hashes.push(register_tx_hash);
    }

    if opts.bridge_owner != initializer.address() {
        let initializer_nonce = eth_client
            .get_nonce(
                initializer.address(),
                BlockIdentifier::Tag(BlockTag::Pending),
            )
            .await?;
        let transfer_calldata = encode_calldata(
            TRANSFER_OWNERSHIP_SIGNATURE,
            &[Value::Address(opts.bridge_owner)],
        )?;
        let transfer_tx_hash = initialize_contract_no_wait(
            contract_addresses.bridge_address,
            transfer_calldata,
            initializer,
            eth_client,
            Overrides {
                nonce: Some(initializer_nonce),
                gas_limit: Some(TRANSACTION_GAS_LIMIT),
                max_fee_per_gas: Some(gas_price),
                max_priority_fee_per_gas: Some(gas_price),
                ..Default::default()
            },
        )
        .await?;

        tx_hashes.push(transfer_tx_hash);

        if let Some(owner_pk) = opts.bridge_owner_pk {
            let signer = Signer::Local(LocalSigner::new(owner_pk));
            let owner_nonce = eth_client
                .get_nonce(signer.address(), BlockIdentifier::Tag(BlockTag::Pending))
                .await?;
            let accept_calldata = encode_calldata(ACCEPT_OWNERSHIP_SIGNATURE, &[])?;
            let accept_tx = build_generic_tx(
                eth_client,
                TxType::EIP1559,
                contract_addresses.bridge_address,
                opts.bridge_owner,
                accept_calldata.into(),
                Overrides {
                    gas_limit: Some(TRANSACTION_GAS_LIMIT),
                    nonce: Some(owner_nonce),
                    max_fee_per_gas: Some(gas_price),
                    max_priority_fee_per_gas: Some(gas_price),
                    ..Default::default()
                },
            )
            .await?;

            let accept_tx_hash = send_generic_transaction(eth_client, accept_tx, &signer).await?;

            tx_hashes.push(accept_tx_hash);
            info!(
                transfer_tx_hash = %format!("{transfer_tx_hash:#x}"),
                accept_tx_hash = %format!("{accept_tx_hash:#x}"),
                "CommonBridge ownership transferred and accepted"
            );
        } else {
            info!(
                transfer_tx_hash = %format!("{transfer_tx_hash:#x}"),
                "CommonBridge ownership transfer pending acceptance"
            );
        }
    }

    trace!("Contracts initialized");
    Ok(tx_hashes)
}

async fn register_chain(
    eth_client: &EthClient,
    contract_addresses: ContractAddresses,
    chain_id: u64,
    deployer: &Signer,
) -> Result<(), DeployerError> {
    let params = vec![
        Value::Uint(U256::from(chain_id)),
        Value::Address(contract_addresses.bridge_address),
    ];

    ethrex_l2_sdk::call_contract(
        eth_client,
        deployer,
        contract_addresses
            .router
            .ok_or(DeployerError::InternalError(
                "Router address is None. This is a bug.".to_string(),
            ))?,
        ROUTER_REGISTER_SIGNATURE,
        params,
    )
    .await?;

    info!(chain_id, "Chain registered");

    Ok(())
}

async fn make_deposits(
    bridge: Address,
    eth_client: &EthClient,
    opts: &DeployerOptions,
) -> Result<(), DeployerError> {
    trace!("Making deposits");

    let genesis: Genesis = if opts.use_compiled_genesis {
        serde_json::from_str(LOCAL_DEVNET_GENESIS_CONTENTS).map_err(|_| DeployerError::Genesis)?
    } else {
        read_genesis_file(
            opts.genesis_l1_path
                .clone()
                .ok_or(DeployerError::ConfigValueNotSet(
                    "--genesis-l1-path".to_string(),
                ))?
                .to_str()
                .ok_or(DeployerError::FailedToGetStringFromPath)?,
        )
    };

    let pks = if let Some(path) = &opts.private_keys_file_path {
        &read_to_string(path).map_err(|_| DeployerError::FailedToGetStringFromPath)?
    } else {
        LOCAL_DEVNET_PRIVATE_KEYS
    };

    let private_keys: Vec<String> = pks
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| line.trim().to_string())
        .collect();

    let mut last_hash = None;

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

        let build = build_generic_tx(
            eth_client,
            TxType::EIP1559,
            bridge,
            signer.address(),
            Bytes::new(),
            overrides,
        )
        .await?;

        match send_generic_transaction(eth_client, build, &signer).await {
            Ok(hash) => {
                last_hash = Some(hash);
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
    if let Some(hash) = last_hash {
        wait_for_transaction_receipt(hash, eth_client, 100).await?;
    }
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
        "ETHREX_DEPLOYER_SP1_VERIFIER_ADDRESS={:#x}",
        contract_addresses.sp1_verifier_address
    )?;

    writeln!(
        writer,
        "ETHREX_DEPLOYER_RISC0_VERIFIER_ADDRESS={:#x}",
        contract_addresses.risc0_verifier_address
    )?;
    writeln!(
        writer,
        "ETHREX_DEPLOYER_ALIGNED_AGGREGATOR_ADDRESS={:#x}",
        contract_addresses.aligned_aggregator_address
    )?;
    writeln!(
        writer,
        "ETHREX_DEPLOYER_TDX_VERIFIER_ADDRESS={:#x}",
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
    writeln!(
        writer,
        "ETHREX_SHARED_BRIDGE_ROUTER_ADDRESS={:#x}",
        contract_addresses.router.unwrap_or_default()
    )?;
    trace!(?env_file_path, "Contract addresses written to .env");
    Ok(())
}
