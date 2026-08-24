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
    /// Guest's peak zkVM memory footprint in bytes, from ziskemu's `RAM
    /// USAGE` line. This is a footprint measurement, not a proving-cost
    /// component (unlike `memory`, the AIR cost of memory opcodes).
    pub ram_usage: u64,
}

/// Parses a comma-grouped integer token (e.g. `"4,930,197,804"`) into a `u64`.
fn parse_cost_token(token: &str) -> Option<u64> {
    token.replace(',', "").parse::<u64>().ok()
}

/// Parses the `COST DISTRIBUTION` summary block out of raw `ziskemu` stdout.
///
/// The exact first whitespace-separated token of each line is matched
/// against the summary labels. The value is then taken as the *first*
/// remaining whitespace token on that line that parses as a `u64` once `,`
/// grouping separators are stripped. A single extraction rule this way
/// serves both single-word labels (`BASE 293,601,280 5.96%`, where the
/// trailing percentage's `.` makes it fail to parse and get skipped) and the
/// two-word `RAM USAGE` label (where `USAGE` fails to parse and gets
/// skipped, leaving `7,304,122` as the value). This also keeps the parser
/// immune to the detailed per-opcode tables that follow the summary in the
/// real output (e.g. lines starting with `OP`, `COST BY OPCODE`, or
/// `FROPS`), since their first token never matches a summary label. It is
/// likewise order-independent, so the `VARIABLE` line (`= TOTAL - BASE`) that
/// ZisK v1.0.0-alpha added to the summary is simply ignored.
pub fn parse_air_cost(stdout: &str) -> Result<ZiskAirCost, BackendError> {
    let mut air_cost = ZiskAirCost::default();
    let mut found_component = false;

    for line in stdout.lines() {
        let mut tokens = line.split_whitespace();
        let Some(label) = tokens.next() else {
            continue;
        };
        let Some(value) = tokens.filter_map(parse_cost_token).next() else {
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
            "RAM" => air_cost.ram_usage = value,
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

    // A committed real ZisK v1.0.0-alpha `ziskemu -X` capture of the
    // `mainnet_25087668_light` benchmark block. Path is relative to this crate.
    const SAMPLE: &str =
        include_str!("../../../../tooling/zkevm_bench/fixtures/ziskemu_sample.txt");

    #[test]
    fn parses_air_cost_from_sample() {
        let ac = parse_air_cost(SAMPLE).unwrap();
        assert_eq!(ac.steps, 8_573_814);
        assert_eq!(ac.base, 293_601_280);
        assert_eq!(ac.main, 583_019_352);
        assert_eq!(ac.opcodes, 124_468_967);
        assert_eq!(ac.precompiles, 227_527_215);
        assert_eq!(ac.memory, 80_638_696);
        assert_eq!(ac.total, 1_309_255_510);
        assert_eq!(ac.ram_usage, 3_697_848);
        // invariant: components sum to total (still holds in v1.0.0-alpha; the
        // added `VARIABLE` line — total minus base — is ignored by the parser).
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
