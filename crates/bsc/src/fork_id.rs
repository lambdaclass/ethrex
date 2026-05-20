use crc32fast::Hasher;
use ethereum_types::H32;
use ethrex_common::types::{BlockHash, BlockNumber, ChainConfig, ForkId};

/// Gathers all fork block numbers and timestamps that contribute to the BSC
/// fork ID computation per EIP-2124.
///
/// Includes both standard EVM forks (Homestead, EIP-150, Byzantium, etc.) and
/// BSC-specific forks (Ramanujan, Niels, Kepler, Feynman, etc.) from
/// `ChainConfig`.
///
/// Returns `(block_number_forks, timestamp_forks)` — sorted, deduplicated,
/// non-zero values (block forks at 0 are excluded; timestamp forks at or
/// before genesis are excluded).
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
        // BSC-specific block-number forks
        chain_config.ramanujan_block,
        chain_config.niels_block,
        chain_config.mirror_sync_block,
        chain_config.bruno_block,
        chain_config.euler_block,
        chain_config.gibbs_block,
        chain_config.nano_block,
        chain_config.moran_block,
        chain_config.planck_block,
        chain_config.luban_block,
        chain_config.plato_block,
        chain_config.hertz_block,
        chain_config.hertzfix_block,
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
        chain_config.osaka_time,
        // BSC-specific timestamp forks
        chain_config.kepler_time,
        chain_config.feynman_time,
        chain_config.feynman_fix_time,
        chain_config.haber_time,
        chain_config.haber_fix_time,
        chain_config.bohr_time,
        chain_config.pascal_time,
        chain_config.lorentz_time,
        chain_config.maxwell_time,
        chain_config.fermi_time,
        chain_config.mendel_time,
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
/// Uses the BSC genesis hash and all fork activations from `ChainConfig`,
/// including BSC-specific block-number forks (Ramanujan through Hertzfix) and
/// timestamp forks (Kepler through Mendel).
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

    #[test]
    fn chapel_fork_id_all_forks_passed() {
        // Verify that the Chapel fork ID computed with all BSC-specific forks
        // included matches the expected value observed from Chapel bootnodes.
        //
        // Chapel genesis timestamp: 0x5e9da7ce = 1587267022
        // The last Chapel fork is mendel/osaka at timestamp 1774319400.
        // At head_block > 35682300 (hertzfixBlock) and head_timestamp > 1774319400,
        // all forks are applied and fork_next should be 0.
        let genesis = crate::genesis::bsc_chapel_genesis();
        let genesis_block = genesis.get_block();
        let genesis_hash = genesis_block.header.hash();
        let genesis_timestamp = genesis_block.header.timestamp;

        // State past all known Chapel forks.
        let head_block = 50_000_000u64;
        let head_timestamp = 1_800_000_000u64;

        let fork_id = bsc_fork_id(
            genesis_hash,
            &genesis.config,
            genesis_timestamp,
            head_block,
            head_timestamp,
        );

        // All forks are in the past, so fork_next must be 0.
        assert_eq!(
            fork_id.fork_next, 0,
            "fork_next should be 0 when past all forks"
        );

        // Verify the block-number forks are correctly parsed from the genesis.
        let (block_forks, time_forks) = gather_bsc_forks(&genesis.config, genesis_timestamp);

        // Chapel has 13 distinct non-zero block-number fork activations.
        // Sorted: 1010000, 1014369, 5582500, 13837000, 19203503, 22800220,
        //         23482428, 23603940, 28196022, 29295050, 29861024, 31103030, 35682300
        assert_eq!(
            block_forks.len(),
            13,
            "expected 13 distinct non-zero block-number forks for Chapel"
        );
        assert_eq!(
            block_forks[0], 1_010_000,
            "first block fork: ramanujanBlock"
        );
        assert_eq!(
            block_forks[12], 35_682_300,
            "last block fork: hertzfixBlock"
        );

        // Chapel has 12 distinct timestamp forks after genesis.
        // Sorted: 1702972800 (shanghai/kepler), 1710136800, 1711342800, 1713330442,
        //         1716962820, 1719986788, 1724116996, 1740452880, 1744097580,
        //         1748243100, 1762741500, 1774319400
        assert_eq!(
            time_forks.len(),
            12,
            "expected 12 distinct timestamp forks after genesis for Chapel"
        );
        assert_eq!(
            time_forks[0], 1_702_972_800,
            "first timestamp fork: shanghaiTime/keplerTime"
        );
        assert_eq!(
            time_forks[11], 1_774_319_400,
            "last timestamp fork: osakaTime/mendelTime"
        );
    }
}
