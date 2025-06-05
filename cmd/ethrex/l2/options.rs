use crate::{cli::Options as NodeOptions, utils};
use clap::Parser;
use ethrex_common::Address;
use ethrex_l2::{
    sequencer::configs::AlignedConfig, BlockProducerConfig, CommitterConfig, EthConfig,
    L1WatcherConfig, ProofCoordinatorConfig, SequencerConfig,
};
use ethrex_rpc::clients::eth::get_address_from_secret_key;
use secp256k1::SecretKey;
use std::net::{IpAddr, Ipv4Addr};

#[derive(Parser, Default)]
pub struct Options {
    #[command(flatten)]
    pub node_opts: NodeOptions,
    #[command(flatten)]
    pub sequencer_opts: SequencerOptions,
    #[arg(
        long = "sponsorable-addresses",
        value_name = "SPONSORABLE_ADDRESSES_PATH",
        help = "Path to a file containing addresses of contracts to which ethrex_SendTransaction should sponsor txs",
        help_heading = "L2 options"
    )]
    pub sponsorable_addresses_file_path: Option<String>,
    #[arg(long, value_parser = utils::parse_private_key, env = "SPONSOR_PRIVATE_KEY", help = "The private key of ethrex L2 transactions sponsor.", help_heading = "L2 options")]
    pub sponsor_private_key: Option<SecretKey>,
}

#[derive(Parser, Default)]
pub struct SequencerOptions {
    #[command(flatten)]
    pub eth_opts: EthOptions,
    #[command(flatten)]
    pub watcher_opts: WatcherOptions,
    #[command(flatten)]
    pub proposer_opts: ProposerOptions,
    #[command(flatten)]
    pub committer_opts: CommitterOptions,
    #[command(flatten)]
    pub proof_coordinator_opts: ProofCoordinatorOptions,
    #[command(flatten)]
    pub aligned_opts: AlignedOptions,
}

impl From<SequencerOptions> for SequencerConfig {
    fn from(opts: SequencerOptions) -> Self {
        Self {
            block_producer: BlockProducerConfig {
                block_time_ms: opts.proposer_opts.block_time_ms,
                coinbase_address: opts.proposer_opts.coinbase_address,
                elasticity_multiplier: opts.proposer_opts.elasticity_multiplier,
            },
            l1_committer: CommitterConfig {
                on_chain_proposer_address: opts.committer_opts.on_chain_proposer_address,
                l1_address: get_address_from_secret_key(
                    &opts.committer_opts.committer_l1_private_key,
                )
                .unwrap(),
                l1_private_key: opts.committer_opts.committer_l1_private_key,
                commit_time_ms: opts.committer_opts.commit_time_ms,
                arbitrary_base_blob_gas_price: opts.committer_opts.arbitrary_base_blob_gas_price,
                validium: opts.committer_opts.validium,
            },
            eth: EthConfig {
                rpc_url: opts.eth_opts.rpc_url,
                max_number_of_retries: opts.eth_opts.max_number_of_retries,
                backoff_factor: opts.eth_opts.backoff_factor,
                min_retry_delay: opts.eth_opts.min_retry_delay,
                max_retry_delay: opts.eth_opts.max_retry_delay,
                maximum_allowed_max_fee_per_gas: opts.eth_opts.maximum_allowed_max_fee_per_gas,
                maximum_allowed_max_fee_per_blob_gas: opts
                    .eth_opts
                    .maximum_allowed_max_fee_per_blob_gas,
            },
            l1_watcher: L1WatcherConfig {
                bridge_address: opts.watcher_opts.bridge_address,
                check_interval_ms: opts.watcher_opts.watch_interval_ms,
                max_block_step: opts.watcher_opts.max_block_step.into(),
                l2_proposer_private_key: opts.watcher_opts.l2_proposer_private_key,
                watcher_block_delay: opts.watcher_opts.watcher_block_delay,
            },
            proof_coordinator: ProofCoordinatorConfig {
                l1_address: get_address_from_secret_key(
                    &opts.proof_coordinator_opts.proof_coordinator_l1_private_key,
                )
                .unwrap(),
                l1_private_key: opts.proof_coordinator_opts.proof_coordinator_l1_private_key,
                listen_ip: opts.proof_coordinator_opts.listen_ip,
                listen_port: opts.proof_coordinator_opts.listen_port,
                proof_send_interval_ms: opts.proof_coordinator_opts.proof_send_interval_ms,
                dev_mode: opts.proof_coordinator_opts.dev_mode,
            },
            aligned: AlignedConfig {
                aligned_verifier_interval_ms: opts.aligned_opts.aligned_verifier_interval_ms,
                beacon_url: opts.aligned_opts.beacon_url,
                network: opts.aligned_opts.aligned_network,
                fee_estimate: opts.aligned_opts.fee_estimate,
                aligned_sp1_vk_path: opts.aligned_opts.aligned_sp1_vk_path,
                aligned_sp1_elf_path: opts.aligned_opts.aligned_sp1_elf_path,
            },
        }
    }
}

#[derive(Parser, Default)]
pub struct EthOptions {
    #[arg(
        long = "eth-rpc-url",
        default_value = "http://localhost:8545",
        value_name = "RPC_URL",
        env = "ETHREX_ETH_RPC_URL",
        help = "List of rpc urls to use.",
        help_heading = "Eth options",
        num_args = 1..10
    )]
    pub rpc_url: Vec<String>,
    #[arg(
        long = "eth-maximum-allowed-max-fee-per-gas",
        default_value = "10000000000",
        value_name = "UINT64",
        env = "ETHREX_MAXIMUM_ALLOWED_MAX_FEE_PER_GAS",
        help_heading = "Eth options"
    )]
    pub maximum_allowed_max_fee_per_gas: u64,
    #[arg(
        long = "eth-maximum-allowed-max-fee-per-blob-gas",
        default_value = "10000000000",
        value_name = "UINT64",
        env = "ETHREX_MAXIMUM_ALLOWED_MAX_FEE_PER_BLOB_GAS",
        help_heading = "Eth options"
    )]
    pub maximum_allowed_max_fee_per_blob_gas: u64,
    #[arg(
        long = "eth-max-number-of-retries",
        default_value = "10",
        value_name = "UINT64",
        env = "ETHREX_MAX_NUMBER_OF_RETRIES",
        help_heading = "Eth options"
    )]
    pub max_number_of_retries: u64,
    #[arg(
        long = "eth-backoff-factor",
        default_value = "2",
        value_name = "UINT64",
        env = "ETHREX_BACKOFF_FACTOR",
        help_heading = "Eth options"
    )]
    pub backoff_factor: u64,
    #[arg(
        long = "eth-min-retry-delay",
        default_value = "96",
        value_name = "UINT64",
        env = "ETHREX_MIN_RETRY_DELAY",
        help_heading = "Eth options"
    )]
    pub min_retry_delay: u64,
    #[arg(
        long = "eth-max-retry-delay",
        default_value = "1800",
        value_name = "UINT64",
        env = "ETHREX_MAX_RETRY_DELAY",
        help_heading = "Eth options"
    )]
    pub max_retry_delay: u64,
}

#[derive(Parser)]
pub struct WatcherOptions {
    #[arg(
        long,
        value_name = "ADDRESS",
        env = "ETHREX_WATCHER_BRIDGE_ADDRESS",
        help_heading = "L1 Watcher options"
    )]
    pub bridge_address: Address,
    #[arg(
        long,
        default_value = "1000",
        value_name = "UINT64",
        env = "ETHREX_WATCHER_WATCH_INTERVAL_MS",
        help_heading = "L1 Watcher options"
    )]
    pub watch_interval_ms: u64,
    #[arg(
        long,
        default_value = "5000",
        value_name = "UINT64",
        env = "ETHREX_WATCHER_MAX_BLOCK_STEP",
        help_heading = "L1 Watcher options"
    )]
    pub max_block_step: u64,
    #[arg(
        long,
        default_value = "0x385c546456b6a603a1cfcaa9ec9494ba4832da08dd6bcf4de9a71e4a01b74924",
        value_name = "PRIVATE_KEY",
        value_parser = utils::parse_private_key,
        env = "ETHREX_WATCHER_L2_PROPOSER_PRIVATE_KEY",
        help_heading = "L1 Watcher options",
    )]
    pub l2_proposer_private_key: SecretKey,
    #[arg(
        long = "watcher.block-delay",
        default_value_t = 10, // Reasonably safe value to account for reorgs
        value_name = "UINT64",
        env = "ETHREX_WATCHER_BLOCK_DELAY",
        help = "Number of blocks the L1 watcher waits before trusting an L1 block.",
        help_heading = "L1 Watcher options"
    )]
    pub watcher_block_delay: u64,
}

impl Default for WatcherOptions {
    fn default() -> Self {
        Self {
            bridge_address: "0x266ffef34e21a7c4ce2e0e42dc780c2c273ca440"
                .parse()
                .unwrap(),
            watch_interval_ms: 1000,
            max_block_step: 5000,
            l2_proposer_private_key: utils::parse_private_key(
                "0x385c546456b6a603a1cfcaa9ec9494ba4832da08dd6bcf4de9a71e4a01b74924",
            )
            .unwrap(),
            watcher_block_delay: 128,
        }
    }
}

#[derive(Parser, Default)]
pub struct ProposerOptions {
    #[arg(
        long = "proposer-block-time-ms",
        default_value = "5000",
        value_name = "UINT64",
        env = "ETHREX_PROPOSER_BLOCK_TIME_MS",
        help_heading = "L1 Watcher options"
    )]
    pub block_time_ms: u64,
    #[arg(
        long,
        default_value = "0x0007a881CD95B1484fca47615B64803dad620C8d",
        value_name = "ADDRESS",
        env = "ETHREX_PROPOSER_COINBASE_ADDRESS",
        help_heading = "Proposer options"
    )]
    pub coinbase_address: Address,
    #[arg(
        long,
        default_value = "2",
        value_name = "UINT64",
        env = "ETHREX_PROPOSER_ELASTICITY_MULTIPLIER",
        help_heading = "Proposer options"
    )]
    pub elasticity_multiplier: u64,
}

#[derive(Parser)]
pub struct CommitterOptions {
    #[arg(
        long = "committer-l1-private-key",
        default_value = "0x385c546456b6a603a1cfcaa9ec9494ba4832da08dd6bcf4de9a71e4a01b74924",
        value_name = "PRIVATE_KEY",
        value_parser = utils::parse_private_key,
        env = "ETHREX_COMMITTER_L1_PRIVATE_KEY",
        help_heading = "L1 Committer options",
        help = "Private key of a funded account that the sequencer will use to send commit txs to the L1.",
    )]
    pub committer_l1_private_key: SecretKey,
    #[arg(
        long,
        value_name = "ADDRESS",
        env = "ETHREX_COMMITTER_ON_CHAIN_PROPOSER_ADDRESS",
        help_heading = "L1 Committer options"
    )]
    pub on_chain_proposer_address: Address,
    #[arg(
        long,
        default_value = "60000",
        value_name = "UINT64",
        env = "ETHREX_COMMITTER_COMMIT_TIME_MS",
        help_heading = "L1 Committer options",
        help = "How often does the sequencer commit new blocks to the L1."
    )]
    pub commit_time_ms: u64,
    #[arg(
        long,
        default_value = "1000000000", // 1 Gwei
        value_name = "UINT64",
        env = "ETHREX_COMMITTER_ARBITRARY_BASE_BLOB_GAS_PRICE",
        help_heading = "L1 Committer options"
    )]
    pub arbitrary_base_blob_gas_price: u64,
    #[arg(
        long,
        default_value = "false",
        value_name = "BOOLEAN",
        env = "ETHREX_COMMITTER_VALIDIUM",
        help_heading = "L1 Committer options",
        help = "If set to true, initializes the committer in validium mode."
    )]
    pub validium: bool,
}

impl Default for CommitterOptions {
    fn default() -> Self {
        Self {
            committer_l1_private_key: utils::parse_private_key(
                "0x385c546456b6a603a1cfcaa9ec9494ba4832da08dd6bcf4de9a71e4a01b74924",
            )
            .unwrap(),
            on_chain_proposer_address: "0xea6d04861106c1fb69176d49eeb8de6dd14a9cfe"
                .parse()
                .unwrap(),
            commit_time_ms: 1000,
            arbitrary_base_blob_gas_price: 1_000_000_000,
            validium: false,
        }
    }
}

#[derive(Parser)]
pub struct ProofCoordinatorOptions {
    #[arg(
        long = "proof-coordinator-l1-private-key",
        default_value = "0x39725efee3fb28614de3bacaffe4cc4bd8c436257e2c8bb887c4b5c4be45e76d",
        value_name = "PRIVATE_KEY",
        value_parser = utils::parse_private_key,
        env = "ETHREX_PROOF_COORDINATOR_L1_PRIVATE_KEY",
        help_heading = "L1 Prover Server options",
        long_help = "Private key of of a funded account that the sequencer will use to send verify txs to the L1. Has to be a different account than --committer-l1-private-key.",
    )]
    pub proof_coordinator_l1_private_key: SecretKey,
    #[arg(
        long = "proof-coordinator-listen-ip",
        default_value = "127.0.0.1",
        value_name = "IP_ADDRESS",
        env = "ETHREX_PROOF_COORDINATOR_LISTEN_IP",
        help_heading = "L1 Prover Server options",
        help = "Set it to 0.0.0.0 to allow connections from other machines."
    )]
    pub listen_ip: IpAddr,
    #[arg(
        long = "proof-coordinator-listen-port",
        default_value = "3900",
        value_name = "UINT16",
        env = "ETHREX_PROOF_COORDINATOR_LISTEN_PORT",
        help_heading = "L1 Prover Server options"
    )]
    pub listen_port: u16,
    #[arg(
        long,
        default_value = "5000",
        value_name = "UINT64",
        env = "ETHREX_PROOF_COORDINATOR_SEND_INTERVAL_MS",
        help_heading = "L1 Prover Server options"
    )]
    pub proof_send_interval_ms: u64,
    #[arg(
        long = "proof-coordinator-dev-mode",
        default_value = "true",
        value_name = "BOOLEAN",
        env = "ETHREX_PROOF_COORDINATOR_DEV_MODE",
        help_heading = "L1 Prover Server options"
    )]
    pub dev_mode: bool,
}

impl Default for ProofCoordinatorOptions {
    fn default() -> Self {
        Self {
            proof_coordinator_l1_private_key:
                "0x39725efee3fb28614de3bacaffe4cc4bd8c436257e2c8bb887c4b5c4be45e76d"
                    .parse()
                    .unwrap(),
            listen_ip: IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)),
            listen_port: 3900,
            proof_send_interval_ms: 5000,
            dev_mode: true,
        }
    }
}

#[derive(Parser)]
pub struct AlignedOptions {
    #[arg(
        default_value = "5000",
        value_name = "UINT64",
        env = "ETHREX_ALIGNED_VERIFIER_INTERVAL_MS",
        help_heading = "Aligned options"
    )]
    pub aligned_verifier_interval_ms: u64,
    #[arg(
        long = "aligned.beacon-url",
        default_value = "http://localhost:58801",
        value_name = "BEACON_URL",
        env = "ETHREX_ALIGNED_BEACON_URL",
        help = "Beacon url to use.",
        help_heading = "Aligned options"
    )]
    pub beacon_url: String,
    #[arg(
        default_value = "devnet",
        value_name = "NETWORK",
        env = "ETHREX_ALIGNED_NETWORK",
        help = "L1 network name for Aligned sdk",
        help_heading = "Aligned options"
    )]
    pub aligned_network: String,
    #[arg(
        long = "aligned.fee-estimate",
        default_value = "instant",
        value_name = "FEE_ESTIMATE",
        env = "ETHREX_ALIGNED_FEE_ESTIMATE",
        help = "Fee estimate for Aligned sdk",
        help_heading = "Aligned options"
    )]
    pub fee_estimate: String,
    #[arg(
        default_value_t = format!("{}/../../crates/l2/prover/zkvm/interface/sp1/out/riscv32im-succinct-zkvm-vk", env!("CARGO_MANIFEST_DIR")),
        value_name = "PATH",
        env = "ETHREX_ALIGNED_SP1_VERIFICATION_KEY_PATH",
        help_heading = "Aligned options",
        help = "Path to the SP1 verification key. This is used for proof verification."
    )]
    pub aligned_sp1_vk_path: String,
    #[arg(
        default_value_t = format!("{}/../../crates/l2/prover/zkvm/interface/sp1/out/riscv32im-succinct-zkvm-elf", env!("CARGO_MANIFEST_DIR")),
        value_name = "PATH",
        env = "ETHREX_ALIGNED_SP1_ELF_PATH",
        help_heading = "Aligned options",
        help = "Path to the SP1 elf. This is used for proof verification."
    )]
    pub aligned_sp1_elf_path: String,
}

impl Default for AlignedOptions {
    fn default() -> Self {
        Self {
            aligned_verifier_interval_ms: 5000,
            beacon_url: "http://127.0.0.1:58801".to_string(),
            aligned_network: "devnet".to_string(),
            fee_estimate: "instant".to_string(),
            aligned_sp1_vk_path: format!(
                "{}/../../prover/zkvm/interface/sp1/out/riscv32im-succinct-zkvm-vk",
                env!("CARGO_MANIFEST_DIR")
            ),
            aligned_sp1_elf_path: format!(
                "{}/../../prover/zkvm/interface/sp1/out/riscv32im-succinct-zkvm-elf",
                env!("CARGO_MANIFEST_DIR")
            ),
        }
    }
}
