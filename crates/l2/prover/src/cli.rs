use clap::Parser;
use ethrex_l2::utils::config::prover::ProverConfig;
use tracing::Level;

#[derive(Parser, Debug)]
pub struct ProverClientOptions {
    #[arg(
        long = "prover-server-endpoint",
        value_name = "PROVER_SERVER_ENDPOINT",
        env = "PROVER_CLIENT_PROVER_SERVER_ENDPOINT",
        help = "Path to a file containing addresses of contracts to which ethrex_SendTransaction should sponsor txs",
        help_heading = "Prover client options",
        default_value = "localhost:3900"
    )]
    pub prover_server_endpoint: String,
    #[arg(
        long = "proving-time",
        value_name = "PROVING_TIME_MS",
        env = "PROVER_CLIENT_PROVING_TIME_MS",
        help = "Path to a file containing addresses of contracts to which ethrex_SendTransaction should sponsor txs",
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
        help_heading = "Prover client options")]
    pub log_level: Level,
}

impl From<ProverClientOptions> for ProverConfig {
    fn from(config: ProverClientOptions) -> Self {
        Self {
            prover_server_endpoint: config.prover_server_endpoint,
            proving_time_ms: config.proving_time_ms,
        }
    }
}

impl Default for ProverClientOptions {
    fn default() -> Self {
        Self {
            prover_server_endpoint: "localhost:3900".to_string(),
            proving_time_ms: 5000,
            log_level: Level::INFO,
        }
    }
}
