use clap::Parser;
use ethrex_prover_lib::config::ProverConfig;
use tracing::Level;

#[derive(Parser)]
pub struct ProverCLI {
    #[command(flatten)]
    pub prover_client_options: ProverClientOptions,
}

#[derive(Parser)]
pub struct ProverClientOptions {
    #[arg(
        long = "prover-server-endpoint",
        value_name = "PROVER_SERVER_ENDPOINT",
        env = "PROVER_CLIENT_PROVER_SERVER_ENDPOINT",
        help = "Endpoint address where the server is running",
        help_heading = "Prover client options",
        default_value = "localhost:3900"
    )]
    pub prover_server_endpoint: String,
    #[arg(
        long = "proving-time",
        value_name = "PROVING_TIME_MS",
        env = "PROVER_CLIENT_PROVING_TIME_MS",
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
        help_heading = "Prover client options")]
    pub log_level: Level,
    #[arg(
        long,
        default_value_t = false,
        value_name = "BOOLEAN",
        env = "PROVER_CLIENT_ALIGNED",
        help = "Activate aligned proving system",
        help_heading = "Prover client options"
    )]
    pub aligned: bool,
}

impl From<ProverClientOptions> for ProverConfig {
    fn from(config: ProverClientOptions) -> Self {
        Self {
            prover_server_endpoint: config.prover_server_endpoint,
            proving_time_ms: config.proving_time_ms,
            aligned_mode: config.aligned,
        }
    }
}

impl Default for ProverClientOptions {
    fn default() -> Self {
        Self {
            prover_server_endpoint: "localhost:3900".to_string(),
            proving_time_ms: 5000,
            log_level: Level::INFO,
            aligned: false,
        }
    }
}
