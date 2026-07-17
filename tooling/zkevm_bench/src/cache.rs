use std::io::Read;

use ethrex_common::NativeCrypto;
use ethrex_common::types::block_execution_witness::{RpcExecutionWitness, decode_witness_headers};
use ethrex_common::types::{Block, ChainConfig};
use ethrex_config::networks::Network;
use ethrex_guest_program::input::ProgramInput;
use serde::{Deserialize, Serialize};

/// Local mirror of ethrex-replay's `Cache` JSON. We only read the fields we
/// need; L2/custom optional fields (`chain_config`, blob fields) are ignored.
///
/// Also `Serialize`d by `generate.rs` (the ethrex-native stress-fixture
/// generator), which writes fixtures in this exact same shape so `load_cache`
/// below can read them back unmodified.
#[derive(Serialize, Deserialize)]
pub struct Cache {
    pub blocks: Vec<Block>,
    pub witness: RpcExecutionWitness,
    pub network: Network,
    #[serde(default)]
    pub chain_config: Option<ChainConfig>,
}

pub fn load_cache(path: &str) -> eyre::Result<Cache> {
    let bytes = std::fs::read(path)?;
    let json = if path.ends_with(".gz") {
        let mut d = flate2::read::GzDecoder::new(&bytes[..]);
        let mut s = Vec::new();
        d.read_to_end(&mut s)?;
        s
    } else {
        bytes
    };
    Ok(serde_json::from_slice(&json)?)
}

pub fn cache_to_program_input(cache: Cache) -> eyre::Result<ProgramInput> {
    let chain_config = match cache.chain_config {
        Some(cfg) => cfg,
        None => {
            cache
                .network
                .get_genesis()
                .map_err(|e| eyre::eyre!("genesis for network: {e}"))?
                .config
        }
    };
    let first_block_number = cache
        .blocks
        .iter()
        .map(|b| b.header.number)
        .min()
        .ok_or_else(|| eyre::eyre!("cache has no blocks"))?;
    let decoded_headers = decode_witness_headers(&cache.witness.headers)
        .map_err(|e| eyre::eyre!("decode witness headers: {e:?}"))?;
    let execution_witness = cache
        .witness
        .into_execution_witness(
            chain_config,
            first_block_number,
            &decoded_headers,
            &NativeCrypto,
        )
        .map_err(|e| eyre::eyre!("into_execution_witness: {e:?}"))?;
    Ok(ProgramInput::new(cache.blocks, execution_witness))
}

#[cfg(test)]
mod tests {
    use super::*;

    // Points at the in-repo orphaned cache; adjust the relative path if the
    // test working dir differs (cargo runs tests from the crate root).
    const HOODI: &str = "../../fixtures/cache/rpc_prover/cache_hoodi_1265656.json";

    #[test]
    fn loads_cache_and_builds_program_input() {
        let cache = load_cache(HOODI).expect("cache should parse");
        assert!(!cache.blocks.is_empty());
        let first = cache.blocks[0].header.number;
        let input = cache_to_program_input(cache).expect("should build program input");
        // The ProgramInput carries the same blocks.
        assert_eq!(input.blocks[0].header.number, first);
    }
}
