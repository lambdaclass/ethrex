use std::path::PathBuf;

use bytes::Bytes;
use clap::{ArgAction, Parser};
use ethrex_common::{Address, H160, H256};
use hex::FromHexError;
use secp256k1::SecretKey;

#[derive(Parser)]
pub struct DeployerOptions {
    #[arg(
        long = "eth-rpc-url",
        value_name = "RPC_URL",
        env = "ETHREX_ETH_RPC_URL",
        help_heading = "Eth options"
    )]
    pub rpc_url: String,
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
        help = "Path to the file containing the private keys of the rich accounts. The default is ../../test_data/private_keys_l1.txt"
    )]
    pub private_keys_file_path: Option<PathBuf>,
    #[arg(
        long,
        value_name = "PATH",
        env = "ETHREX_DEPLOYER_GENESIS_L1_PATH",
        required_if_eq("deposit_rich", "true"),
        help_heading = "Deployer options",
        help = "Path to the genesis file. The default is ../../test_data/genesis-l1-dev.json"
    )]
    pub genesis_l1_path: Option<PathBuf>,
    #[arg(
        long,
        value_name = "PATH",
        env = "ETHREX_DEPLOYER_GENESIS_L2_PATH",
        help_heading = "Deployer options",
        help = "Path to the l2 genesis file. The default is ../../test_data/genesis-l2.json"
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
        value_name = "PATH",
        env = "ETHREX_DEPLOYER_CONTRACTS_PATH",
        help_heading = "Deployer options",
        help = "Path to the contracts directory. The default is the current directory."
    )]
    pub contracts_path: PathBuf,
    #[arg(
        long = "pico.verifier-address",
        value_name = "ADDRESS",
        env = "ETHREX_DEPLOYER_PICO_CONTRACT_VERIFIER",
        required_if_eq("pico_deploy_verifier", "false"),
        help_heading = "Deployer options",
        help = "If set to 0xAA skip proof verification -> Only use in dev mode."
    )]
    pub pico_verifier_address: Option<Address>,
    #[arg(
        long = "pico.deploy-verifier",
        default_value = "false",
        value_name = "BOOLEAN",
        env = "ETHREX_DEPLOYER_PICO_DEPLOY_VERIFIER",
        action = ArgAction::SetTrue,
        required_unless_present = "pico_verifier_address",
        help_heading = "Deployer options",
        help = "If set to true, it will deploy the contract and override the address above with the deployed one."
    )]
    pub pico_deploy_verifier: bool,
    // TODO: This should work side by side with a risc0_deploy_verifier flag.
    #[arg(
        long = "risc0.verifier-address",
        value_name = "ADDRESS",
        env = "ETHREX_DEPLOYER_RISC0_CONTRACT_VERIFIER",
        required = true, // TODO: This should be required_unless_present = "risc0_deploy_verifier",
        help_heading = "Deployer options",
        help = "If set to 0xAA skip proof verification -> Only use in dev mode."
    )]
    pub risc0_verifier_address: Option<Address>,
    #[arg(
        long = "sp1.verifier-address",
        value_name = "ADDRESS",
        env = "ETHREX_DEPLOYER_SP1_CONTRACT_VERIFIER",
        required_if_eq("sp1_deploy_verifier", "false"),
        help_heading = "Deployer options",
        help = "If set to 0xAA skip proof verification -> Only use in dev mode."
    )]
    pub sp1_verifier_address: Option<Address>,
    #[arg(
        long = "sp1.deploy-verifier",
        default_value = "false",
        value_name = "BOOLEAN",
        action = ArgAction::SetTrue,
        env = "ETHREX_DEPLOYER_SP1_DEPLOY_VERIFIER",
        required_unless_present = "sp1_verifier_address",
        help_heading = "Deployer options",
        help = "If set to true, it will deploy the contract and override the address above with the deployed one.",
    )]
    pub sp1_deploy_verifier: bool,
    #[arg(
        long = "tdx.verifier-address",
        value_name = "ADDRESS",
        env = "ETHREX_DEPLOYER_TDX_CONTRACT_VERIFIER",
        required_if_eq("tdx_deploy_verifier", "false"),
        help_heading = "Deployer options",
        help = "If set to 0xAA skip proof verification -> Only use in dev mode."
    )]
    pub tdx_verifier_address: Option<Address>,
    #[arg(
        long = "tdx.deploy-verifier",
        default_value = "false",
        value_name = "BOOLEAN",
        action = ArgAction::SetTrue,
        env = "ETHREX_DEPLOYER_TDX_DEPLOY_VERIFIER",
        required_unless_present = "tdx_verifier_address",
        help_heading = "Deployer options",
        help = "If set to true, it will deploy the contract and override the address above with the deployed one.",
    )]
    pub tdx_deploy_verifier: bool,
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
        help_heading = "Deployer options",
        help = "If true, L2 will run on validium mode as opposed to the default rollup mode, meaning it will not publish state diffs to the L1."
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
}

impl Default for DeployerOptions {
    fn default() -> Self {
        Self {
            rpc_url: "http://localhost:8545".to_string(),
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
            env_file_path: None,
            deposit_rich: false,
            private_keys_file_path: None,
            genesis_l1_path: None,
            genesis_l2_path: "../../test_data/genesis-l2.json".into(),
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
            contracts_path: PathBuf::from("."),
            pico_verifier_address: Some(H160([
                0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
                0x00, 0x00, 0x00, 0x00, 0x00, 0xaa,
            ])),
            pico_deploy_verifier: false,
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
            randomize_contract_deployment: false,
            validium: false,
            // 0x03d0a0aee676cc45bf7032649e0871927c947c8e
            on_chain_proposer_owner: H160([
                0x03, 0xd0, 0xa0, 0xae, 0xe6, 0x76, 0xcc, 0x45, 0xbf, 0x70, 0x32, 0x64, 0x9e, 0x08,
                0x71, 0x92, 0x7c, 0x94, 0x7c, 0x8e,
            ]),
            // 0x03d0a0aee676cc45bf7032649e0871927c947c8e
            bridge_owner: H160([
                0x03, 0xd0, 0xa0, 0xae, 0xe6, 0x76, 0xcc, 0x45, 0xbf, 0x70, 0x32, 0x64, 0x9e, 0x08,
                0x71, 0x92, 0x7c, 0x94, 0x7c, 0x8e,
            ]),
            on_chain_proposer_owner_pk: None,
            sp1_vk_path: format!(
                "{}/../prover/zkvm/interface/sp1/out/riscv32im-succinct-zkvm-vk",
                env!("CARGO_MANIFEST_DIR")
            ),
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
