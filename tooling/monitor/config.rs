#[derive(Clone, Debug)]
pub struct MonitorConfig {
    pub enabled: bool,
    /// time in ms between two ticks.
    pub tick_rate: u64,
    /// height in lines of the batch widget
    pub batch_widget_height: Option<u16>,
}
