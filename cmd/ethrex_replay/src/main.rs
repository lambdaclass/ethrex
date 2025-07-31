mod bench;
mod cache;
mod cli;
mod constants;
mod fetcher;
mod plot_composition;
mod run;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::ERROR)
        .init();

    if let Err(err) = cli::start().await {
        tracing::error!("{err:?}");
        std::process::exit(1);
    }
}
