use config::LoggingConfig;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::signal::unix::{signal, SignalKind};
use tracing::level_filters::LevelFilter;
use tracing_appender::{non_blocking::WorkerGuard, rolling::Rotation};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter, Layer};
use uuid::Uuid;

pub mod communication;
pub mod config;
pub mod db;
pub mod time;
pub mod transaction;

pub const DEFAULT_TRACING_ENV_FILTERS: &[&str] = &[
    "hyper::proto::h1=off",
    "trust_dns_proto=off",
    "trust_dns_resolver=off",
    "discv5=off",
    "hyper_util=off",
    "reqwest=info",
    "jsonrpsee=info",
    "alloy_transport_http=info",
    "alloy_rpc_client=info",
    "storage::db::mdbx=info",
];

/// Builds an environment filter for logging. Uses a default set of filters plus some optional
/// extras.
pub fn build_env_filter(default_level: LevelFilter, env_filters: Option<String>) -> EnvFilter {
    let env_builder = EnvFilter::builder().with_default_directive(default_level.into());

    let mut env_filter = if let Some(env_filters) = env_filters {
        env_builder.parse_lossy(env_filters)
    } else {
        env_builder.from_env_lossy()
    };

    for directive in DEFAULT_TRACING_ENV_FILTERS {
        env_filter = env_filter.add_directive(directive.parse().unwrap());
    }

    env_filter
}

/// Initialises tracing logger that creates daily log files.
pub fn init_tracing(config: LoggingConfig) -> WorkerGuard {
    let format = tracing_subscriber::fmt::format()
        .with_level(true)
        .with_thread_ids(false)
        .with_target(false);

    if config.enable_file_logging {
        let mut builder = tracing_appender::rolling::Builder::new()
            .rotation(Rotation::DAILY)
            .max_log_files(config.max_files);

        if let Some(prefix) = config.prefix {
            builder = builder.filename_prefix(prefix);
        }

        let appender = builder
            .build(config.path)
            .expect("failed to create log appender!");
        let (writer, guard) = tracing_appender::non_blocking(appender);

        let stdout_layer = tracing_subscriber::fmt::layer()
            .event_format(format.clone())
            .with_filter(build_env_filter(config.level, config.filters.clone()));

        let file_layer = tracing_subscriber::fmt::layer()
            .event_format(format)
            .with_ansi(false)
            .with_writer(writer)
            .with_filter(build_env_filter(config.level, config.filters.clone()));

        tracing_subscriber::registry()
            .with(stdout_layer.and_then(file_layer))
            .init();

        guard
    } else {
        let (writer, guard) = tracing_appender::non_blocking(std::io::stdout());

        let stdout_layer = tracing_subscriber::fmt::layer()
            .event_format(format)
            .with_writer(writer)
            .with_filter(build_env_filter(config.level, config.filters.clone()));

        tracing_subscriber::registry().with(stdout_layer).init();
        guard
    }
}

pub fn initialize_test_tracing(level: LevelFilter) {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .with_max_level(level)
        .with_thread_names(true)
        .with_file(true)
        .with_line_number(true)
        .init();
}

pub async fn wait_for_signal() -> eyre::Result<()> {
    let mut sigint = signal(SignalKind::interrupt())?;
    let mut sigterm = signal(SignalKind::terminate())?;

    tokio::select! {
        _ = sigint.recv() => {}
        _ = sigterm.recv() => {}
    }

    Ok(())
}

/// Strips all top-level (non-generic) path segments and returns only the last segment,
/// including any generic parameters.
///
/// For example:
/// - `foo::Bar::Baz<T::b::db>` => `Baz<T::b::db>`
/// - `my_crate::some_mod::Type<U>` => `Type<U>`
/// - `JustType` => `JustType`
///
/// If there are no top-level "::" separators, the original string is returned.
pub fn strip_namespace(s: &str) -> &str {
    let mut depth = 0;
    let mut start = 0;
    let bytes = s.as_bytes();
    let len = bytes.len();
    let mut i = 0;

    while i < len {
        match bytes[i] {
            b'<' => {
                // Entering a generic parameter region, increase depth
                depth += 1;
            }
            b'>' => {
                // Leaving a generic parameter region, decrease depth
                if depth > 0 {
                    depth -= 1;
                }
            }
            b':' => {
                // Check if we have "::" at top-level (depth == 0)
                if depth == 0 && i + 1 < len && bytes[i + 1] == b':' {
                    // Update start to skip past this "::"
                    start = i + 2;
                    // Skip the extra ':' in the loop
                    i += 1;
                }
            }
            _ => {}
        }
        i += 1;
    }

    &s[start..]
}

pub fn full_last_part_of_typename<T>() -> &'static str {
    strip_namespace(std::any::type_name::<T>())
}

pub fn last_part_of_typename<T>() -> &'static str {
    let full_name = strip_namespace(std::any::type_name::<T>());
    if let Some(generic_start) = full_name.find('<') {
        &full_name[..generic_start]
    } else {
        full_name
    }
}

pub fn uuid() -> Uuid {
    Uuid::new_v4()
}

pub fn utcnow_sec() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}
