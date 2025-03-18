use clap::{Arg, ArgAction, Command};
use ethrex_p2p::types::Node;
use tracing::Level;

pub fn cli() -> Command {
    let cmd = Command::new("ethrex")
        .about("ethrex Execution client")
        .author("Lambdaclass")
        .arg(
            Arg::new("http.addr")
                .long("http.addr")
                .default_value("localhost")
                .value_name("ADDRESS")
                .action(ArgAction::Set),
        )
        .arg(
            Arg::new("http.port")
                .long("http.port")
                .default_value("8545")
                .value_name("PORT")
                .action(ArgAction::Set),
        )
        .arg(
            Arg::new("log.level")
                .long("log.level")
                .default_value(Level::INFO.as_str())
                .value_name("LOG_LEVEL")
                .required(false)
                .action(ArgAction::Set),
        )
        .arg(
            Arg::new("authrpc.addr")
                .long("authrpc.addr")
                .default_value("localhost")
                .value_name("ADDRESS")
                .action(ArgAction::Set),
        )
        .arg(
            Arg::new("authrpc.port")
                .long("authrpc.port")
                .default_value("8551")
                .value_name("PORT")
                .action(ArgAction::Set),
        )
        .arg(
            Arg::new("authrpc.jwtsecret")
                .long("authrpc.jwtsecret")
                .default_value("jwt.hex")
                .value_name("JWTSECRET_PATH")
                .action(ArgAction::Set),
        )
        .arg(
            Arg::new("p2p.enabled")
                .long("p2p.enabled")
                .required(false)
                .default_value(if cfg!(feature = "l2") { "false" } else { "true" })
                .value_name("P2P_ENABLED")
                .action(clap::ArgAction::SetTrue),
        )
        .arg(
            Arg::new("p2p.addr")
                .long("p2p.addr")
                .default_value("0.0.0.0")
                .value_name("ADDRESS")
                .action(ArgAction::Set),
        )
        .arg(
            Arg::new("p2p.port")
                .long("p2p.port")
                .default_value("30303")
                .value_name("PORT")
                .action(ArgAction::Set),
        )
        .arg(
            Arg::new("discovery.addr")
                .long("discovery.addr")
                .default_value("0.0.0.0")
                .value_name("ADDRESS")
                .action(ArgAction::Set),
        )
        .arg(
            Arg::new("discovery.port")
                .long("discovery.port")
                .default_value("30303")
                .value_name("PORT")
                .action(ArgAction::Set),
        )
        .arg(
            Arg::new("network")
                .long("network")
                .value_name("GENESIS_FILE_PATH")
                .action(ArgAction::Set),
        )
        .arg(
            Arg::new("bootnodes")
                .long("bootnodes")
                .value_name("BOOTNODE_LIST")
                .value_parser(clap::value_parser!(Node))
                .value_delimiter(',')
                .num_args(1..)
                .action(ArgAction::Set),
        )
        .arg(
            Arg::new("datadir")
                .long("datadir")
                .value_name("DATABASE_DIRECTORY")
                .action(ArgAction::Set)
                .help("If the datadir is the word `memory`, ethrex will use the InMemory Engine"),
        )
        .arg(
            Arg::new("syncmode")
                .long("syncmode")
                .required(false)
                .default_value("full")
                .value_name("SYNC_MODE"),
        )
        .arg(
            Arg::new("metrics.port")
                .long("metrics.port")
                .required(false)
                .value_name("PROMETHEUS_METRICS_PORT"),
        )
        .arg(
            Arg::new("dev")
                .long("dev")
                .required(false)
                .action(clap::ArgAction::SetTrue) // This turns the flag into a boolean
                .help("Used to create blocks without requiring a Consensus Client"),
        )
        .arg(
            Arg::new("evm")
                .long("evm")
                .required(false)
                .default_value("revm")
                .value_name("EVM_BACKEND")
                .help("Has to be `levm` or `revm`"),
        )
        .subcommand(
            Command::new("removedb").about("Remove the database").arg(
                Arg::new("datadir")
                    .long("datadir")
                    .value_name("DATABASE_DIRECTORY")
                    .action(ArgAction::Set),
            ),
        )
        .subcommand(
            Command::new("import")
                .about("Import blocks to the database") 
                .arg(
                    Arg::new("path")
                        .required(true)
                        .value_name("FILE_PATH/FOLDER")
                        .help("Path to a RLP chain file or a folder containing files with individual Blocks")
                        .action(ArgAction::Set),
                )
                .arg(
                    Arg::new("removedb")
                        .long("removedb")
                        .required(false)
                        .action(clap::ArgAction::SetTrue)
                )
        );
    #[cfg(feature = "based")]
    let cmd = cmd
        .arg(
            Arg::new("gateway.addr")
                .long("gateway.addr")
                .default_value("0.0.0.0")
                .value_name("GATEWAY_ADDRESS")
                .action(ArgAction::Set),
        )
        .arg(
            Arg::new("gateway.eth_port")
                .long("gateway.eth_port")
                .default_value("8546")
                .value_name("GATEWAY_ETH_PORT")
                .action(ArgAction::Set),
        )
        .arg(
            Arg::new("gateway.auth_port")
                .long("gateway.auth_port")
                .default_value("8553")
                .value_name("GATEWAY_AUTH_PORT")
                .action(ArgAction::Set),
        )
        .arg(
            Arg::new("gateway.jwtsecret")
                .long("gateway.jwtsecret")
                .default_value("jwt.hex")
                .value_name("GATEWAY_JWTSECRET_PATH")
                .action(ArgAction::Set),
        );
    #[cfg(feature="l2")]
    let cmd = cmd.arg(
        Arg::new("sponsorable_addresses")
            .long("sponsorable_addresses")
            .value_name("SPONSORABLE_ADDRESSES_PATH")
            .action(ArgAction::Set)
            .required(false)
            .help("Path to a file containing addresses of contracts to wich ethrex_SendTransaction should sponsor txs"),
    );

    cmd
}
