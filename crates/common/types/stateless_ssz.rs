//! SSZ containers for stateless validation (Native Rollups).
//!
//! Re-exports common EIP-8025 types from `eip8025_ssz` and defines additional
//! containers for the EXECUTE precompile / native rollups flow:
//! - `SszStatelessInput`
//! - `SszStatelessValidationResult`
//! - `SszExecutionWitness`
//! - `SszChainConfig`

use libssz::{SszDecode, SszEncode};
use libssz_derive::{HashTreeRoot, SszDecode, SszEncode};
use libssz_types::SszList;

// Re-export common types from eip8025_ssz so callers can use a single import path.
pub use super::eip8025_ssz::{
    Bytes20, ConsolidationRequest, DepositRequest, ExecutionPayload, ExecutionPayloadHeader,
    LogsBloom, NewPayloadRequest, NewPayloadRequestHeader, PublicInput, Withdrawal,
    WithdrawalRequest,
};

// ── Stateless validation limits (execution-specs projects/zkevm) ─

/// MAX_WITNESS_NODES — max trie-node preimages in an execution witness.
const MAX_WITNESS_NODES: usize = 1_048_576; // 2^20
/// MAX_WITNESS_NODE_SIZE — max size of a single witness node.
const MAX_WITNESS_NODE_SIZE: usize = 1_048_576; // 2^20
/// MAX_WITNESS_CODES — max contract code preimages in an execution witness.
const MAX_WITNESS_CODES: usize = 65_536; // 2^16
/// MAX_WITNESS_CODE_SIZE — max size of a single code preimage.
const MAX_WITNESS_CODE_SIZE: usize = 1_048_576; // 2^20
/// MAX_WITNESS_HEADERS — max RLP-encoded block headers in witness (up to 256).
const MAX_WITNESS_HEADERS: usize = 256;
/// MAX_WITNESS_HEADER_SIZE — max size of a single RLP-encoded header.
const MAX_WITNESS_HEADER_SIZE: usize = 1_048_576; // 2^20
/// MAX_PUBLIC_KEYS — max recovered transaction public keys.
const MAX_PUBLIC_KEYS: usize = 1_048_576; // 2^20
/// MAX_BYTES_PER_PUBLIC_KEY.
const MAX_BYTES_PER_PUBLIC_KEY: usize = 48;

// ── Stateless validation types ───────────────────────────────────
//
// These match the execution-specs `projects/zkevm` branch definitions:
// - StatelessInput (stateless.py)
// - StatelessValidationResult (stateless.py)
// - ExecutionWitness (stateless.py)
// - ChainConfig (stateless.py)

/// SSZ `ChainConfig` container — matches the execution-specs definition
/// (`projects/zkevm` branch, `stateless.py`).
///
/// Only carries `chain_id`. Fork rules are implicit: the EXECUTE precompile
/// and stateless validator always run at the latest fork (Amsterdam), so all
/// prior forks are activated at timestamp 0 during conversion to the internal
/// `ChainConfig`.
#[derive(Debug, Clone, PartialEq, Eq, SszEncode, SszDecode, HashTreeRoot)]
pub struct SszChainConfig {
    pub chain_id: u64,
}

/// SSZ `ExecutionWitness` container matching the execution-specs definition.
///
/// Contains all data needed for stateless execution:
/// - `state`: trie-node preimages
/// - `codes`: contract code preimages
/// - `headers`: RLP-encoded parent block headers (up to 256)
#[derive(Debug, Clone, PartialEq, Eq, SszEncode, SszDecode, HashTreeRoot)]
pub struct SszExecutionWitness {
    pub state: SszList<SszList<u8, MAX_WITNESS_NODE_SIZE>, MAX_WITNESS_NODES>,
    pub codes: SszList<SszList<u8, MAX_WITNESS_CODE_SIZE>, MAX_WITNESS_CODES>,
    pub headers: SszList<SszList<u8, MAX_WITNESS_HEADER_SIZE>, MAX_WITNESS_HEADERS>,
}

/// SSZ `StatelessInput` — the top-level input to `verify_stateless_new_payload`.
///
/// Wraps a `NewPayloadRequest` together with the execution witness,
/// chain configuration, and (optionally) pre-recovered public keys.
#[derive(Debug, Clone, PartialEq, Eq, SszEncode, SszDecode, HashTreeRoot)]
pub struct SszStatelessInput {
    pub new_payload_request: NewPayloadRequest,
    pub witness: SszExecutionWitness,
    pub chain_config: SszChainConfig,
    pub public_keys: SszList<SszList<u8, MAX_BYTES_PER_PUBLIC_KEY>, MAX_PUBLIC_KEYS>,
}

/// SSZ `StatelessValidationResult` — the output of `verify_stateless_new_payload`.
#[derive(Debug, Clone, PartialEq, Eq, SszEncode, SszDecode, HashTreeRoot)]
pub struct SszStatelessValidationResult {
    pub new_payload_request_root: [u8; 32],
    pub successful_validation: bool,
    pub chain_config: SszChainConfig,
}

// ── Conversions to internal types ────────────────────────────────

impl SszExecutionWitness {
    /// Extract raw bytes from SSZ lists for codes.
    pub fn codes_as_vecs(&self) -> Vec<Vec<u8>> {
        self.codes
            .iter()
            .map(|c| c.iter().copied().collect())
            .collect()
    }

    /// Extract raw bytes from SSZ lists for headers.
    pub fn headers_as_vecs(&self) -> Vec<Vec<u8>> {
        self.headers
            .iter()
            .map(|h| h.iter().copied().collect())
            .collect()
    }

    /// Extract raw bytes from SSZ lists for state nodes.
    pub fn state_as_vecs(&self) -> Vec<Vec<u8>> {
        self.state
            .iter()
            .map(|n| n.iter().copied().collect())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn list<T: SszEncode + SszDecode, const N: usize>(items: Vec<T>) -> SszList<T, N> {
        let mut list = SszList::new();
        for item in items {
            list.push(item);
        }
        list
    }

    fn round_trip<T: SszEncode + SszDecode + PartialEq + std::fmt::Debug>(value: &T) {
        let mut buf = Vec::new();
        value.ssz_append(&mut buf);
        let decoded = T::from_ssz_bytes(&buf).expect("SSZ decode failed");
        assert_eq!(*value, decoded, "round-trip mismatch");
    }

    #[test]
    fn ssz_chain_config_round_trip() {
        round_trip(&SszChainConfig { chain_id: 1 });
        round_trip(&SszChainConfig { chain_id: 0 });
        round_trip(&SszChainConfig { chain_id: u64::MAX });
    }

    #[test]
    fn ssz_execution_witness_round_trip() {
        let witness = SszExecutionWitness {
            state: list(vec![list(vec![1u8, 2, 3]), list(vec![4u8, 5])]),
            codes: list(vec![list(vec![0x60u8, 0x00, 0x60, 0x00, 0xf3])]),
            headers: list(vec![list(vec![0xf9u8, 0x02, 0x11])]),
        };
        round_trip(&witness);
    }

    #[test]
    fn ssz_execution_witness_empty_round_trip() {
        let witness = SszExecutionWitness {
            state: SszList::new(),
            codes: SszList::new(),
            headers: SszList::new(),
        };
        round_trip(&witness);
    }

    #[test]
    fn ssz_stateless_validation_result_round_trip() {
        let result = SszStatelessValidationResult {
            new_payload_request_root: [0xab; 32],
            successful_validation: true,
            chain_config: SszChainConfig { chain_id: 42 },
        };
        round_trip(&result);

        let result_false = SszStatelessValidationResult {
            new_payload_request_root: [0x00; 32],
            successful_validation: false,
            chain_config: SszChainConfig { chain_id: 1 },
        };
        round_trip(&result_false);
    }
}
