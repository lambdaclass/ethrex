//! Tests of the t8n over an input that the execution-specs filler sent, with
//! what the EELS t8n answers to it as the expected output. `testdata/README.md`
//! says where the files come from.
//!
//! The input is block 3 of a chain. Its only transaction calls a contract
//! that stores `BLOCKHASH(1)`, so the witness needs the headers of blocks 1
//! and 2.

use std::path::PathBuf;

use bytes::Bytes;
use clap::Parser;
use ethrex_common::types::{BlockHeader, ChainConfig};
use ethrex_common::{H256, U256};
use ethrex_rlp::decode::RLPDecode;
use serde_json::Value;

use super::output::{opcode_counts_json, result_json};
use super::types::{Inputs, read_inputs};
use super::witness::WitnessOutput;
use super::{Execution, T8nArgs, execute, fork, run};

#[derive(Parser)]
struct Cli {
    #[command(flatten)]
    args: T8nArgs,
}

fn testdata(file: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("testdata/blockhash_two_blocks_back")
        .join(file)
}

fn read_json(file: &str) -> Value {
    let raw = std::fs::read_to_string(testdata(file)).expect("testdata file");
    serde_json::from_str(&raw).expect("testdata JSON")
}

/// The arguments of a run over the test input that writes under `basedir`.
fn args_writing_to(basedir: &str, fork: &str, extra: &[&str]) -> T8nArgs {
    let mut argv = vec![
        "t8n".to_string(),
        format!("--input.alloc={}", testdata("alloc.json").display()),
        format!("--input.env={}", testdata("env.json").display()),
        format!("--input.txs={}", testdata("txs.json").display()),
        format!("--output.basedir={basedir}"),
        format!("--state.fork={fork}"),
    ];
    argv.extend(extra.iter().map(|arg| arg.to_string()));
    Cli::parse_from(argv).args
}

/// The arguments of a run that is only executed, so it writes nothing.
fn args(fork: &str, extra: &[&str]) -> T8nArgs {
    args_writing_to("unused", fork, extra)
}

fn amsterdam_inputs() -> (T8nArgs, Inputs) {
    let args = args("Amsterdam", &[]);
    let inputs = read_inputs(&args).expect("inputs");
    (args, inputs)
}

fn amsterdam_config() -> ChainConfig {
    fork::chain_config_for(fork::parse_fork("Amsterdam").expect("fork"), 1)
}

/// The error of an execution that has to fail.
fn execute_error(args: &T8nArgs, inputs: &Inputs) -> String {
    match execute(args, inputs) {
        Ok(_) => panic!("the t8n accepted the input"),
        Err(error) => error.to_string(),
    }
}

/// The witness EELS generates for the input.
fn eels_witness() -> WitnessOutput {
    let witness = read_json("eels_witness.json");
    let list = |name: &str| -> Vec<Bytes> {
        witness[name]
            .as_array()
            .expect("witness list")
            .iter()
            .map(|item| {
                let item = item.as_str().expect("hex string");
                Bytes::from(hex::decode(item.trim_start_matches("0x")).expect("hex"))
            })
            .collect()
    };
    WitnessOutput {
        state: list("state"),
        codes: list("codes"),
        headers: list("headers"),
    }
}

fn header_numbers(witness: &WitnessOutput) -> Vec<u64> {
    witness
        .headers
        .iter()
        .map(|header| BlockHeader::decode(header).expect("header").number)
        .collect()
}

#[test]
fn witness_is_the_one_eels_generates() {
    let (args, inputs) = amsterdam_inputs();
    let execution = execute(&args, &inputs).expect("execution");

    let witness = execution.witness.expect("witness");
    assert_eq!(witness.witness, eels_witness());
    assert!(witness.successful_validation);
}

#[test]
fn witness_lists_are_ordered() {
    let (args, inputs) = amsterdam_inputs();
    let witness = execute(&args, &inputs)
        .expect("execution")
        .witness
        .expect("witness")
        .witness;

    assert!(witness.state.len() > 1 && witness.codes.len() > 1);
    assert!(witness.state.is_sorted());
    assert!(witness.codes.is_sorted());
    assert_eq!(header_numbers(&witness), vec![1, 2]);
}

#[test]
fn result_has_the_witness_and_the_validation() {
    let (args, inputs) = amsterdam_inputs();
    let result = result_json(&execute(&args, &inputs).expect("execution"));

    assert_eq!(result["executionWitness"], read_json("eels_witness.json"));
    assert_eq!(result["successfulValidation"], Value::Bool(true));
}

/// Every node, code and header in the witness is there because the block
/// needs it, so the validation has to fail when any one of them is missing.
#[test]
fn validation_fails_without_any_one_item_of_the_witness() {
    let (args, inputs) = amsterdam_inputs();
    let execution = execute(&args, &inputs).expect("execution");
    let block = &execution.result.payload;
    let witness = execution.witness.expect("witness").witness;
    let config = amsterdam_config();
    witness
        .validate(block, config)
        .expect("the whole witness validates");

    for index in 0..witness.state.len() {
        let mut without = witness.clone();
        without.state.remove(index);
        assert!(
            without.validate(block, config).is_err(),
            "validated without state node {index}"
        );
    }
    for index in 0..witness.codes.len() {
        let mut without = witness.clone();
        without.codes.remove(index);
        assert!(
            without.validate(block, config).is_err(),
            "validated without code {index}"
        );
    }
    for index in 0..witness.headers.len() {
        let mut without = witness.clone();
        without.headers.remove(index);
        assert!(
            without.validate(block, config).is_err(),
            "validated without header {index}"
        );
    }
}

#[test]
fn validation_fails_for_another_block() {
    let (args, inputs) = amsterdam_inputs();
    let execution = execute(&args, &inputs).expect("execution");
    let witness = execution.witness.expect("witness").witness;

    let mut block = execution.result.payload.clone();
    block.header.gas_used += 1;
    assert!(witness.validate(&block, amsterdam_config()).is_err());
}

fn assert_no_witness(execution: &Execution) {
    assert!(execution.witness.is_none());
    let result = result_json(execution);
    assert!(result.get("executionWitness").is_none());
    assert!(result.get("successfulValidation").is_none());
}

#[test]
fn no_witness_without_block_headers() {
    let (args, mut inputs) = amsterdam_inputs();
    let with_headers = execute(&args, &inputs).expect("execution");

    inputs.env.block_headers.clear();
    let without_headers = execute(&args, &inputs).expect("execution");

    assert_no_witness(&without_headers);
    // The headers only add the witness: the block doesn't change.
    assert_eq!(
        without_headers.result.payload.hash(),
        with_headers.result.payload.hash()
    );
}

#[test]
fn no_witness_before_amsterdam() {
    let args = args("Osaka", &[]);
    let inputs = read_inputs(&args).expect("inputs");
    assert!(!inputs.env.block_headers.is_empty());

    assert_no_witness(&execute(&args, &inputs).expect("execution"));
}

#[test]
fn no_witness_for_state_tests() {
    let args = args("Amsterdam", &["--state-test"]);
    let inputs = read_inputs(&args).expect("inputs");

    assert_no_witness(&execute(&args, &inputs).expect("execution"));
}

#[test]
fn fails_if_a_header_does_not_have_its_hash() {
    let (args, mut inputs) = amsterdam_inputs();
    let other_hash = H256::repeat_byte(0x11);
    inputs
        .env
        .block_hashes
        .insert("0x01".to_string(), other_hash);

    let error = execute_error(&args, &inputs);
    assert!(
        error.contains("blockHeaders[1] hash mismatch"),
        "unexpected error: {error}"
    );
}

#[test]
fn fails_if_a_header_is_of_another_block() {
    let (args, mut inputs) = amsterdam_inputs();
    let parent = inputs.env.block_headers["0x02"].clone();
    inputs.env.block_headers.insert("0x01".to_string(), parent);

    let error = execute_error(&args, &inputs);
    assert!(
        error.contains("blockHeaders[1] decodes to header number 2"),
        "unexpected error: {error}"
    );
}

#[test]
fn fails_without_the_parent_header() {
    let (args, mut inputs) = amsterdam_inputs();
    inputs.env.block_headers.remove("0x02");

    let error = execute_error(&args, &inputs);
    assert!(
        error.contains("no header for the parent of block 3"),
        "unexpected error: {error}"
    );
}

#[test]
fn fails_if_the_parent_does_not_commit_to_the_alloc() {
    let (args, mut inputs) = amsterdam_inputs();
    let account = inputs.alloc.values_mut().next().expect("an account");
    account.balance += U256::one();

    let error = execute_error(&args, &inputs);
    assert!(
        error.contains("but the root of the pre-state alloc is"),
        "unexpected error: {error}"
    );
}

#[test]
fn counts_the_opcodes_like_eels() {
    let args = args("Amsterdam", &["--opcode.count", "opcodes.json"]);
    let inputs = read_inputs(&args).expect("inputs");
    let execution = execute(&args, &inputs).expect("execution");

    let counts = execution.opcode_counts.expect("opcode counts");
    assert_eq!(
        opcode_counts_json(&counts),
        read_json("eels_opcode_counts.json")
    );
}

#[test]
fn does_not_count_opcodes_without_the_flag() {
    let (args, inputs) = amsterdam_inputs();
    assert!(
        execute(&args, &inputs)
            .expect("execution")
            .opcode_counts
            .is_none()
    );

    // Geth takes an empty file name as the flag not being set.
    let args = self::args("Amsterdam", &["--opcode.count", ""]);
    assert!(
        execute(&args, &inputs)
            .expect("execution")
            .opcode_counts
            .is_none()
    );
}

#[test]
fn writes_the_result_and_the_opcode_counts() {
    let basedir = tempfile::tempdir().expect("temp dir");
    let args = args_writing_to(
        &basedir.path().display().to_string(),
        "Amsterdam",
        &[
            "--output.result=output/result.json",
            "--opcode.count",
            "opcodes.json",
        ],
    );
    run(args).expect("t8n run");

    let read = |file: &str| -> Value {
        let raw = std::fs::read_to_string(basedir.path().join(file)).expect("output file");
        serde_json::from_str(&raw).expect("output JSON")
    };
    let result = read("output/result.json");
    assert_eq!(result["executionWitness"], read_json("eels_witness.json"));
    assert_eq!(result["successfulValidation"], Value::Bool(true));
    assert_eq!(read("opcodes.json"), read_json("eels_opcode_counts.json"));
}
