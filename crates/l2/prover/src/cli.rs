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
        long = "http.addr",
        value_name = "IP_ADDRESS",
        env = "PROVER_CLIENT_PROVER_CLIENT_ADDRESS",
        help_heading = "Prover client options"
    )]
    pub http_addr: String,
    #[arg(
        long = "http.port",
        value_name = "PORT",
        env = "PROVER_CLIENT_PROVER_CLIENT_PORT",
        help_heading = "Prover client options"
    )]
    pub http_port: u16,
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
            http_addr: config.http_addr,
            http_port: config.http_port,
            proving_time_ms: config.proving_time_ms,
            aligned_mode: config.aligned,
        }
    }
}

impl Default for ProverClientOptions {
    fn default() -> Self {
        Self {
            http_addr: "127.0.0.1".to_string(),
            http_port: 3900,
            proving_time_ms: 5000,
            log_level: Level::INFO,
            aligned: false,
        }
    }
}
