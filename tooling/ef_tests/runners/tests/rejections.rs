//! Each result lists the blocks or payloads the client rejected, with the
//! client's own error, and the runner does not check that error itself.
//!
//! The fixtures are tests@v21.0.0 fixtures, some edited: `rejected_twice`
//! repeats its invalid block, `rejected_for_another_reason` names an exception
//! other than the one the block is rejected for, and `json_rpc_error` drops the
//! payload's `blockHash` and expects the resulting error code, with the genesis
//! as its last block.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::{Map, Value};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

/// The results for `fixture`, by fixture name.
fn results(bin: &str, fixture_name: &str) -> HashMap<String, Value> {
    results_at(bin, &fixture(fixture_name), false)
}

/// The results for the fixture file at `path`, by fixture name, with
/// `ETHREX_ENGINE_STRICT_EXCEPTIONS` set when `strict`.
fn results_at(bin: &str, path: &Path, strict: bool) -> HashMap<String, Value> {
    let mut command = Command::new(bin);
    command.arg("--json").arg(path);
    if strict {
        command.env("ETHREX_ENGINE_STRICT_EXCEPTIONS", "1");
    } else {
        command.env_remove("ETHREX_ENGINE_STRICT_EXCEPTIONS");
    }
    let output = command.output().expect("failed to run the binary");
    let results: Vec<Value> =
        serde_json::from_slice(&output.stdout).expect("stdout is one JSON array");
    results
        .into_iter()
        .map(|r| (r["name"].as_str().unwrap().to_string(), r))
        .collect()
}

/// `(index, error)` of each rejection in `result`, checking each has a hash
/// exactly when `hashed`.
fn rejections_in(result: &Value, hashed: bool) -> Vec<(u64, String)> {
    result["rejections"]
        .as_array()
        .expect("rejections is an array")
        .iter()
        .map(|r| {
            assert_eq!(r.get("hash").is_some(), hashed, "{r}");
            (
                r["index"].as_u64().unwrap(),
                r["error"].as_str().unwrap().to_string(),
            )
        })
        .collect()
}

fn assert_passes(result: &Value) {
    assert_eq!(result["pass"], true, "{result}");
}

/// ethrex's error for the fixtures' invalid base fee.
const BASE_FEE_ERROR: &str = "Base fee per gas is incorrect";

#[test]
fn blocktest_reports_each_rejected_block() {
    let results = results(env!("CARGO_BIN_EXE_blocktest"), "blocktest_rejections.json");

    let rejected = &results["rejected_twice"];
    assert_passes(rejected);
    let rejections = rejections_in(rejected, true);
    assert_eq!(
        rejections.iter().map(|r| r.0).collect::<Vec<_>>(),
        [0, 1],
        "{rejected}"
    );
    assert!(rejections[0].1.contains(BASE_FEE_ERROR), "{rejected}");

    // A block that does not decode has no hash, and its error is the decoder's.
    let undecodable = &results["undecodable"];
    assert_passes(undecodable);
    let rejections = rejections_in(undecodable, false);
    assert_eq!(rejections.len(), 1, "{undecodable}");
    assert_eq!(rejections[0].0, 0);
    assert!(
        rejections[0].1.starts_with("Error decoding field"),
        "{undecodable}"
    );

    let clean = &results["clean"];
    assert_passes(clean);
    assert_eq!(clean["rejections"], serde_json::json!([]));
}

#[test]
fn blocktest_does_not_check_the_rejection_reason() {
    let results = results(env!("CARGO_BIN_EXE_blocktest"), "blocktest_rejections.json");
    let wrong = &results["rejected_for_another_reason"];
    assert_passes(wrong);
    let rejections = rejections_in(wrong, true);
    assert_eq!(rejections.len(), 1, "{wrong}");
    assert!(rejections[0].1.contains(BASE_FEE_ERROR), "{wrong}");
}

#[test]
fn enginetest_reports_each_rejected_payload() {
    let results = results(
        env!("CARGO_BIN_EXE_enginetest"),
        "enginetest_rejections.json",
    );

    let rejected = &results["rejected_twice"];
    assert_passes(rejected);
    let rejections = rejections_in(rejected, true);
    assert_eq!(
        rejections.iter().map(|r| r.0).collect::<Vec<_>>(),
        [0, 1],
        "{rejected}"
    );
    assert!(rejections[0].1.contains(BASE_FEE_ERROR), "{rejected}");

    // A JSON-RPC error has no hash, and its error is `<code>: <message>`.
    let rpc_error = &results["json_rpc_error"];
    assert_passes(rpc_error);
    let rejections = rejections_in(rpc_error, false);
    assert_eq!(rejections.len(), 1, "{rpc_error}");
    assert_eq!(rejections[0].0, 0);
    assert!(rejections[0].1.starts_with("-32602: "), "{rpc_error}");

    let clean = &results["clean"];
    assert_passes(clean);
    assert_eq!(clean["rejections"], serde_json::json!([]));
}

#[test]
fn enginetest_does_not_check_the_rejection_reason() {
    let results = results(
        env!("CARGO_BIN_EXE_enginetest"),
        "enginetest_rejections.json",
    );
    let wrong = &results["rejected_for_another_reason"];
    assert_passes(wrong);
    let rejections = rejections_in(wrong, true);
    assert_eq!(rejections.len(), 1, "{wrong}");
    assert!(rejections[0].1.contains(BASE_FEE_ERROR), "{wrong}");
}

/// A copy of the fixture file `name` in which every block or payload that expects
/// to be rejected names `exception` instead.
fn renamed_copy(name: &str, exception: &str) -> PathBuf {
    let mut fixtures: Map<String, Value> =
        serde_json::from_slice(&std::fs::read(fixture(name)).unwrap()).unwrap();
    for fixture in fixtures.values_mut() {
        for (list, field) in [
            ("blocks", "expectException"),
            ("engineNewPayloads", "validationError"),
        ] {
            for entry in fixture[list].as_array_mut().into_iter().flatten() {
                if entry.get(field).is_some_and(|v| !v.is_null()) {
                    entry[field] = exception.into();
                }
            }
        }
    }
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("renamed-{exception}"));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    std::fs::write(&path, serde_json::to_vec(&fixtures).unwrap()).unwrap();
    path
}

/// The verdict and rejections of each fixture, which renaming the exceptions the
/// fixtures expect must leave alone.
fn outcomes(results: &HashMap<String, Value>) -> HashMap<String, (Value, Value)> {
    results
        .iter()
        .map(|(name, r)| (name.clone(), (r["pass"].clone(), r["rejections"].clone())))
        .collect()
}

#[test]
fn blocktest_verdicts_do_not_depend_on_the_exception_named() {
    let bin = env!("CARGO_BIN_EXE_blocktest");
    let original = results(bin, "blocktest_rejections.json");
    // Includes the block that does not decode, now expecting an execution error.
    let renamed = results_at(
        bin,
        &renamed_copy(
            "blocktest_rejections.json",
            "BlockException.INCORRECT_BLOB_GAS_USED",
        ),
        false,
    );
    assert_eq!(outcomes(&renamed), outcomes(&original));
    assert_passes(&renamed["undecodable"]);
}

#[test]
fn enginetest_verdicts_do_not_depend_on_the_exception_named() {
    let bin = env!("CARGO_BIN_EXE_enginetest");
    let original = results(bin, "enginetest_rejections.json");
    let renamed_file = renamed_copy(
        "enginetest_rejections.json",
        "BlockException.INCORRECT_BLOB_GAS_USED",
    );
    // `ETHREX_ENGINE_STRICT_EXCEPTIONS` is for the cargo engine harness only.
    for strict in [false, true] {
        let renamed = results_at(bin, &renamed_file, strict);
        assert_eq!(outcomes(&renamed), outcomes(&original), "strict={strict}");
    }
}
