use ethrex::chain::{Chain, ETHEREUM, NodeComponents, run};
use serde::Deserialize;
use std::time::Duration;
use tracing::info;

const LATEST_VERSION_URL: &str = "https://api.github.com/repos/lambdaclass/ethrex/releases/latest";

ethrex::global_allocator!();

/// Fetches the latest release version on github
/// Returns None if there was an error when requesting the latest version
async fn latest_release_version() -> Option<String> {
    #[derive(Deserialize)]
    struct Release {
        tag_name: String,
    }
    let client = reqwest::Client::new();
    let response = client
        .get(LATEST_VERSION_URL)
        .header("User-Agent", "ethrex")
        .send()
        .await
        .ok()?;
    if !response.status().is_success() {
        None
    } else {
        Some(
            response
                .json::<Release>()
                .await
                .ok()?
                .tag_name
                .trim_start_matches("v")
                .to_string(),
        )
    }
}

/// Reads current crate version
fn current_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// Returns true if the received latest version is higher than the current ethrex version
fn is_higher_than_current(latest_version: &str) -> bool {
    let current_version_numbers = current_version()
        .split(".")
        .map(|c| c.parse::<u64>().unwrap_or_default());
    let latest_version_numbers = latest_version
        .split(".")
        .map(|c| c.parse::<u64>().unwrap_or_default());
    for (current, latest) in current_version_numbers.zip(latest_version_numbers) {
        match current.cmp(&latest) {
            std::cmp::Ordering::Less => return true,
            std::cmp::Ordering::Equal => {}
            std::cmp::Ordering::Greater => return false,
        };
    }
    false
}

/// Checks if the latest released version is higher than the current version and emits an info log
/// Won't emit a log line if the current version is newer or equal, or if there was a problem reading either version
async fn check_version_update() {
    if let Some(latest_version) = latest_release_version().await
        && is_higher_than_current(&latest_version)
    {
        info!(
            "There is a newer ethrex version available, current version: {} vs latest version: {latest_version}",
            current_version()
        );
    }
}

/// Checks if there is a newer ethrex verison available every hour
async fn periodically_check_version_update() {
    let mut interval = tokio::time::interval(Duration::from_secs(60 * 60));
    loop {
        interval.tick().await;
        check_version_update().await;
    }
}

fn main() -> eyre::Result<()> {
    // Looking for new releases is this binary's own business, so it is started
    // here and not by `run`, which treats every chain alike.
    let chain = Chain {
        components: |_, _| {
            tokio::spawn(periodically_check_version_update());
            Ok(NodeComponents::default())
        },
        ..ETHEREUM
    };
    run(chain, GLOBAL_ALLOCATOR)
}
