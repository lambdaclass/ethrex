//! Glamsterdam banner, logged once this node has executed or built the chain's first
//! Amsterdam block.

use std::sync::Once;

use ethrex_common::{H256, types::ChainConfig};
use tracing::info;

/// Polar bear adapted from Lodestar's Gloas fork banner (ChainSafe, Apache-2.0).
const BANNER: &str = include_str!("glamsterdam_banner.txt");

static SHOWN: Once = Once::new();

/// Whether this process has already logged the banner.
pub(crate) fn shown() -> bool {
    SHOWN.is_completed()
}

/// Whether a block at `timestamp` whose parent is at `parent_timestamp` is the chain's
/// first Amsterdam block: Amsterdam applies to it but not to its parent.
pub(crate) fn is_first_amsterdam_block(
    config: &ChainConfig,
    parent_timestamp: u64,
    timestamp: u64,
) -> bool {
    config.is_amsterdam_activated(timestamp) && !config.is_amsterdam_activated(parent_timestamp)
}

/// Logs the banner line by line, so every line carries the log prefix. Only the first
/// call in a process logs anything: a node that builds the first Amsterdam block also
/// imports it afterwards, and the banner should appear once.
pub(crate) fn log_once(block_number: u64, block_hash: H256) {
    SHOWN.call_once(|| {
        info!("");
        for line in BANNER.lines() {
            info!("{line}");
        }
        info!("");
        info!(
            "                  ✦ GLAMSTERDAM ✦ · Amsterdam is live from block {block_number} · {block_hash}"
        );
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config_with_amsterdam_at(time: u64) -> ChainConfig {
        ChainConfig {
            amsterdam_time: Some(time),
            ..Default::default()
        }
    }

    #[test]
    fn first_amsterdam_block_is_the_one_crossing_the_fork_time() {
        let config = config_with_amsterdam_at(100);
        assert!(is_first_amsterdam_block(&config, 88, 100));
        // Missed slots: the first Amsterdam block can land well after the fork time.
        assert!(is_first_amsterdam_block(&config, 88, 136));
    }

    #[test]
    fn blocks_on_one_side_of_the_fork_are_not_first() {
        let config = config_with_amsterdam_at(100);
        assert!(!is_first_amsterdam_block(&config, 76, 88));
        assert!(!is_first_amsterdam_block(&config, 100, 112));
    }

    #[test]
    fn no_first_amsterdam_block_without_a_scheduled_fork() {
        let config = ChainConfig::default();
        assert!(!is_first_amsterdam_block(&config, 0, u64::MAX));
    }
}
