use ethrex_common::Address;

/// Configuration for the monitor.
/// This contains all the configuration the monitor needs from the sequencer config.
#[derive(Clone, Debug)]
pub struct MonitorConfig {
    /// Whether the monitor is enabled
    pub enabled: bool,
    /// Time in ms between two ticks
    pub tick_rate: u64,
    /// Height in lines of the batch widget
    pub batch_widget_height: Option<u16>,
    /// L1 RPC URL for the eth client
    pub l1_rpc_url: reqwest::Url,
    /// On chain proposer address
    pub on_chain_proposer_address: Address,
    /// Bridge address
    pub bridge_address: Address,
    /// Whether this is a based rollup
    pub is_based: bool,
    /// Sequencer registry address (for based mode)
    pub sequencer_registry: Option<Address>,
    /// Osaka activation time
    pub osaka_activation_time: Option<u64>,
}
