use std::io::Read;

use ethrex_common::types::block_execution_witness::{RpcExecutionWitness, decode_witness_headers};
use ethrex_common::types::{Block, ChainConfig};
use ethrex_guest_program::input::ProgramInput;
use serde::Deserialize;

/// Local mirror of eth-act's `witness-generator-cli` stress-fixture JSON.
/// `chain_config` is kept as a raw `serde_json::Value` because the fixture
/// emits it in snake_case while `ChainConfig`'s serde is `camelCase`; the
/// keys are rewritten before the final typed deserialization (see
/// `rewrite_keys_camel_case`).
#[derive(Deserialize)]
struct StressFixture {
    stateless_input: StatelessInput,
}

#[derive(Deserialize)]
struct StatelessInput {
    /// Kept as a raw `Value`: `Block`'s `camelCase` rename covers most
    /// fields, but `BlockBody::ommers` carries an explicit
    /// `#[serde(rename = "uncles")]` (field-level renames win over the
    /// struct's `rename_all`), while eth-act's fixture emits `"ommers"`.
    /// `fixup_block_body_ommers_key` bridges that one mismatched key
    /// before the typed deserialization.
    block: serde_json::Value,
    witness: RpcExecutionWitness,
    chain_config: serde_json::Value,
}

/// Converts a single `snake_case` (or already-`camelCase`) key to
/// `camelCase`. Keys without an underscore are returned unchanged, so
/// nested keys that are already camelCase (e.g. `baseFeeUpdateFraction`)
/// pass through untouched.
fn snake_to_camel(key: &str) -> String {
    if !key.contains('_') {
        return key.to_string();
    }
    let mut parts = key.split('_');
    let mut out = parts.next().unwrap_or_default().to_string();
    for part in parts {
        let mut chars = part.chars();
        if let Some(first) = chars.next() {
            out.push(first.to_ascii_uppercase());
            out.push_str(chars.as_str());
        }
    }
    out
}

/// Recursively rewrites every object key in a `serde_json::Value` from
/// snake_case to camelCase, leaving array elements and scalar values
/// untouched.
fn rewrite_keys_camel_case(value: serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Object(map) => {
            let mut rewritten = serde_json::Map::with_capacity(map.len());
            for (k, v) in map {
                rewritten.insert(snake_to_camel(&k), rewrite_keys_camel_case(v));
            }
            serde_json::Value::Object(rewritten)
        }
        serde_json::Value::Array(items) => {
            serde_json::Value::Array(items.into_iter().map(rewrite_keys_camel_case).collect())
        }
        other => other,
    }
}

/// eth-act's benchmark test chains carry no deposit contract and emit
/// `"deposit_contract_address": null` (→ `depositContractAddress` after the
/// camelCase rewrite). `ChainConfig::deposit_contract_address` is a
/// non-optional `Address` with no serde default, so a JSON `null` fails to
/// deserialize. Coalesce that one specific null to the zero address ("none
/// configured") before typed deserialization. Scoped to this key only — no
/// blanket null-coalescing — and applied to the already-rewritten
/// (camelCase) object.
fn coalesce_null_deposit_contract_address(
    mut chain_config: serde_json::Value,
) -> serde_json::Value {
    const ZERO_ADDRESS: &str = "0x0000000000000000000000000000000000000000";
    if let Some(obj) = chain_config.as_object_mut()
        && obj.get("depositContractAddress") == Some(&serde_json::Value::Null)
    {
        obj.insert(
            "depositContractAddress".to_string(),
            serde_json::Value::String(ZERO_ADDRESS.to_string()),
        );
    }
    chain_config
}

/// `ethrex_common::types::BlockBody::ommers` deserializes under the JSON key
/// `"uncles"` (an explicit field-level rename, which overrides `Block`'s
/// otherwise-`camelCase` struct rename). eth-act's fixture emits the field
/// under its Rust name, `"ommers"`. Rename that one key in place; if
/// `"uncles"` is already present, leave it untouched.
fn fixup_block_body_ommers_key(mut block: serde_json::Value) -> serde_json::Value {
    if let Some(body) = block.get_mut("body").and_then(|b| b.as_object_mut())
        && let Some(ommers) = body.remove("ommers")
    {
        body.entry("uncles".to_string()).or_insert(ommers);
    }
    block
}

/// Reads an eth-act `witness-generator-cli` stress fixture (plain `.json` or
/// gzipped `.json.gz`, mirroring `cache.rs::load_cache`'s handling) and
/// converts it into a single-block `ProgramInput`.
pub fn stress_to_program_input(path: &str) -> eyre::Result<ProgramInput> {
    let bytes = std::fs::read(path)?;
    let json = if path.ends_with(".gz") {
        let mut d = flate2::read::GzDecoder::new(&bytes[..]);
        let mut s = Vec::new();
        d.read_to_end(&mut s)?;
        s
    } else {
        bytes
    };
    let fixture: StressFixture = serde_json::from_slice(&json)?;
    let StatelessInput {
        block,
        witness,
        chain_config,
    } = fixture.stateless_input;

    let block = fixup_block_body_ommers_key(block);
    let block: Block = serde_json::from_value(block)
        .map_err(|e| eyre::eyre!("parse block (after ommers->uncles body-key fixup): {e}"))?;

    let chain_config = rewrite_keys_camel_case(chain_config);
    let chain_config = coalesce_null_deposit_contract_address(chain_config);
    let chain_config: ChainConfig = serde_json::from_value(chain_config)
        .map_err(|e| eyre::eyre!("parse chain_config after snake->camel rewrite: {e}"))?;

    let decoded_headers = decode_witness_headers(&witness.headers)
        .map_err(|e| eyre::eyre!("decode witness headers: {e:?}"))?;
    let ew = witness
        .into_execution_witness(
            chain_config,
            block.header.number,
            &decoded_headers,
            &ethrex_common::NativeCrypto,
        )
        .map_err(|e| eyre::eyre!("into_execution_witness: {e:?}"))?;
    Ok(ProgramInput::new(vec![block], ew))
}

#[cfg(test)]
mod tests {
    use super::*;

    const STRESS_FIXTURE: &str = "fixtures/stress/eest_jumpdest_analysis_150M.json.gz";

    #[test]
    fn loads_stress_fixture() {
        let input = stress_to_program_input(STRESS_FIXTURE).unwrap();
        #[allow(unused)]
        let _ = &input;
        assert!(!input.blocks.is_empty());
    }

    /// Proves the snake_case->camelCase key rewrite actually maps real
    /// fixture values into `ChainConfig`, rather than silently falling back
    /// to `Option::None`/`Default` for every field. `chain_id` and
    /// `cancun_time` are both present with concrete (non-`None`) values in
    /// the committed sample's `chain_config` (verified by inspecting the
    /// decompressed fixture). Only the null `depositContractAddress` is
    /// coalesced (to the zero address); every asserted field is a genuine
    /// fixture value carried through the rewrite.
    #[test]
    fn chain_config_rewrite_loads_real_fields() {
        let bytes = std::fs::read(STRESS_FIXTURE).unwrap();
        let mut decoder = flate2::read::GzDecoder::new(&bytes[..]);
        let mut raw = Vec::new();
        decoder.read_to_end(&mut raw).unwrap();
        let fixture: StressFixture = serde_json::from_slice(&raw).unwrap();

        let rewritten = rewrite_keys_camel_case(fixture.stateless_input.chain_config);
        let rewritten = coalesce_null_deposit_contract_address(rewritten);
        let chain_config: ChainConfig = serde_json::from_value(rewritten).unwrap();

        // Real fixture values proving the rewrite maps keys, not defaults.
        assert_eq!(chain_config.chain_id, 1);
        assert_eq!(chain_config.cancun_time, Some(0));
        // The one coalesced field is the zero address, not garbage.
        assert_eq!(
            chain_config.deposit_contract_address,
            ethrex_common::Address::zero()
        );
    }
}
