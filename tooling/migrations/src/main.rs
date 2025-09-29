mod cli;
mod utils;

use tracing::Level;
use tracing_subscriber::prelude::__tracing_subscriber_SubscriberExt;
use tracing_subscriber::{EnvFilter, Layer, Registry, filter::Directive, fmt, reload};

use crate::cli::CLI;
use clap::Parser;

pub fn init_tracing() -> reload::Handle<EnvFilter, Registry> {
    let log_filter = EnvFilter::builder()
        .with_default_directive(Directive::from(Level::TRACE))
        .from_env_lossy();

    let (filter, filter_handle) = reload::Layer::new(log_filter);

    let fmt_layer = fmt::layer().with_filter(filter);
    let subscriber = Box::new(Registry::default().with(fmt_layer));

    tracing::subscriber::set_global_default(subscriber).expect("setting default subscriber failed");

    filter_handle
}

#[tokio::main]
async fn main() {
    let CLI { command } = CLI::parse();

    init_tracing();

    command.run().await;
}
