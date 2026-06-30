//! Maps EEST canonical exception names (e.g. `TransactionException.NONCE_MISMATCH_TOO_LOW`)
//! to ethrex's actual error wording.
//!
//! Ported from `execution-specs/packages/testing/src/execution_testing/client_clis/clis/ethrex.py`.
//! When the Python mapper is updated (new exceptions or reworded ethrex messages), this file
//! must be updated in lock-step. The tests-vs-mapper drift is the main source of
//! `validation_error` mismatch noise; keep this honest.
//!
//! Match logic: a fixture's expected `validation_error` is one or more canonical names
//! (e.g. `"TransactionException.A|TransactionException.B"`). We consider it a match if
//! any alternative's substring OR regex (both tables consulted) matches ethrex's actual
//! validationError string.

use regex::Regex;
use std::collections::HashMap;
use std::sync::OnceLock;

#[derive(Clone, Copy)]
enum Kind {
    /// Plain `actual.contains(text)`.
    Sub,
    /// `Regex::new(text).is_match(actual)`.
    Re,
}

struct Entry {
    canonical: &'static str,
    kind: Kind,
    text: &'static str,
}

// ─── Mapping tables (kept in lock-step with the Python `EthrexExceptionMapper`) ────────

#[rustfmt::skip]
const PATTERNS: &[Entry] = &[
    // ─── mapping_substring ─────────────────────────────────────────────────────────
    Entry { canonical: "BlockException.INVALID_GASLIMIT", kind: Kind::Sub,
        text: "Gas limit changed more than allowed from the parent" },
    Entry { canonical: "TransactionException.TYPE_3_TX_MAX_BLOB_GAS_ALLOWANCE_EXCEEDED", kind: Kind::Sub,
        text: "Exceeded MAX_BLOB_GAS_PER_BLOCK" },
    Entry { canonical: "BlockException.INVALID_DEPOSIT_EVENT_LAYOUT", kind: Kind::Sub,
        text: "Invalid deposit request layout" },
    Entry { canonical: "BlockException.INVALID_REQUESTS", kind: Kind::Sub,
        text: "Requests hash does not match the one in the header after executing" },
    Entry { canonical: "BlockException.INVALID_RECEIPTS_ROOT", kind: Kind::Sub,
        text: "Receipts Root does not match the one in the header after executing" },
    Entry { canonical: "BlockException.INVALID_STATE_ROOT", kind: Kind::Sub,
        text: "World State Root does not match the one in the header after executing" },
    Entry { canonical: "BlockException.GAS_USED_OVERFLOW", kind: Kind::Sub,
        text: "Gas allowance exceeded" },
    Entry { canonical: "BlockException.INVALID_BLOCK_ACCESS_LIST", kind: Kind::Sub,
        text: "Block access list hash does not match the one in the header after executing" },
    Entry { canonical: "BlockException.INVALID_BAL_HASH", kind: Kind::Sub,
        text: "Block access list hash does not match the one in the header after executing" },
    Entry { canonical: "BlockException.INCORRECT_BLOCK_FORMAT", kind: Kind::Sub,
        text: "not in strictly ascending order for" },
    Entry { canonical: "BlockException.BLOCK_ACCESS_LIST_GAS_LIMIT_EXCEEDED", kind: Kind::Sub,
        text: "Block access list exceeds gas limit" },
    Entry { canonical: "BlockException.INVALID_GAS_USED", kind: Kind::Sub,
        text: "Gas used doesn't match value in header" },
    Entry { canonical: "BlockException.INCORRECT_BLOB_GAS_USED", kind: Kind::Sub,
        text: "Blob gas used doesn't match value in header" },
    Entry { canonical: "BlockException.INVALID_BASEFEE_PER_GAS", kind: Kind::Sub,
        text: "Base fee per gas is incorrect" },

    // ─── mapping_regex ────────────────────────────────────────────────────────────
    Entry { canonical: "TransactionException.PRIORITY_GREATER_THAN_MAX_FEE_PER_GAS", kind: Kind::Re,
        text: r"(?i)priority fee.* is greater than max fee.*" },
    Entry { canonical: "TransactionException.TYPE_4_EMPTY_AUTHORIZATION_LIST", kind: Kind::Re,
        text: r"(?i)empty authorization list" },
    Entry { canonical: "TransactionException.SENDER_NOT_EOA", kind: Kind::Re,
        text: r"reject transactions from senders with deployed code|Sender account .* shouldn't be a contract" },
    Entry { canonical: "TransactionException.NONCE_MISMATCH_TOO_LOW", kind: Kind::Re,
        text: r"nonce \d+ too low, expected \d+|Nonce mismatch.*" },
    Entry { canonical: "TransactionException.NONCE_MISMATCH_TOO_HIGH", kind: Kind::Re,
        text: r"Nonce mismatch.*" },
    Entry { canonical: "TransactionException.TYPE_3_TX_MAX_BLOB_GAS_ALLOWANCE_EXCEEDED", kind: Kind::Re,
        text: r"blob gas used \d+ exceeds maximum allowance \d+" },
    Entry { canonical: "TransactionException.TYPE_3_TX_ZERO_BLOBS", kind: Kind::Re,
        text: r"blob transactions present in pre-cancun payload|empty blobs|Type 3 transaction without blobs" },
    Entry { canonical: "TransactionException.TYPE_3_TX_INVALID_BLOB_VERSIONED_HASH", kind: Kind::Re,
        text: r"blob version not supported|Invalid blob versioned hash" },
    Entry { canonical: "TransactionException.TYPE_2_TX_PRE_FORK", kind: Kind::Re,
        text: r"Type 2 transactions are not supported before the London fork" },
    Entry { canonical: "TransactionException.TYPE_3_TX_PRE_FORK", kind: Kind::Re,
        text: r"blob versioned hashes not supported|Type 3 transactions are not supported before the Cancun fork" },
    Entry { canonical: "TransactionException.TYPE_4_TX_CONTRACT_CREATION", kind: Kind::Re,
        text: r"unexpected length|Contract creation in type 4 transaction|Error decoding field 'to' of type primitive_types::H160: InvalidLength" },
    Entry { canonical: "TransactionException.TYPE_3_TX_CONTRACT_CREATION", kind: Kind::Re,
        text: r"unexpected length|Contract creation in type 3 transaction|Error decoding field 'to' of type primitive_types::H160: InvalidLength" },
    Entry { canonical: "TransactionException.TYPE_4_TX_PRE_FORK", kind: Kind::Re,
        text: r"eip 7702 transactions present in pre-prague payload|Type 4 transactions are not supported before the Prague fork" },
    Entry { canonical: "TransactionException.INSUFFICIENT_ACCOUNT_FUNDS", kind: Kind::Re,
        text: r"lack of funds \(\d+\) for max fee \(\d+\)|Insufficient account funds" },
    Entry { canonical: "TransactionException.INTRINSIC_GAS_TOO_LOW", kind: Kind::Re,
        text: r"gas floor exceeds the gas limit|call gas cost exceeds the gas limit|Transaction gas limit lower than the minimum gas cost to execute the transaction|Transaction gas limit lower than the gas cost floor for calldata tokens" },
    Entry { canonical: "TransactionException.INTRINSIC_GAS_BELOW_FLOOR_GAS_COST", kind: Kind::Re,
        text: r"Transaction gas limit lower than the gas cost floor for calldata tokens" },
    Entry { canonical: "TransactionException.INSUFFICIENT_MAX_FEE_PER_GAS", kind: Kind::Re,
        text: r"gas price is less than basefee|Insufficient max fee per gas" },
    Entry { canonical: "TransactionException.INSUFFICIENT_MAX_FEE_PER_BLOB_GAS", kind: Kind::Re,
        text: r"blob gas price is greater than max fee per blob gas|Insufficient max fee per blob gas.*" },
    Entry { canonical: "TransactionException.INITCODE_SIZE_EXCEEDED", kind: Kind::Re,
        text: r"create initcode size limit|Initcode size exceeded.*" },
    Entry { canonical: "TransactionException.NONCE_IS_MAX", kind: Kind::Re,
        text: r"Nonce is max" },
    Entry { canonical: "TransactionException.GAS_ALLOWANCE_EXCEEDED", kind: Kind::Re,
        text: r"Gas allowance exceeded.*" },
    Entry { canonical: "BlockException.GAS_USED_OVERFLOW", kind: Kind::Re,
        text: r"Gas allowance exceeded.*" },
    Entry { canonical: "TransactionException.TYPE_3_TX_BLOB_COUNT_EXCEEDED", kind: Kind::Re,
        text: r"Blob count exceeded.*" },
    Entry { canonical: "TransactionException.GASLIMIT_PRICE_PRODUCT_OVERFLOW", kind: Kind::Re,
        text: r"Invalid transaction: Gas limit price product overflow.*" },
    Entry { canonical: "TransactionException.GAS_LIMIT_EXCEEDS_MAXIMUM", kind: Kind::Re,
        text: r"Invalid transaction: Transaction gas limit exceeds maximum.*" },
    Entry { canonical: "BlockException.INVALID_DEPOSIT_EVENT_LAYOUT", kind: Kind::Re,
        text: r"Invalid deposit request layout|BAL validation failed.*" },
    Entry { canonical: "BlockException.SYSTEM_CONTRACT_CALL_FAILED", kind: Kind::Re,
        text: r"System call failed.*" },
    Entry { canonical: "BlockException.SYSTEM_CONTRACT_EMPTY", kind: Kind::Re,
        text: r"System contract:.* has no code after deployment" },
    Entry { canonical: "BlockException.INCORRECT_BLOB_GAS_USED", kind: Kind::Re,
        text: r"Blob gas used doesn't match value in header" },
    Entry { canonical: "BlockException.RLP_STRUCTURES_ENCODING", kind: Kind::Re,
        text: r"Error decoding field '\D+' of type \w+.*" },
    Entry { canonical: "BlockException.INCORRECT_EXCESS_BLOB_GAS", kind: Kind::Re,
        text: r".* Excess blob gas is incorrect" },
    Entry { canonical: "BlockException.INVALID_BLOCK_HASH", kind: Kind::Re,
        text: r"Invalid block hash. Expected \w+, got \w+" },
    Entry { canonical: "BlockException.RLP_BLOCK_LIMIT_EXCEEDED", kind: Kind::Re,
        text: r"Maximum block size exceeded.*" },
    Entry { canonical: "BlockException.INVALID_BAL_HASH", kind: Kind::Re,
        text: r"BAL validation failed" },
    Entry { canonical: "BlockException.INVALID_BLOCK_ACCESS_LIST", kind: Kind::Re,
        text: r"Block access list contains index \d+ exceeding max valid index \d+|Failed to RLP decode BAL|Block access list .+ not in strictly ascending order.*|BAL validation failed for (tx \d+|system_tx|withdrawal): .*|BAL validation failed: .*|absent from BAL|Block access list slot .+ is in both storage_changes and storage_reads.*|Block access list .+ has an empty change set|Invalid block hash" },
    Entry { canonical: "BlockException.INCORRECT_BLOCK_FORMAT", kind: Kind::Re,
        text: r"Block access list hash does not match the one in the header after executing|Block access list contains index \d+ exceeding max valid index \d+|Failed to RLP decode BAL|Block access list accounts not in strictly ascending order.*" },
];

fn compiled() -> &'static HashMap<&'static str, Regex> {
    static CELL: OnceLock<HashMap<&'static str, Regex>> = OnceLock::new();
    CELL.get_or_init(|| {
        PATTERNS
            .iter()
            .filter(|e| matches!(e.kind, Kind::Re))
            .map(|e| {
                let re = Regex::new(e.text)
                    .unwrap_or_else(|err| panic!("invalid mapper regex `{}`: {err}", e.text));
                (e.text, re)
            })
            .collect()
    })
}

/// Whether ethrex's `actual` validationError matches a single canonical exception name.
pub fn matches_canonical(canonical: &str, actual: &str) -> bool {
    let regexes = compiled();
    for entry in PATTERNS {
        if entry.canonical != canonical {
            continue;
        }
        let hit = match entry.kind {
            Kind::Sub => actual.contains(entry.text),
            Kind::Re => regexes
                .get(entry.text)
                .map(|re| re.is_match(actual))
                .unwrap_or(false),
        };
        if hit {
            return true;
        }
    }
    false
}

/// `expected` may be one or more `|`-separated canonical names. Any alternative
/// matching `actual` is treated as a match.
pub fn matches(expected: &str, actual: &str) -> bool {
    expected
        .split('|')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .any(|alt| matches_canonical(alt, actual))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn substring_match_bal_hash() {
        assert!(matches_canonical(
            "BlockException.INVALID_BAL_HASH",
            "Block access list hash does not match the one in the header after executing",
        ));
    }

    #[test]
    fn regex_match_nonce_too_low() {
        assert!(matches_canonical(
            "TransactionException.NONCE_MISMATCH_TOO_LOW",
            "nonce 5 too low, expected 7",
        ));
        assert!(matches_canonical(
            "TransactionException.NONCE_MISMATCH_TOO_LOW",
            "Nonce mismatch for sender 0xabc",
        ));
    }

    #[test]
    fn or_alternatives() {
        assert!(matches(
            "TransactionException.NONCE_MISMATCH_TOO_LOW|TransactionException.NONCE_MISMATCH_TOO_HIGH",
            "Nonce mismatch for sender 0xabc",
        ));
    }

    #[test]
    fn unknown_canonical_returns_false() {
        assert!(!matches_canonical(
            "TransactionException.DOES_NOT_EXIST",
            "anything at all",
        ));
    }

    #[test]
    fn no_alternative_matches() {
        assert!(!matches(
            "BlockException.INVALID_BAL_HASH",
            "some unrelated ethrex message",
        ));
    }
}
