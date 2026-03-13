use crc32fast::Hasher;
use ethereum_types::H32;
use ethrex_common::types::{BlockHash, BlockNumber, ForkId};

use crate::bor_config::BorConfig;

/// Gathers all fork block numbers that contribute to the Polygon fork ID.
///
/// Bor computes fork IDs using ALL forks: both EVM-level forks (London,
/// Shanghai, Cancun, Prague) and Polygon-specific forks (Jaipur, Delhi, …).
/// On Polygon, these are all block-number-activated (no timestamp forks).
///
/// Returns sorted, deduplicated, non-zero fork block numbers.
pub fn gather_polygon_forks(bor_config: &BorConfig) -> Vec<u64> {
    let mut forks: Vec<u64> = [
        // EVM-level forks (stored as block numbers in Bor)
        bor_config.london_block,
        bor_config.shanghai_block,
        bor_config.cancun_block,
        bor_config.prague_block,
        // Polygon-specific forks
        bor_config.jaipur_block,
        bor_config.delhi_block,
        bor_config.indore_block,
        bor_config.ahmedabad_block,
        bor_config.bhilai_block,
        bor_config.rio_block,
        bor_config.madhugiri_block,
        bor_config.madhugiri_pro_block,
        bor_config.dandeli_block,
        bor_config.lisovo_block,
        // NOTE: LisovoProBlock and GiuglianoBlock are intentionally excluded.
        // Bor's GatherForks() does not include them in fork ID computation.
    ]
    .into_iter()
    .flatten()
    .filter(|&b| b != 0) // Forks at genesis don't contribute
    .collect();

    forks.sort();
    forks.dedup();
    forks
}

/// Computes the Polygon fork ID per EIP-2124.
///
/// Uses the Polygon genesis hash and Polygon-specific fork schedule (from BorConfig).
/// Polygon uses only block-number-based forks (no timestamp forks).
pub fn polygon_fork_id(
    genesis_hash: BlockHash,
    bor_config: &BorConfig,
    head_block_number: BlockNumber,
) -> ForkId {
    let forks = gather_polygon_forks(bor_config);

    let mut hasher = Hasher::new();
    hasher.update(genesis_hash.as_bytes());

    let mut last_included = 0u64;

    for &activation in &forks {
        if activation <= head_block_number {
            if activation != last_included {
                hasher.update(&activation.to_be_bytes());
                last_included = activation;
            }
        } else {
            // This is the next upcoming fork
            let fork_hash = H32::from_slice(&hasher.finalize().to_be_bytes());
            return ForkId {
                fork_hash,
                fork_next: activation,
            };
        }
    }

    // All known forks passed
    let fork_hash = H32::from_slice(&hasher.finalize().to_be_bytes());
    ForkId {
        fork_hash,
        fork_next: 0,
    }
}

/// Validates a remote Polygon fork ID against the local state per EIP-2124.
///
/// Uses the Polygon fork schedule (from BorConfig) instead of Ethereum forks.
/// Polygon only has block-number-based forks, so `head_block_number` is used
/// for all comparisons (no timestamp forks).
pub fn polygon_is_fork_id_valid(
    genesis_hash: BlockHash,
    bor_config: &BorConfig,
    head_block_number: BlockNumber,
    remote: &ForkId,
) -> bool {
    let local = polygon_fork_id(genesis_hash, bor_config, head_block_number);
    let forks = gather_polygon_forks(bor_config);

    // Rule 1: Same hash — compatible if remote's next fork hasn't passed locally.
    if remote.fork_hash == local.fork_hash {
        if remote.fork_next != 0 && remote.fork_next <= head_block_number {
            return false;
        }
        return true;
    }

    // Build all valid (fork_hash, fork_next) combinations from the Polygon schedule.
    let combinations = polygon_fork_combinations(&forks, genesis_hash);

    let mut is_subset = true;
    for (hash, next) in &combinations {
        if is_subset {
            // Rule 2: Remote is a subset of our past forks.
            if remote.fork_hash == *hash && remote.fork_next == *next {
                return true;
            }
        } else {
            // Rule 3: Remote is a superset of our past forks.
            if remote.fork_hash == *hash {
                return true;
            }
        }
        if *hash == local.fork_hash {
            is_subset = false;
        }
    }

    // Rule 4: No match — incompatible.
    false
}

/// Builds all valid (fork_hash, fork_next) combinations from a Polygon fork schedule.
fn polygon_fork_combinations(forks: &[u64], genesis_hash: BlockHash) -> Vec<(H32, u64)> {
    let mut combinations = Vec::new();
    let mut hasher = Hasher::new();
    hasher.update(genesis_hash.as_bytes());
    let mut last = 0u64;
    for &activation in forks {
        if activation == last {
            continue;
        }
        combinations.push((
            H32::from_slice(&hasher.clone().finalize().to_be_bytes()),
            activation,
        ));
        hasher.update(&activation.to_be_bytes());
        last = activation;
    }
    combinations.push((H32::from_slice(&hasher.finalize().to_be_bytes()), 0));
    combinations
}

/// Returns the minimum possible cumulative total difficulty at a given block.
///
/// On Polygon, each block has difficulty 1 (in-turn) or higher (out-of-turn).
/// The minimum TD at block N is N (every block in-turn with difficulty 1).
///
/// For the P2P status exchange, use the actual cumulative TD from storage
/// instead of this estimate. Unlike Ethereum PoS (where TD is fixed after
/// the merge), Polygon's TD grows with every block.
pub fn polygon_min_total_difficulty(block_number: BlockNumber) -> u64 {
    block_number
}

#[cfg(test)]
mod tests {
    use super::*;
    use ethereum_types::H256;
    use std::str::FromStr;

    fn mainnet_bor_config() -> BorConfig {
        serde_json::from_str(
            r#"{
                "period": {"0": 2},
                "producerDelay": {"0": 6},
                "sprint": {"0": 64, "38189056": 16},
                "backupMultiplier": {"0": 2},
                "validatorContract": "0x0000000000000000000000000000000000001000",
                "stateReceiverContract": "0x0000000000000000000000000000000000001001",
                "londonBlock": 23850000,
                "shanghaiBlock": 50523000,
                "cancunBlock": 54876000,
                "pragueBlock": 73440256,
                "jaipurBlock": 23850000,
                "delhiBlock": 38189056,
                "indoreBlock": 44934656,
                "ahmedabadBlock": 62278656,
                "bhilaiBlock": 73440256,
                "rioBlock": 77414656,
                "madhugiriBlock": 80084800,
                "madhugiriProBlock": 80084800,
                "dandeliBlock": 81424000,
                "lisovoBlock": 83756500,
                "lisovoProBlock": 83756500,
                "giuglianoBlock": 90000000
            }"#,
        )
        .expect("valid config")
    }

    #[test]
    fn gather_polygon_forks_mainnet() {
        let config = mainnet_bor_config();
        let forks = gather_polygon_forks(&config);

        // Should be sorted and deduplicated
        // London=23850000 coincides with Jaipur, Prague=73440256 with Bhilai (both deduped)
        assert_eq!(
            forks,
            vec![
                23_850_000, // London + Jaipur (deduplicated)
                38_189_056, // Delhi
                44_934_656, // Indore
                50_523_000, // Shanghai
                54_876_000, // Cancun
                62_278_656, // Ahmedabad
                73_440_256, // Prague + Bhilai (deduplicated)
                77_414_656, // Rio
                80_084_800, // Madhugiri + MadhugiriPro (deduplicated)
                81_424_000, // Dandeli
                83_756_500, // Lisovo (LisovoPro coincides, but excluded from fork ID)
            ]
        );
    }

    #[test]
    fn gather_polygon_forks_empty_config() {
        let config: BorConfig = serde_json::from_str(
            r#"{
                "period": {"0": 2},
                "producerDelay": {"0": 6},
                "sprint": {"0": 64},
                "backupMultiplier": {"0": 2},
                "validatorContract": "0x0000000000000000000000000000000000001000",
                "stateReceiverContract": "0x0000000000000000000000000000000000001001"
            }"#,
        )
        .expect("valid config");

        let forks = gather_polygon_forks(&config);
        assert!(forks.is_empty());
    }

    #[test]
    fn polygon_fork_id_before_any_fork() {
        let config = mainnet_bor_config();
        // Use a dummy genesis hash for testing
        let genesis_hash =
            H256::from_str("0xa9c28ce2141b56c474f1dc504bee9b01eb1bd7d1a507580d5519d4437a97de1b")
                .unwrap();

        let fork_id = polygon_fork_id(genesis_hash, &config, 0);
        // Before any Polygon fork, fork_next should be Jaipur block
        assert_eq!(fork_id.fork_next, 23_850_000);
        // fork_hash is just CRC32 of genesis hash (no forks XORed in yet)
        let mut hasher = Hasher::new();
        hasher.update(genesis_hash.as_bytes());
        let expected_hash = H32::from_slice(&hasher.finalize().to_be_bytes());
        assert_eq!(fork_id.fork_hash, expected_hash);
    }

    #[test]
    fn polygon_fork_id_after_jaipur() {
        let config = mainnet_bor_config();
        let genesis_hash =
            H256::from_str("0xa9c28ce2141b56c474f1dc504bee9b01eb1bd7d1a507580d5519d4437a97de1b")
                .unwrap();

        let fork_id = polygon_fork_id(genesis_hash, &config, 23_850_000);
        // After Jaipur, fork_next should be Delhi
        assert_eq!(fork_id.fork_next, 38_189_056);
        // fork_hash includes Jaipur
        let mut hasher = Hasher::new();
        hasher.update(genesis_hash.as_bytes());
        hasher.update(&23_850_000u64.to_be_bytes());
        let expected_hash = H32::from_slice(&hasher.finalize().to_be_bytes());
        assert_eq!(fork_id.fork_hash, expected_hash);
    }

    #[test]
    fn polygon_fork_id_after_all_forks() {
        let config = mainnet_bor_config();
        let genesis_hash =
            H256::from_str("0xa9c28ce2141b56c474f1dc504bee9b01eb1bd7d1a507580d5519d4437a97de1b")
                .unwrap();

        let fork_id = polygon_fork_id(genesis_hash, &config, 100_000_000);
        // All forks passed, fork_next should be 0
        assert_eq!(fork_id.fork_next, 0);
    }

    #[test]
    fn polygon_fork_id_consistency() {
        let config = mainnet_bor_config();
        let genesis_hash =
            H256::from_str("0xa9c28ce2141b56c474f1dc504bee9b01eb1bd7d1a507580d5519d4437a97de1b")
                .unwrap();

        // Fork ID should be identical at blocks right before and at a fork boundary
        let before_delhi = polygon_fork_id(genesis_hash, &config, 38_189_055);
        let at_delhi = polygon_fork_id(genesis_hash, &config, 38_189_056);

        // Before Delhi, we're in the Jaipur era - fork_next = Delhi
        assert_eq!(before_delhi.fork_next, 38_189_056);
        // At Delhi, we're in the Delhi era - fork_next = Indore
        assert_eq!(at_delhi.fork_next, 44_934_656);
        // Hash changes at boundary
        assert_ne!(before_delhi.fork_hash, at_delhi.fork_hash);
    }

    #[test]
    fn polygon_min_td() {
        assert_eq!(polygon_min_total_difficulty(0), 0);
        assert_eq!(polygon_min_total_difficulty(50_000_000), 50_000_000);
    }
}
