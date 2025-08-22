use std::str::FromStr;
use tracing_subscriber::filter::Directive;

mod bench;
mod block_run_report;
mod cache;
mod cli;
mod fetcher;
mod plot_composition;
mod run;

#[cfg(feature = "jemalloc")]
use jemallocator::Jemalloc;

#[cfg(feature = "jemalloc")]
#[global_allocator]
static GLOBAL: Jemalloc = Jemalloc;

#[tokio::main]
async fn main() {
    let log_filter = tracing_subscriber::EnvFilter::builder()
        .with_default_directive(
            // Filters all sp1-executor logs (clock and program counter information)
            Directive::from_str("sp1_core_executor::executor=off").expect("this can't fail"),
        )
        .from_env_lossy()
        .add_directive(Directive::from(tracing::Level::INFO));
    tracing::subscriber::set_global_default(
        tracing_subscriber::FmtSubscriber::builder()
            .with_env_filter(log_filter)
            .finish(),
    )
    .expect("setting default subscriber failed");
    if let Err(e) = cli::start().await {
        tracing::error!("{e:?}");
        std::process::exit(1);
    }
}
