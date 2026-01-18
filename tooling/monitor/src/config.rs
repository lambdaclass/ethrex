use ethrex_common::Address;
use std::fmt::Display;

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

/// Trait for providing sequencer status to the monitor.
/// This allows the monitor to be decoupled from the sequencer state implementation.
pub trait SequencerStatusProvider: Clone + Send + Sync + 'static {
    /// Get the current status of the sequencer
    fn status(&self) -> impl std::future::Future<Output = SequencerStatus> + Send;
}

/// Status of the sequencer
#[derive(Clone, Debug, Default)]
pub enum SequencerStatus {
    #[default]
    Starting,
    Running,
    Stopping,
    Stopped,
}

impl Display for SequencerStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SequencerStatus::Starting => write!(f, "Starting"),
            SequencerStatus::Running => write!(f, "Running"),
            SequencerStatus::Stopping => write!(f, "Stopping"),
            SequencerStatus::Stopped => write!(f, "Stopped"),
        }
    }
}
