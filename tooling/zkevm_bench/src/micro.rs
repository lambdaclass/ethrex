use std::collections::BTreeMap;

use ethrex_guest_program::input::ProgramInput;
use serde::Deserialize;

#[derive(Deserialize)]
struct MicroTest {
    blocks: Vec<MicroBlock>,
}

#[derive(Deserialize)]
struct MicroBlock {
    /// The spec's schema-prefixed stateless input — exactly the bytes another
    /// client's guest would consume for this block.
    #[serde(rename = "statelessInputBytes", default)]
    stateless_input_bytes: Option<String>,
    /// Committed result the guest must reproduce; byte 32 is
    /// `successful_validation`.
    #[serde(rename = "statelessOutputBytes", default)]
    stateless_output_bytes: Option<String>,
    #[serde(rename = "expectException", default)]
    expect_exception: Option<serde_json::Value>,
}

fn decode_hex(label: &str, s: &str) -> eyre::Result<Vec<u8>> {
    hex::decode(s.trim_start_matches("0x")).map_err(|e| eyre::eyre!("{label} hex decode: {e}"))
}

/// Reads an EEST zkevm fixture and returns the block's `statelessInputBytes`
/// verbatim as the guest input.
///
/// The bytes are taken from the fixture rather than rebuilt from `rlp` plus
/// `executionWitness`. Re-deriving them host side means re-deriving every field
/// the wire format carries, and missing one is silent: an earlier version
/// passed no block access list, so on Amsterdam the header's
/// `block_access_list_hash` was absent and `validate_block_pre_execution`
/// rejected the block before the EVM ran. `run_stateless_guest` never returns
/// an error — it commits `successful_validation = 0` — and the emulator still
/// exits 0, so the run was recorded as a success whose AIR cost measured
/// witness loading and header validation instead of the workload. Consuming the
/// fixture's own bytes removes that whole class of divergence.
///
/// `case` picks the pytest id to measure. These files hold up to 18
/// parametrisations and the choice must not depend on map ordering: on an
/// experimental tag an upstream rename that sorts earlier would silently change
/// what a workload measures, and `compare` matches on workload name alone, so
/// the swap would read as a client regression. A substring is enough to be
/// stable against unrelated suffixes; ambiguity is an error rather than a
/// coin flip.
///
/// `gas` is workload metadata only (the target is baked into the fixture); it
/// is recorded in the report, not applied here.
pub fn micro_to_program_input(
    source: &str,
    case: Option<&str>,
    _gas: Option<u64>,
) -> eyre::Result<ProgramInput> {
    let raw = std::fs::read_to_string(source)?;
    let fixture: BTreeMap<String, MicroTest> = serde_json::from_str(&raw)?;

    let matches: Vec<(&String, &MicroTest)> = match case {
        Some(c) => fixture.iter().filter(|(k, _)| k.contains(c)).collect(),
        None => fixture.iter().collect(),
    };
    let (name, test) = match matches.as_slice() {
        [] => eyre::bail!(
            "no test in {source} matches {:?} (file holds {} case(s))",
            case.unwrap_or("<any>"),
            fixture.len()
        ),
        [one] => *one,
        many => eyre::bail!(
            "{:?} matches {} cases in {source}; narrow it (first: {})",
            case.unwrap_or("<any>"),
            many.len(),
            many[0].0
        ),
    };

    for block in &test.blocks {
        if block.expect_exception.is_some() {
            continue;
        }
        let Some(input_hex) = block.stateless_input_bytes.as_deref() else {
            continue;
        };
        let input = decode_hex("statelessInputBytes", input_hex)?;
        if input.is_empty() {
            continue;
        }
        // A fixture whose own expected result is a failure would benchmark the
        // rejection path, not the workload.
        if let Some(out_hex) = block.stateless_output_bytes.as_deref() {
            let out = decode_hex("statelessOutputBytes", out_hex)?;
            match out.get(32) {
                Some(1) => {}
                Some(_) => continue,
                None => eyre::bail!(
                    "statelessOutputBytes for {name} is {} bytes, too short to carry \
                     successful_validation at index 32",
                    out.len()
                ),
            }
        }
        return Ok(input);
    }
    eyre::bail!("no executable block with statelessInputBytes in {source} ({name})")
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = "../ef_tests/blockchain/vectors_zkevm/eest/for_amsterdam/amsterdam/eip8025_optional_proofs/witness_7702/witness_codes_delegation_chain.json";

    #[test]
    fn builds_program_input_from_eest_fixture() {
        // `vectors_zkevm` is gitignored (downloaded via `make zkevm-vectors`).
        if !std::path::Path::new(FIXTURE).exists() {
            eprintln!("skipping: EEST fixture absent (run `make zkevm-vectors`)");
            return;
        }
        let input = micro_to_program_input(FIXTURE, None, Some(100_000_000)).expect("convert");
        assert!(!input.is_empty(), "stateless input bytes must not be empty");
        let schema_id = u16::from_be_bytes([input[0], input[1]]);
        assert_eq!(
            schema_id,
            ethrex_common::types::stateless_ssz::STATELESS_INPUT_SCHEMA_ID
        );
    }

    #[test]
    fn ambiguous_case_selector_is_an_error() {
        if !std::path::Path::new(FIXTURE).exists() {
            return;
        }
        // "test_" prefixes every pytest id, so it can only be unambiguous when
        // the file holds exactly one case.
        let raw = std::fs::read_to_string(FIXTURE).unwrap();
        let n = serde_json::from_str::<BTreeMap<String, serde_json::Value>>(&raw)
            .unwrap()
            .len();
        let got = micro_to_program_input(FIXTURE, Some("test_"), None);
        assert_eq!(
            got.is_err(),
            n > 1,
            "ambiguity must not be resolved silently"
        );
    }
}
