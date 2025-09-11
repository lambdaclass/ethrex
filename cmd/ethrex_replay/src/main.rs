use clap::Parser;
use ethrex_replay::cli::EthrexReplayCLI;
use std::str::FromStr;
use tracing_subscriber::filter::Directive;

#[cfg(feature = "jemalloc")]
use tikv_jemallocator::Jemalloc;

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
        .add_directive(Directive::from(tracing::Level::INFO))
        .add_directive(Directive::from_str("ethrex_storage::store=off").expect("this can't fail"))
        .add_directive(
            Directive::from_str("ethrex_storage_rollup::store=off").expect("this can't fail"),
        )
        .add_directive(
            Directive::from_str("ethrex_l2::sequencer::block_producer::payload_builder=off")
                .expect("this can't fail"),
        )
        .add_directive(
            Directive::from_str("ethrex_blockchain::payload=off").expect("this can't fail"),
        );

    tracing::subscriber::set_global_default(
        tracing_subscriber::FmtSubscriber::builder()
            .with_env_filter(log_filter)
            .finish(),
    )
    .expect("setting default subscriber failed");

    let EthrexReplayCLI { command } = EthrexReplayCLI::parse();

    if let Err(e) = command.run().await {
        tracing::error!("{e:?}");
        std::process::exit(1);
    }
}
