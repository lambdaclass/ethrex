/// Test ProgramOutput encoding against real prover fixture data.
/// These tests verify that ProgramOutput.encode() produces byte-exact
/// matches with what the SP1 prover committed as public values.
mod fixture_types;

use fixture_types::{
    discover_all_apps, fixture_to_program_output, hex_to_bytes, load_all_fixtures, sha256_hex,
};

/// Auto-discovery: test ALL fixtures across ALL apps.
#[test]
fn encode_matches_all_app_fixtures() {
    let apps = discover_all_apps();
    assert!(!apps.is_empty(), "No fixture apps found in tests/fixtures/");

    for app in &apps {
        let fixtures = load_all_fixtures(app);
        for f in &fixtures {
            let output = fixture_to_program_output(f);
            let encoded = output.encode();
            let expected = hex_to_bytes(&f.prover.encoded_public_values);
            assert_eq!(
                encoded, expected,
                "[{app}] batch {} ({}): ProgramOutput.encode() mismatch",
                f.batch_number, f.description
            );
            assert_eq!(
                sha256_hex(&encoded),
                f.prover.sha256_public_values,
                "[{app}] batch {} ({}): sha256 mismatch",
                f.batch_number, f.description
            );
        }
    }
}
