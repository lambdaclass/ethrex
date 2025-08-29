use crate::{
    cli::Options as NodeOptions,
    utils::{self},
};
use clap::Parser;
use ethrex_common::Address;
use ethrex_l2::{
    BasedConfig, BlockFetcherConfig, BlockProducerConfig, CommitterConfig, EthConfig,
    L1WatcherConfig, ProofCoordinatorConfig, SequencerConfig, StateUpdaterConfig,
    sequencer::{
        configs::{AlignedConfig, MonitorConfig},
        utils::resolve_aligned_network,
    },
};
use ethrex_l2_rpc::signer::{LocalSigner, RemoteSigner, Signer};
use ethrex_prover_lib::{backends::Backend, config::ProverConfig};
use ethrex_rpc::clients::eth::{
    BACKOFF_FACTOR, MAX_NUMBER_OF_RETRIES, MAX_RETRY_DELAY, MIN_RETRY_DELAY,
};
use reqwest::Url;
use secp256k1::{PublicKey, SecretKey};
use std::{
    net::{IpAddr, Ipv4Addr},
    str::FromStr,
};
use tracing::Level;

#[derive(Parser, Debug)]
#[group(id = "L2Options")]
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
    //TODO: make optional when the the sponsored feature is complete
    #[arg(long, default_value = "0xffd790338a2798b648806fc8635ac7bf14af15425fed0c8f25bcc5febaa9b192", value_parser = utils::parse_private_key, env = "SPONSOR_PRIVATE_KEY", help = "The private key of ethrex L2 transactions sponsor.", help_heading = "L2 options")]
    pub sponsor_private_key: SecretKey,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            node_opts: NodeOptions::default(),
            sequencer_opts: SequencerOptions::default(),
            sponsorable_addresses_file_path: None,
            sponsor_private_key: utils::parse_private_key(
                "0xffd790338a2798b648806fc8635ac7bf14af15425fed0c8f25bcc5febaa9b192",
            )
            .unwrap(),
        }
    }
}

#[derive(Parser, Default, Debug)]
pub struct SequencerOptions {
    #[command(flatten)]
    pub eth_opts: EthOptions,
    #[command(flatten)]
    pub watcher_opts: WatcherOptions,
    #[command(flatten)]
    pub block_producer_opts: BlockProducerOptions,
    #[command(flatten)]
    pub committer_opts: CommitterOptions,
    #[command(flatten)]
    pub proof_coordinator_opts: ProofCoordinatorOptions,
    #[command(flatten)]
    pub based_opts: BasedOptions,
    #[command(flatten)]
    pub aligned_opts: AlignedOptions,
    #[command(flatten)]
    pub monitor_opts: MonitorOptions,
    #[arg(
        long = "validium",
        default_value = "false",
        value_name = "BOOLEAN",
        env = "ETHREX_L2_VALIDIUM",
        help_heading = "L2 options",
        long_help = "If true, L2 will run on validium mode as opposed to the default rollup mode, meaning it will not publish state diffs to the L1."
    )]
    pub validium: bool,
    #[clap(
        long,
        default_value = "false",
        value_name = "BOOLEAN",
        env = "ETHREX_BASED",
        help_heading = "Based options"
    )]
    pub based: bool,
    #[clap(
        long,
        default_value = "false",
        value_name = "BOOLEAN",
        env = "ETHREX_NO_MONITOR",
        help_heading = "Monitor options"
    )]
    pub no_monitor: bool,
}

pub fn parse_signer(
    private_key: Option<SecretKey>,
    url: Option<Url>,
    public_key: Option<PublicKey>,
) -> Result<Signer, SequencerOptionsError> {
    Ok(match url {
        Some(url) => RemoteSigner::new(
            url,
            public_key.ok_or(SequencerOptionsError::RemoteUrlWithoutPubkey)?,
        )
        .into(),
        None => LocalSigner::new(private_key.ok_or(SequencerOptionsError::NoSigner(
            "ProofCoordinator".to_string(),
        ))?)
        .into(),
    })
}

#[derive(Debug, thiserror::Error)]
pub enum SequencerOptionsError {
    #[error("Remote signer URL was provided without a public key")]
    RemoteUrlWithoutPubkey,
    #[error("No signer was set up for {0}")]
    NoSigner(String),
    #[error("No coinbase address was provided")]
    NoCoinbaseAddress,
    #[error("No on-chain proposer address was provided")]
    NoOnChainProposerAddress,
    #[error("No bridge address was provided")]
    NoBridgeAddress,
}

impl TryFrom<SequencerOptions> for SequencerConfig {
    type Error = SequencerOptionsError;

    fn try_from(opts: SequencerOptions) -> Result<Self, Self::Error> {
        let committer_signer = parse_signer(
            opts.committer_opts.committer_l1_private_key,
            opts.committer_opts.committer_remote_signer_url,
            opts.committer_opts.committer_remote_signer_public_key,
        )?;

        let proof_coordinator_signer = parse_signer(
            opts.proof_coordinator_opts.proof_coordinator_l1_private_key,
            opts.proof_coordinator_opts.remote_signer_url,
            opts.proof_coordinator_opts.remote_signer_public_key,
        )?;

        Ok(Self {
            block_producer: BlockProducerConfig {
                block_time_ms: opts.block_producer_opts.block_time_ms,
                coinbase_address: opts
                    .block_producer_opts
                    .coinbase_address
                    .ok_or(SequencerOptionsError::NoCoinbaseAddress)?,
                elasticity_multiplier: opts.block_producer_opts.elasticity_multiplier,
            },
            l1_committer: CommitterConfig {
                on_chain_proposer_address: opts
                    .committer_opts
                    .on_chain_proposer_address
                    .ok_or(SequencerOptionsError::NoOnChainProposerAddress)?,
                first_wake_up_time_ms: opts.committer_opts.first_wake_up_time_ms.unwrap_or(0),
                commit_time_ms: opts.committer_opts.commit_time_ms,
                arbitrary_base_blob_gas_price: opts.committer_opts.arbitrary_base_blob_gas_price,
                signer: committer_signer,
                validium: opts.validium,
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
                bridge_address: opts
                    .watcher_opts
                    .bridge_address
                    .ok_or(SequencerOptionsError::NoBridgeAddress)?,
                check_interval_ms: opts.watcher_opts.watch_interval_ms,
                max_block_step: opts.watcher_opts.max_block_step.into(),
                watcher_block_delay: opts.watcher_opts.watcher_block_delay,
            },
            proof_coordinator: ProofCoordinatorConfig {
                listen_ip: opts.proof_coordinator_opts.listen_ip,
                listen_port: opts.proof_coordinator_opts.listen_port,
                proof_send_interval_ms: opts.proof_coordinator_opts.proof_send_interval_ms,
                signer: proof_coordinator_signer,
                tdx_private_key: opts
                    .proof_coordinator_opts
                    .proof_coordinator_tdx_private_key,
                validium: opts.validium,
            },
            based: BasedConfig {
                enabled: opts.based,
                state_updater: StateUpdaterConfig {
                    sequencer_registry: opts
                        .based_opts
                        .state_updater_opts
                        .sequencer_registry
                        .unwrap_or_default(),
                    check_interval_ms: opts.based_opts.state_updater_opts.check_interval_ms,
                },
                block_fetcher: BlockFetcherConfig {
                    fetch_interval_ms: opts.based_opts.block_fetcher.fetch_interval_ms,
                    fetch_block_step: opts.based_opts.block_fetcher.fetch_block_step,
                },
            },
            aligned: AlignedConfig {
                aligned_mode: opts.aligned_opts.aligned,
                aligned_verifier_interval_ms: opts.aligned_opts.aligned_verifier_interval_ms,
                beacon_urls: opts.aligned_opts.beacon_url.unwrap_or_default(),
                network: resolve_aligned_network(
                    &opts.aligned_opts.aligned_network.unwrap_or_default(),
                ),
                fee_estimate: opts.aligned_opts.fee_estimate,
            },
            monitor: MonitorConfig {
                enabled: !opts.no_monitor,
                tick_rate: opts.monitor_opts.tick_rate,
                batch_widget_height: opts.monitor_opts.batch_widget_height,
            },
        })
    }
}

#[derive(Parser, Debug)]
pub struct EthOptions {
    #[arg(
        long = "eth.rpc-url",
        value_name = "RPC_URL",
        env = "ETHREX_ETH_RPC_URL",
        help = "List of rpc urls to use.",
        help_heading = "Eth options",
        num_args = 1..
    )]
    pub rpc_url: Vec<String>,
    #[arg(
        long = "eth.maximum-allowed-max-fee-per-gas",
        default_value = "10000000000",
        value_name = "UINT64",
        env = "ETHREX_MAXIMUM_ALLOWED_MAX_FEE_PER_GAS",
        help_heading = "Eth options"
    )]
    pub maximum_allowed_max_fee_per_gas: u64,
    #[arg(
        long = "eth.maximum-allowed-max-fee-per-blob-gas",
        default_value = "10000000000",
        value_name = "UINT64",
        env = "ETHREX_MAXIMUM_ALLOWED_MAX_FEE_PER_BLOB_GAS",
        help_heading = "Eth options"
    )]
    pub maximum_allowed_max_fee_per_blob_gas: u64,
    #[arg(
        long = "eth.max-number-of-retries",
        default_value = "10",
        value_name = "UINT64",
        env = "ETHREX_MAX_NUMBER_OF_RETRIES",
        help_heading = "Eth options"
    )]
    pub max_number_of_retries: u64,
    #[arg(
        long = "eth.backoff-factor",
        default_value = "2",
        value_name = "UINT64",
        env = "ETHREX_BACKOFF_FACTOR",
        help_heading = "Eth options"
    )]
    pub backoff_factor: u64,
    #[arg(
        long = "eth.min-retry-delay",
        default_value = "96",
        value_name = "UINT64",
        env = "ETHREX_MIN_RETRY_DELAY",
        help_heading = "Eth options"
    )]
    pub min_retry_delay: u64,
    #[arg(
        long = "eth.max-retry-delay",
        default_value = "1800",
        value_name = "UINT64",
        env = "ETHREX_MAX_RETRY_DELAY",
        help_heading = "Eth options"
    )]
    pub max_retry_delay: u64,
}

impl Default for EthOptions {
    fn default() -> Self {
        Self {
            rpc_url: vec!["http://localhost:8545".to_string()],
            maximum_allowed_max_fee_per_gas: 10000000000,
            maximum_allowed_max_fee_per_blob_gas: 10000000000,
            max_number_of_retries: MAX_NUMBER_OF_RETRIES,
            backoff_factor: BACKOFF_FACTOR,
            min_retry_delay: MIN_RETRY_DELAY,
            max_retry_delay: MAX_RETRY_DELAY,
        }
    }
}

#[derive(Parser, Debug)]
pub struct WatcherOptions {
    #[arg(
        long = "l1.bridge-address",
        value_name = "ADDRESS",
        env = "ETHREX_WATCHER_BRIDGE_ADDRESS",
        help_heading = "L1 Watcher options",
        required_unless_present = "dev"
    )]
    pub bridge_address: Option<Address>,
    #[arg(
        long = "watcher.watch-interval",
        default_value = "12000", // One L1 slot
        value_name = "UINT64",
        env = "ETHREX_WATCHER_WATCH_INTERVAL",
        help = "How often the L1 watcher checks for new blocks in milliseconds.",
        help_heading = "L1 Watcher options"
    )]
    pub watch_interval_ms: u64,
    #[arg(
        long = "watcher.max-block-step",
        default_value = "5000",
        value_name = "UINT64",
        env = "ETHREX_WATCHER_MAX_BLOCK_STEP",
        help_heading = "L1 Watcher options"
    )]
    pub max_block_step: u64,
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
            bridge_address: None,
            watch_interval_ms: 1000,
            max_block_step: 5000,
            watcher_block_delay: 0,
        }
    }
}

#[derive(Parser, Debug)]
pub struct BlockProducerOptions {
    #[arg(
        long = "block-producer.block-time",
        default_value = "5000",
        value_name = "UINT64",
        env = "ETHREX_BLOCK_PRODUCER_BLOCK_TIME",
        help = "How often does the sequencer produce new blocks to the L1 in milliseconds.",
        help_heading = "Block producer options"
    )]
    pub block_time_ms: u64,
    #[arg(
        long = "block-producer.coinbase-address",
        value_name = "ADDRESS",
        env = "ETHREX_BLOCK_PRODUCER_COINBASE_ADDRESS",
        help_heading = "Block producer options",
        required_unless_present = "dev"
    )]
    pub coinbase_address: Option<Address>,
    #[arg(
        long,
        default_value = "2",
        value_name = "UINT64",
        env = "ETHREX_PROPOSER_ELASTICITY_MULTIPLIER",
        help_heading = "Proposer options"
    )]
    pub elasticity_multiplier: u64,
}

impl Default for BlockProducerOptions {
    fn default() -> Self {
        Self {
            block_time_ms: 5000,
            coinbase_address: Some(
                "0x0007a881cd95b1484fca47615b64803dad620c8d"
                    .parse()
                    .unwrap(),
            ),
            elasticity_multiplier: 2,
        }
    }
}

#[derive(Parser, Debug)]
pub struct CommitterOptions {
    #[arg(
        long = "committer.l1-private-key",
        value_name = "PRIVATE_KEY",
        value_parser = utils::parse_private_key,
        env = "ETHREX_COMMITTER_L1_PRIVATE_KEY",
        help_heading = "L1 Committer options",
        help = "Private key of a funded account that the sequencer will use to send commit txs to the L1.",
        conflicts_with_all = &["committer_remote_signer_url", "committer_remote_signer_public_key"],
        required_unless_present = "committer_remote_signer_url",
        required_unless_present = "dev"
    )]
    pub committer_l1_private_key: Option<SecretKey>,
    #[arg(
        long = "committer.remote-signer-url",
        value_name = "URL",
        env = "ETHREX_COMMITTER_REMOTE_SIGNER_URL",
        help_heading = "L1 Committer options",
        help = "URL of a Web3Signer-compatible server to remote sign instead of a local private key.",
        requires = "committer_remote_signer_public_key",
        required_unless_present = "committer_l1_private_key",
        required_unless_present = "dev"
    )]
    pub committer_remote_signer_url: Option<Url>,
    #[arg(
        long = "committer.remote-signer-public-key",
        value_name = "PUBLIC_KEY",
        value_parser = utils::parse_public_key,
        env = "ETHREX_COMMITTER_REMOTE_SIGNER_PUBLIC_KEY",
        help_heading = "L1 Committer options",
        help = "Public key to request the remote signature from.",
        requires = "committer_remote_signer_url",
    )]
    pub committer_remote_signer_public_key: Option<PublicKey>,
    #[arg(
        long = "l1.on-chain-proposer-address",
        value_name = "ADDRESS",
        env = "ETHREX_COMMITTER_ON_CHAIN_PROPOSER_ADDRESS",
        help_heading = "L1 Committer options",
        required_unless_present = "dev"
    )]
    pub on_chain_proposer_address: Option<Address>,
    #[arg(
        long = "committer.commit-time",
        default_value = "60000",
        value_name = "UINT64",
        env = "ETHREX_COMMITTER_COMMIT_TIME",
        help_heading = "L1 Committer options",
        help = "How often does the sequencer commit new blocks to the L1 in milliseconds."
    )]
    pub commit_time_ms: u64,
    #[arg(
        long = "committer.first-wake-up-time",
        value_name = "UINT64",
        env = "ETHREX_COMMITTER_FIRST_WAKE_UP_TIME",
        help_heading = "L1 Committer options",
        help = "Time to wait before the sequencer seals a batch when started. After committing the first batch, `committer.commit-time` will be used."
    )]
    pub first_wake_up_time_ms: Option<u64>,
    #[arg(
        long = "committer.arbitrary-base-blob-gas-price",
        default_value = "1000000000", // 1 Gwei
        value_name = "UINT64",
        env = "ETHREX_COMMITTER_ARBITRARY_BASE_BLOB_GAS_PRICE",
        help_heading = "L1 Committer options"
    )]
    pub arbitrary_base_blob_gas_price: u64,
}

impl Default for CommitterOptions {
    fn default() -> Self {
        Self {
            committer_l1_private_key: utils::parse_private_key(
                "0x385c546456b6a603a1cfcaa9ec9494ba4832da08dd6bcf4de9a71e4a01b74924",
            )
            .ok(),
            on_chain_proposer_address: None,
            commit_time_ms: 60000,
            first_wake_up_time_ms: None,
            arbitrary_base_blob_gas_price: 1_000_000_000,
            committer_remote_signer_url: None,
            committer_remote_signer_public_key: None,
        }
    }
}

#[derive(Parser, Debug)]
pub struct ProofCoordinatorOptions {
    #[arg(
        long = "proof-coordinator.l1-private-key",
        value_name = "PRIVATE_KEY",
        value_parser = utils::parse_private_key,
        env = "ETHREX_PROOF_COORDINATOR_L1_PRIVATE_KEY",
        help_heading = "Proof coordinator options",
        long_help = "Private key of of a funded account that the sequencer will use to send verify txs to the L1. Has to be a different account than --committer-l1-private-key.",
        conflicts_with_all = &["remote_signer_url", "remote_signer_public_key"],
        required_unless_present = "remote_signer_url",
        required_unless_present = "dev"
    )]
    pub proof_coordinator_l1_private_key: Option<SecretKey>,
    #[arg(
        long = "proof-coordinator.tdx-private-key",
        value_name = "PRIVATE_KEY",
        value_parser = utils::parse_private_key,
        env = "ETHREX_PROOF_COORDINATOR_TDX_PRIVATE_KEY",
        help_heading = "Proof coordinator options",
        long_help = "Private key of of a funded account that the TDX tool that will use to send the tdx attestation to L1.",
    )]
    pub proof_coordinator_tdx_private_key: Option<SecretKey>,
    #[arg(
        long = "proof-coordinator.remote-signer-url",
        value_name = "URL",
        env = "ETHREX_PROOF_COORDINATOR_REMOTE_SIGNER_URL",
        help_heading = "Proof coordinator options",
        help = "URL of a Web3Signer-compatible server to remote sign instead of a local private key.",
        requires = "remote_signer_public_key",
        required_unless_present = "proof_coordinator_l1_private_key",
        required_unless_present = "dev"
    )]
    pub remote_signer_url: Option<Url>,
    #[arg(
        long = "proof-coordinator.remote-signer-public-key",
        value_name = "PUBLIC_KEY",
        value_parser = utils::parse_public_key,
        env = "ETHREX_PROOF_COORDINATOR_REMOTE_SIGNER_PUBLIC_KEY",
        help_heading = "Proof coordinator options",
        help = "Public key to request the remote signature from.",
        requires = "remote_signer_url",
    )]
    pub remote_signer_public_key: Option<PublicKey>,
    #[arg(
        long = "proof-coordinator.addr",
        default_value = "127.0.0.1",
        value_name = "IP_ADDRESS",
        env = "ETHREX_PROOF_COORDINATOR_LISTEN_ADDRESS",
        help_heading = "Proof coordinator options",
        help = "Set it to 0.0.0.0 to allow connections from other machines."
    )]
    pub listen_ip: IpAddr,
    #[arg(
        long = "proof-coordinator.port",
        default_value = "3900",
        value_name = "UINT16",
        env = "ETHREX_PROOF_COORDINATOR_LISTEN_PORT",
        help_heading = "Proof coordinator options"
    )]
    pub listen_port: u16,
    #[arg(
        long = "proof-coordinator.send-interval",
        default_value = "5000",
        value_name = "UINT64",
        env = "ETHREX_PROOF_COORDINATOR_SEND_INTERVAL",
        help = "How often does the proof coordinator send proofs to the L1 in milliseconds.",
        help_heading = "Proof coordinator options"
    )]
    pub proof_send_interval_ms: u64,
}

impl Default for ProofCoordinatorOptions {
    fn default() -> Self {
        let proof_coordinator_l1_private_key = utils::parse_private_key(
            "0x39725efee3fb28614de3bacaffe4cc4bd8c436257e2c8bb887c4b5c4be45e76d",
        )
        .ok();
        Self {
            remote_signer_url: None,
            remote_signer_public_key: None,
            proof_coordinator_l1_private_key,
            listen_ip: IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)),
            listen_port: 3900,
            proof_send_interval_ms: 5000,
            proof_coordinator_tdx_private_key: None,
        }
    }
}
#[derive(Parser, Debug, Clone)]
pub struct AlignedOptions {
    #[arg(
        long,
        action = clap::ArgAction::SetTrue,
        default_value = "false",
        value_name = "ALIGNED_MODE",
        env = "ETHREX_ALIGNED_MODE",
        help_heading = "Aligned options"
    )]
    pub aligned: bool,
    #[arg(
        long,
        default_value = "5000",
        value_name = "ETHREX_ALIGNED_VERIFIER_INTERVAL_MS",
        env = "ETHREX_ALIGNED_VERIFIER_INTERVAL_MS",
        help_heading = "Aligned options"
    )]
    pub aligned_verifier_interval_ms: u64,
    #[arg(
        long = "aligned.beacon-url",
        value_name = "BEACON_URL",
        required_if_eq("aligned", "true"),
        env = "ETHREX_ALIGNED_BEACON_URL",
        help = "List of beacon urls to use.",
        help_heading = "Aligned options",
        num_args = 1..,
    )]
    pub beacon_url: Option<Vec<Url>>,
    #[arg(
        long,
        value_name = "ETHREX_ALIGNED_NETWORK",
        env = "ETHREX_ALIGNED_NETWORK",
        required_if_eq("aligned", "true"),
        default_value = "devnet",
        help = "L1 network name for Aligned sdk",
        help_heading = "Aligned options"
    )]
    pub aligned_network: Option<String>,

    #[arg(
        long = "aligned.fee-estimate",
        default_value = "instant",
        value_name = "FEE_ESTIMATE",
        env = "ETHREX_ALIGNED_FEE_ESTIMATE",
        help = "Fee estimate for Aligned sdk",
        help_heading = "Aligned options"
    )]
    pub fee_estimate: String,
}

impl Default for AlignedOptions {
    fn default() -> Self {
        Self {
            aligned: false,
            aligned_verifier_interval_ms: 5000,
            beacon_url: None,
            aligned_network: Some("devnet".to_string()),
            fee_estimate: "instant".to_string(),
        }
    }
}

#[derive(Parser, Default, Debug)]
pub struct BasedOptions {
    #[clap(flatten)]
    pub state_updater_opts: StateUpdaterOptions,
    #[clap(flatten)]
    pub block_fetcher: BlockFetcherOptions,
}

#[derive(Parser, Debug)]
pub struct StateUpdaterOptions {
    #[arg(
        long = "state-updater.sequencer-registry",
        value_name = "ADDRESS",
        env = "ETHREX_STATE_UPDATER_SEQUENCER_REGISTRY",
        required_if_eq("based", "true"),
        help_heading = "Based options"
    )]
    pub sequencer_registry: Option<Address>,
    #[arg(
        long = "state-updater.check-interval",
        default_value = "1000",
        value_name = "UINT64",
        env = "ETHREX_STATE_UPDATER_CHECK_INTERVAL",
        help_heading = "Based options"
    )]
    pub check_interval_ms: u64,
}

impl Default for StateUpdaterOptions {
    fn default() -> Self {
        Self {
            sequencer_registry: None,
            check_interval_ms: 1000,
        }
    }
}

#[derive(Parser, Debug)]
pub struct BlockFetcherOptions {
    #[arg(
        long = "block-fetcher.fetch_interval_ms",
        default_value = "5000",
        value_name = "UINT64",
        env = "ETHREX_BLOCK_FETCHER_FETCH_INTERVAL_MS",
        help_heading = "Based options"
    )]
    pub fetch_interval_ms: u64,
    #[arg(
        long,
        default_value = "5000",
        value_name = "UINT64",
        env = "ETHREX_BLOCK_FETCHER_FETCH_BLOCK_STEP",
        help_heading = "Based options"
    )]
    pub fetch_block_step: u64,
}

impl Default for BlockFetcherOptions {
    fn default() -> Self {
        Self {
            fetch_interval_ms: 5000,
            fetch_block_step: 5000,
        }
    }
}

#[derive(Parser, Debug)]
pub struct MonitorOptions {
    /// time in ms between two ticks.
    #[arg(short, long, default_value_t = 1000)]
    pub tick_rate: u64,
    #[arg(long)]
    batch_widget_height: Option<u16>,
}

impl Default for MonitorOptions {
    fn default() -> Self {
        Self {
            tick_rate: 1000,
            batch_widget_height: None,
        }
    }
}

#[derive(Parser)]
pub struct ProverClientOptions {
    #[arg(
        long = "backend",
        env = "PROVER_CLIENT_BACKEND",
        default_value = "exec",
        help_heading = "Prover client options",
        value_enum
    )]
    pub backend: Backend,
    #[arg(
        long = "proof-coordinator",
        value_name = "URL",
        env = "PROVER_CLIENT_PROOF_COORDINATOR_URL",
        help_heading = "Prover client options",
        help = "URL of the sequencer's proof coordinator"
    )]
    pub proof_coordinator_endpoint: Url,
    #[arg(
        long = "proving-time",
        value_name = "PROVING_TIME",
        env = "PROVER_CLIENT_PROVING_TIME",
        help = "Time to wait before requesting new data to prove",
        help_heading = "Prover client options",
        default_value_t = 5000
    )]
    pub proving_time_ms: u64,
    #[arg(
        long = "log.level",
        default_value_t = Level::INFO,
        value_name = "LOG_LEVEL",
        help = "The verbosity level used for logs.",
        long_help = "Possible values: info, debug, trace, warn, error",
        help_heading = "Prover client options"
    )]
    pub log_level: Level,
}

impl From<ProverClientOptions> for ProverConfig {
    fn from(config: ProverClientOptions) -> Self {
        Self {
            backend: config.backend,
            proof_coordinator: config.proof_coordinator_endpoint,
            proving_time_ms: config.proving_time_ms,
        }
    }
}

impl Default for ProverClientOptions {
    fn default() -> Self {
        Self {
            proof_coordinator_endpoint: Url::from_str("127.0.0.1:3900").expect("Invalid URL"),
            proving_time_ms: 5000,
            log_level: Level::INFO,
            backend: Backend::Exec,
        }
    }
}
