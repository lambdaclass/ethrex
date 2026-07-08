use crate::backend::BackendError;

/// AIR-cost breakdown parsed from `ziskemu`'s `COST DISTRIBUTION` report block.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ZiskAirCost {
    pub base: u64,
    pub main: u64,
    pub opcodes: u64,
    pub precompiles: u64,
    pub memory: u64,
    pub total: u64,
    pub steps: u64,
}

/// Parses a comma-grouped integer token (e.g. `"4,930,197,804"`) into a `u64`.
fn parse_cost_token(token: &str) -> Option<u64> {
    token.replace(',', "").parse::<u64>().ok()
}

/// Parses the `COST DISTRIBUTION` summary block out of raw `ziskemu` stdout.
///
/// Only the exact first whitespace-separated token of each line is matched
/// against the summary labels, and the second token (with `,` grouping
/// separators stripped) is taken as the value. This keeps the parser immune
/// to the detailed per-opcode tables that follow the summary in the real
/// output (e.g. lines starting with `OP`, `COST BY OPCODE`, or `FROPS`),
/// since their first token never matches a summary label.
pub fn parse_air_cost(stdout: &str) -> Result<ZiskAirCost, BackendError> {
    let mut air_cost = ZiskAirCost::default();
    let mut found_component = false;

    for line in stdout.lines() {
        let mut tokens = line.split_whitespace();
        let Some(label) = tokens.next() else {
            continue;
        };
        let Some(value_token) = tokens.next() else {
            continue;
        };
        let Some(value) = parse_cost_token(value_token) else {
            continue;
        };

        match label {
            "STEPS" => air_cost.steps = value,
            "BASE" => {
                air_cost.base = value;
                found_component = true;
            }
            "MAIN" => {
                air_cost.main = value;
                found_component = true;
            }
            "OPCODES" => {
                air_cost.opcodes = value;
                found_component = true;
            }
            "PRECOMPILES" => {
                air_cost.precompiles = value;
                found_component = true;
            }
            "MEMORY" => {
                air_cost.memory = value;
                found_component = true;
            }
            "TOTAL" => air_cost.total = value,
            _ => {}
        }
    }

    if !found_component {
        return Err(BackendError::execution(
            "ziskemu output contained no COST DISTRIBUTION block",
        ));
    }

    Ok(air_cost)
}

#[cfg(test)]
mod tests {
    use super::*;

    // The committed real sample from Task 6. Path is relative to this crate.
    const SAMPLE: &str =
        include_str!("../../../../tooling/zkevm_bench/fixtures/ziskemu_sample.txt");

    #[test]
    fn parses_air_cost_from_sample() {
        let ac = parse_air_cost(SAMPLE).unwrap();
        assert_eq!(ac.steps, 40_007_528);
        assert_eq!(ac.base, 293_601_280);
        assert_eq!(ac.main, 2_720_511_904);
        assert_eq!(ac.opcodes, 482_648_015);
        assert_eq!(ac.precompiles, 937_548_926);
        assert_eq!(ac.memory, 495_887_679);
        assert_eq!(ac.total, 4_930_197_804);
        // invariant: components sum to total
        assert_eq!(
            ac.total,
            ac.base + ac.main + ac.opcodes + ac.precompiles + ac.memory
        );
    }

    #[test]
    fn errors_on_missing_cost_block() {
        assert!(parse_air_cost("no distribution here\nOP add 1 2%\n").is_err());
    }
}
