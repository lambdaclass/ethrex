use crate::{cli::Options as NodeOptions, utils};
use clap::Parser;
use ethrex_common::Address;
use ethrex_l2::{
    BlockProducerConfig, CommitterConfig, EthConfig, L1WatcherConfig, ProofCoordinatorConfig,
    SequencerConfig,
};
use ethrex_rpc::clients::eth::get_address_from_secret_key;
use reqwest::Url;
use secp256k1::SecretKey;
use std::net::{IpAddr, Ipv4Addr};

#[derive(Parser)]
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
    #[arg(long, default_value = "0xffd790338a2798b648806fc8635ac7bf14af15425fed0c8f25bcc5febaa9b192", value_parser = utils::parse_private_key, env = "SPONSOR_PRIVATE_KEY", help = "The private key of ethrex L2 transactions sponsor.", help_heading = "L2 options")]
    pub sponsor_private_key: SecretKey,
    #[cfg(feature = "based")]
    #[command(flatten)]
    pub based_opts: BasedOptions,
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
            #[cfg(feature = "based")]
            based_opts: BasedOptions::default(),
        }
    }
}

#[derive(Parser, Default)]
pub struct SequencerOptions {
    #[command(flatten)]
    pub eth_opts: EthOptions,
    #[command(flatten)]
    pub watcher_opts: WatcherOptions,
    #[command(flatten)]
    pub proposer_opts: BlockProducerOptions,
    #[command(flatten)]
    pub committer_opts: CommitterOptions,
    #[command(flatten)]
    pub proof_coordinator_opts: ProofCoordinatorOptions,
}

impl From<SequencerOptions> for SequencerConfig {
    fn from(opts: SequencerOptions) -> Self {
        Self {
            block_producer: BlockProducerConfig {
                block_time_ms: opts.proposer_opts.block_time_ms,
                coinbase_address: opts.proposer_opts.coinbase_address,
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
                maximum_allowed_max_fee_per_gas: opts.eth_opts.maximum_allowed_max_fee_per_gas,
                maximum_allowed_max_fee_per_blob_gas: opts
                    .eth_opts
                    .maximum_allowed_max_fee_per_blob_gas,
            },
            l1_watcher: L1WatcherConfig {
                bridge_address: opts.watcher_opts.bridge_address,
                check_interval_ms: opts.watcher_opts.watch_interval_ms,
                max_block_step: opts.watcher_opts.max_block_step.into(),
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
        }
    }
}

#[derive(Parser)]
pub struct EthOptions {
    #[arg(
        long = "eth.rpc-url",
        value_name = "RPC_URL",
        env = "ETHREX_ETH_RPC_URL",
        help_heading = "Eth options"
    )]
    pub rpc_url: Url,
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
}

impl Default for EthOptions {
    fn default() -> Self {
        Self {
            rpc_url: Url::parse("http://localhost:8545").unwrap(),
            maximum_allowed_max_fee_per_gas: Default::default(),
            maximum_allowed_max_fee_per_blob_gas: Default::default(),
        }
    }
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
        long = "watcher.watch-interval-ms",
        default_value = "1000",
        value_name = "UINT64",
        env = "ETHREX_WATCHER_WATCH_INTERVAL_MS",
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
}

impl Default for WatcherOptions {
    fn default() -> Self {
        Self {
            bridge_address: "0x266ffef34e21a7c4ce2e0e42dc780c2c273ca440"
                .parse()
                .unwrap(),
            watch_interval_ms: 1000,
            max_block_step: 5000,
        }
    }
}

#[derive(Parser, Default)]
pub struct BlockProducerOptions {
    #[arg(
        long = "block-producer.block-time-ms",
        default_value = "5000",
        value_name = "UINT64",
        env = "ETHREX_BLOCK_PRODUCER_BLOCK_TIME_MS",
        help_heading = "L1 Watcher options"
    )]
    pub block_time_ms: u64,
    #[arg(
        long = "block-producer.coinbase-address",
        value_name = "ADDRESS",
        env = "ETHREX_BLOCK_PRODUCER_COINBASE_ADDRESS",
        help_heading = "Block producer options"
    )]
    pub coinbase_address: Address,
}

#[derive(Parser)]
pub struct CommitterOptions {
    #[arg(
        long = "committer.l1-private-key",
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
        long = "committer.commit-time-ms",
        default_value = "60000",
        value_name = "UINT64",
        env = "ETHREX_COMMITTER_COMMIT_TIME_MS",
        help_heading = "L1 Committer options",
        help = "How often does the sequencer commit new blocks to the L1."
    )]
    pub commit_time_ms: u64,
    #[arg(
        long = "committer.arbitrary-base-blob-gas-price",
        default_value = "1000000000", // 1 Gwei
        value_name = "UINT64",
        env = "ETHREX_COMMITTER_ARBITRARY_BASE_BLOB_GAS_PRICE",
        help_heading = "L1 Committer options"
    )]
    pub arbitrary_base_blob_gas_price: u64,
    #[arg(
        long = "committer.validium",
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
        long = "proof-coordinator.l1-private-key",
        value_name = "PRIVATE_KEY",
        value_parser = utils::parse_private_key,
        env = "ETHREX_PROOF_COORDINATOR_L1_PRIVATE_KEY",
        help_heading = "Proof coordinator options",
        long_help = "Private key of of a funded account that the sequencer will use to send verify txs to the L1. Has to be a different account than --committer-l1-private-key.",
    )]
    pub proof_coordinator_l1_private_key: SecretKey,
    #[arg(
        long = "proof-coordinator.address",
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
        long = "proof-coordinator.send-interval-ms",
        default_value = "5000",
        value_name = "UINT64",
        env = "ETHREX_PROOF_COORDINATOR_SEND_INTERVAL_MS",
        help_heading = "Proof coordinator options"
    )]
    pub proof_send_interval_ms: u64,
    #[clap(
        long = "proof-coordinator.dev-mode",
        default_value = "true",
        value_name = "BOOLEAN",
        env = "ETHREX_PROOF_COORDINATOR_DEV_MODE",
        help_heading = "Proof coordinator options"
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

#[cfg(feature = "based")]
#[derive(Parser, Default)]
pub struct BasedOptions {
    #[arg(
        long = "gateway.addr",
        default_value = "0.0.0.0",
        value_name = "GATEWAY_ADDRESS",
        env = "GATEWAY_ADDRESS",
        help_heading = "Based options"
    )]
    pub gateway_addr: String,
    #[arg(
        long = "gateway.eth_port",
        default_value = "8546",
        value_name = "GATEWAY_ETH_PORT",
        env = "GATEWAY_ETH_PORT",
        help_heading = "Based options"
    )]
    pub gateway_eth_port: String,
    #[arg(
        long = "gateway.auth_port",
        default_value = "8553",
        value_name = "GATEWAY_AUTH_PORT",
        env = "GATEWAY_AUTH_PORT",
        help_heading = "Based options"
    )]
    pub gateway_auth_port: String,
    #[arg(
        long = "gateway.jwtsecret",
        default_value = "jwt.hex",
        value_name = "GATEWAY_JWTSECRET_PATH",
        env = "GATEWAY_JWTSECRET_PATH",
        help_heading = "Based options"
    )]
    pub gateway_jwtsecret: String,
    #[arg(
        long = "gateway.pubkey",
        value_name = "GATEWAY_PUBKEY",
        env = "GATEWAY_PUBKEY",
        help_heading = "Based options"
    )]
    pub gateway_pubkey: String,
}
