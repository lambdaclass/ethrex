/// Test ProgramOutput encoding against real prover fixture data.
/// These tests verify that ProgramOutput.encode() produces byte-exact
/// matches with what the SP1 prover committed as public values.
mod fixture_types;

use fixture_types::{
    fixture_to_program_output, hex_to_bytes, load_all_fixtures, load_fixture, sha256_hex,
};

#[test]
fn encode_matches_prover_output_batch8_empty() {
    let f = load_fixture("zk-dex", "batch_8_empty.json");
    let output = fixture_to_program_output(&f);
    let encoded = output.encode();

    let expected = hex_to_bytes(&f.prover.encoded_public_values);
    assert_eq!(
        encoded, expected,
        "batch {}: ProgramOutput.encode() mismatch",
        f.batch_number
    );
    assert_eq!(
        sha256_hex(&encoded),
        f.prover.sha256_public_values,
        "batch {}: sha256 mismatch",
        f.batch_number
    );
}

#[test]
fn encode_matches_prover_output_batch11_deposit() {
    let f = load_fixture("zk-dex", "batch_11_deposit.json");
    let output = fixture_to_program_output(&f);
    let encoded = output.encode();

    let expected = hex_to_bytes(&f.prover.encoded_public_values);
    assert_eq!(
        encoded, expected,
        "batch {}: ProgramOutput.encode() mismatch",
        f.batch_number
    );
    assert_eq!(
        sha256_hex(&encoded),
        f.prover.sha256_public_values,
        "batch {}: sha256 mismatch",
        f.batch_number
    );
}

#[test]
fn encode_matches_prover_output_batch12_withdrawal() {
    let f = load_fixture("zk-dex", "batch_12_withdrawal.json");
    let output = fixture_to_program_output(&f);
    let encoded = output.encode();

    let expected = hex_to_bytes(&f.prover.encoded_public_values);
    assert_eq!(
        encoded, expected,
        "batch {}: ProgramOutput.encode() mismatch",
        f.batch_number
    );
    assert_eq!(
        sha256_hex(&encoded),
        f.prover.sha256_public_values,
        "batch {}: sha256 mismatch",
        f.batch_number
    );
}

/// Parameterized: test ALL fixtures in zk-dex directory.
#[test]
fn encode_matches_all_zk_dex_fixtures() {
    let fixtures = load_all_fixtures("zk-dex");
    assert!(!fixtures.is_empty(), "No zk-dex fixtures found");

    for f in &fixtures {
        let output = fixture_to_program_output(f);
        let encoded = output.encode();
        let expected = hex_to_bytes(&f.prover.encoded_public_values);
        assert_eq!(
            encoded, expected,
            "batch {} ({}): ProgramOutput.encode() mismatch",
            f.batch_number, f.description
        );
        assert_eq!(
            sha256_hex(&encoded),
            f.prover.sha256_public_values,
            "batch {} ({}): sha256 mismatch",
            f.batch_number, f.description
        );
    }
}
