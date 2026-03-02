#[derive(Clone, Debug)]
pub struct MonitorConfig {
    pub enabled: bool,
    /// time in ms between two ticks.
    pub tick_rate: u64,
    /// height in lines of the batch widget
    pub batch_widget_height: Option<u16>,
    pub on_chain_proposer_address: ethrex_common::Address,
    pub bridge_address: ethrex_common::Address,
    pub sequencer_registry_address: Option<ethrex_common::Address>,
    pub rpc_urls: Vec<reqwest::Url>,
    pub is_based: bool,
    pub osaka_activation_time: Option<u64>,
}
