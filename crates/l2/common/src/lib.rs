pub mod calldata;
pub mod merkle_tree;
pub mod messages;
pub mod privileged_transactions;
pub mod prover;
pub mod sequencer_state;
pub mod utils;

/// Maps a guest program ID string to its on-chain `programTypeId`.
///
/// This is the single source of truth for program type IDs used by
/// the deployer, committer, and proof sender.
///
/// Official programs have fixed type IDs (1-9).
/// Returns 0 for unknown programs — the deployer should use
/// `registerProgram()` (community, auto-assign typeId 10+) for these.
pub fn resolve_program_type_id(program_id: &str) -> u8 {
    match program_id {
        "evm-l2" => 1,
        "zk-dex" => 2,
        "tokamon" => 3,
        _ => 0,
    }
}

/// Parses a program spec that may include an explicit type ID.
///
/// Supported formats:
///   - `"my-app"` → (programId="my-app", typeId=None) — use registerProgram (auto-assign)
///   - `"my-app:10"` → (programId="my-app", typeId=Some(10)) — use registerOfficialProgram
///   - `"zk-dex"` → resolved via `resolve_program_type_id` as before
pub fn parse_program_spec(spec: &str) -> (String, Option<u8>) {
    // Check for explicit "programId:typeId" format
    if let Some((id, type_str)) = spec.split_once(':') {
        if let Ok(type_id) = type_str.parse::<u8>() {
            if type_id >= 2 {
                // Valid explicit typeId (2-255)
                return (id.to_string(), Some(type_id));
            }
            // Reserved typeId (0 or 1) — treat as community program
            return (id.to_string(), None);
        }
    }
    // No colon or non-numeric suffix — check known programs
    let known = resolve_program_type_id(spec);
    if known > 0 {
        (spec.to_string(), Some(known))
    } else {
        (spec.to_string(), None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolve_known_programs() {
        assert_eq!(resolve_program_type_id("evm-l2"), 1);
        assert_eq!(resolve_program_type_id("zk-dex"), 2);
        assert_eq!(resolve_program_type_id("tokamon"), 3);
        assert_eq!(resolve_program_type_id("unknown"), 0);
    }

    #[test]
    fn test_parse_known_program() {
        assert_eq!(parse_program_spec("zk-dex"), ("zk-dex".into(), Some(2)));
        assert_eq!(parse_program_spec("evm-l2"), ("evm-l2".into(), Some(1)));
    }

    #[test]
    fn test_parse_community_program() {
        assert_eq!(parse_program_spec("my-app"), ("my-app".into(), None));
    }

    #[test]
    fn test_parse_explicit_type_id() {
        assert_eq!(parse_program_spec("my-app:10"), ("my-app".into(), Some(10)));
        assert_eq!(parse_program_spec("custom:5"), ("custom".into(), Some(5)));
    }

    #[test]
    fn test_parse_rejects_reserved_type_ids() {
        // typeId 0 and 1 are reserved — should fall through to community (None)
        assert_eq!(parse_program_spec("bad:0"), ("bad".into(), None));
        assert_eq!(parse_program_spec("bad:1"), ("bad".into(), None));
    }

    #[test]
    fn test_parse_invalid_type_id() {
        // Non-numeric suffix — treated as unknown community program
        assert_eq!(parse_program_spec("my-app:abc"), ("my-app:abc".into(), None));
    }
}
