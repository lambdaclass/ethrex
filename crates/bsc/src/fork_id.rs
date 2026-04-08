use crc32fast::Hasher;
use ethereum_types::H32;
use ethrex_common::types::{BlockHash, BlockNumber, ChainConfig, ForkId};

/// Gathers all fork block numbers and timestamps that contribute to the BSC
/// fork ID computation per EIP-2124.
///
/// BSC uses the standard EVM fork schedule from `ChainConfig` (Homestead,
/// EIP-150, Byzantium, etc.) plus timestamp-based forks (Shanghai, Cancun,
/// Prague). BSC-specific consensus forks (Lorentz, Maxwell, Fermi) are
/// tracked in `ParliaConfig` and do NOT appear in the EIP-2124 fork ID,
/// mirroring how BSC's geth client uses `gatherForks()` on the outer
/// `ChainConfig` struct only.
///
/// Returns `(block_number_forks, timestamp_forks)` — sorted, deduplicated,
/// non-zero values.
pub fn gather_bsc_forks(
    chain_config: &ChainConfig,
    genesis_timestamp: u64,
) -> (Vec<u64>, Vec<u64>) {
    let mut block_forks: Vec<u64> = [
        chain_config.homestead_block,
        chain_config.eip150_block,
        chain_config.eip155_block,
        chain_config.eip158_block,
        chain_config.byzantium_block,
        chain_config.constantinople_block,
        chain_config.petersburg_block,
        chain_config.istanbul_block,
        chain_config.muir_glacier_block,
        chain_config.berlin_block,
        chain_config.london_block,
        chain_config.arrow_glacier_block,
        chain_config.gray_glacier_block,
        chain_config.merge_netsplit_block,
    ]
    .into_iter()
    .flatten()
    .filter(|&b| b != 0)
    .collect();

    block_forks.sort();
    block_forks.dedup();

    let mut time_forks: Vec<u64> = [
        chain_config.shanghai_time,
        chain_config.cancun_time,
        chain_config.prague_time,
    ]
    .into_iter()
    .flatten()
    .filter(|&t| t > genesis_timestamp)
    .collect();

    time_forks.sort();
    time_forks.dedup();

    (block_forks, time_forks)
}

/// Computes the BSC fork ID per EIP-2124.
///
/// Uses the BSC genesis hash and the standard EVM fork schedule from
/// `ChainConfig`. BSC-specific Parlia forks (Lorentz, Maxwell, Fermi) do
/// not contribute to the fork ID.
pub fn bsc_fork_id(
    genesis_hash: BlockHash,
    config: &ChainConfig,
    genesis_timestamp: u64,
    head_block_number: BlockNumber,
    head_timestamp: u64,
) -> ForkId {
    let (block_forks, time_forks) = gather_bsc_forks(config, genesis_timestamp);

    let mut hasher = Hasher::new();
    hasher.update(genesis_hash.as_bytes());

    // Apply block-number-based forks.
    let mut last_included = 0u64;
    for &activation in &block_forks {
        if activation <= head_block_number {
            if activation != last_included {
                hasher.update(&activation.to_be_bytes());
                last_included = activation;
            }
        } else {
            let fork_hash = H32::from_slice(&hasher.finalize().to_be_bytes());
            return ForkId {
                fork_hash,
                fork_next: activation,
            };
        }
    }

    // Apply timestamp-based forks.
    let mut last_included = 0u64;
    for &activation in &time_forks {
        if activation <= head_timestamp {
            if activation != last_included {
                hasher.update(&activation.to_be_bytes());
                last_included = activation;
            }
        } else {
            let fork_hash = H32::from_slice(&hasher.finalize().to_be_bytes());
            return ForkId {
                fork_hash,
                fork_next: activation,
            };
        }
    }

    let fork_hash = H32::from_slice(&hasher.finalize().to_be_bytes());
    ForkId {
        fork_hash,
        fork_next: 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ethrex_common::H256;

    #[test]
    fn bsc_fork_id_all_forks_at_zero() {
        // When all EVM forks are at block 0 and timestamp 0 (as in BSC mainnet
        // config), the fork ID is just CRC32 of the genesis hash with fork_next=0.
        let config = crate::genesis::bsc_mainnet_chain_config();
        let genesis_hash = H256::from_low_u64_be(1);
        let fork_id = bsc_fork_id(genesis_hash, &config, 0, 100, 1_000_000);

        // With all forks at 0, no fork transitions occur — only genesis hash CRC32.
        let mut hasher = crc32fast::Hasher::new();
        hasher.update(genesis_hash.as_bytes());
        let expected_hash = H32::from_slice(&hasher.finalize().to_be_bytes());

        assert_eq!(fork_id.fork_hash, expected_hash);
        assert_eq!(fork_id.fork_next, 0);
    }
}
