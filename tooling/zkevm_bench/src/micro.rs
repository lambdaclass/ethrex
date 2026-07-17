use std::collections::BTreeMap;

use bytes::Bytes;
use ef_tests_blockchain::fork::Fork;
use ethrex_common::NativeCrypto;
use ethrex_common::types::Block;
use ethrex_common::types::block_execution_witness::{RpcExecutionWitness, decode_witness_headers};
use ethrex_guest_program::input::ProgramInput;
use ethrex_rlp::decode::RLPDecode;
use serde::Deserialize;

#[derive(Deserialize)]
struct MicroTest {
    network: Fork,
    blocks: Vec<MicroBlock>,
}

#[derive(Deserialize)]
struct MicroBlock {
    #[serde(with = "ethrex_common::serde_utils::bytes")]
    rlp: Bytes,
    #[serde(rename = "executionWitness", default)]
    execution_witness: Option<RpcExecutionWitness>,
    #[serde(rename = "expectException", default)]
    expect_exception: Option<serde_json::Value>,
}

/// Reads an EEST zkevm fixture (keyed by test name), takes the first test's
/// first executable block (no `expectException`, present `executionWitness`),
/// RLP-decodes it, converts the witness into the guest-consumable
/// `ExecutionWitness`, and returns a single-block `ProgramInput`.
///
/// `gas` is workload metadata only (the gas limit is already baked into the
/// fixture); it is recorded in the report, not applied here.
pub fn micro_to_program_input(source: &str, _gas: Option<u64>) -> eyre::Result<ProgramInput> {
    let raw = std::fs::read_to_string(source)?;
    let fixture: BTreeMap<String, MicroTest> = serde_json::from_str(&raw)?;
    let (_name, test) = fixture
        .into_iter()
        .next()
        .ok_or_else(|| eyre::eyre!("empty fixture {source}"))?;
    let chain_config = *test.network.chain_config();

    for block in test.blocks {
        if block.expect_exception.is_some() {
            continue;
        }
        let Some(rpc_witness) = block.execution_witness else {
            continue;
        };
        let core_block =
            Block::decode(&block.rlp).map_err(|e| eyre::eyre!("decode block rlp: {e:?}"))?;
        let block_number = core_block.header.number;
        let decoded_headers = decode_witness_headers(&rpc_witness.headers)
            .map_err(|e| eyre::eyre!("decode witness headers: {e:?}"))?;
        let execution_witness = rpc_witness
            .into_execution_witness(chain_config, block_number, &decoded_headers, &NativeCrypto)
            .map_err(|e| eyre::eyre!("into_execution_witness: {e:?}"))?;
        return Ok(ProgramInput::new(vec![core_block], execution_witness));
    }
    eyre::bail!("no executable block with executionWitness in {source}")
}

#[cfg(test)]
mod tests {
    use super::*;

    // Verified to exist: network Amsterdam, 1 block, has executionWitness, no expectException.
    const FIXTURE: &str = "../ef_tests/blockchain/vectors_zkevm/eest/for_amsterdam/amsterdam/eip8025_optional_proofs/witness_7702/witness_codes_delegation_chain.json";

    #[test]
    fn builds_program_input_from_eest_fixture() {
        // `vectors_zkevm` is gitignored (downloaded via `make zkevm-vectors`).
        // CI's L1 unit-test step runs this crate before any vector download, so
        // skip gracefully when the fixture is absent (mirrors the ziskemu-skip
        // pattern in tests/smoke.rs). When present, the test runs and asserts.
        if !std::path::Path::new(FIXTURE).exists() {
            eprintln!(
                "skipping: EEST fixture absent (run `make zkevm-vectors` in tooling/ef_tests/blockchain)"
            );
            return;
        }
        // If this path moves in a future fixture bump, pick any file under
        // vectors_zkevm whose first block has no expectException and has an
        // executionWitness; the schema is uniform across the set.
        let input = micro_to_program_input(FIXTURE, Some(100_000_000)).expect("convert");
        assert!(!input.blocks.is_empty());
    }
}
