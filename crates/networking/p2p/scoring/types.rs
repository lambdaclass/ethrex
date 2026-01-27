//! Core types for the peer scoring system.

use std::fmt;

/// Types of requests that can be made to peers.
///
/// This allows the scoring system to track performance per request type,
/// enabling request-type-aware peer selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RequestType {
    // ETH protocol requests
    /// Block header requests (eth/66+)
    BlockHeaders,
    /// Block body requests (eth/66+)
    BlockBodies,
    /// Receipt requests (eth/66+)
    Receipts,
    /// Pooled transaction requests (eth/66+)
    PooledTransactions,

    // SNAP protocol requests
    /// Account range requests (snap/1)
    AccountRange,
    /// Storage range requests (snap/1)
    StorageRanges,
    /// Bytecode requests (snap/1)
    ByteCodes,
    /// Trie node requests (snap/1)
    TrieNodes,
}

impl RequestType {
    /// Returns all request types.
    pub fn all() -> &'static [RequestType] {
        &[
            RequestType::BlockHeaders,
            RequestType::BlockBodies,
            RequestType::Receipts,
            RequestType::PooledTransactions,
            RequestType::AccountRange,
            RequestType::StorageRanges,
            RequestType::ByteCodes,
            RequestType::TrieNodes,
        ]
    }

    /// Returns true if this is an ETH protocol request.
    pub fn is_eth(self) -> bool {
        matches!(
            self,
            RequestType::BlockHeaders
                | RequestType::BlockBodies
                | RequestType::Receipts
                | RequestType::PooledTransactions
        )
    }

    /// Returns true if this is a SNAP protocol request.
    pub fn is_snap(self) -> bool {
        matches!(
            self,
            RequestType::AccountRange
                | RequestType::StorageRanges
                | RequestType::ByteCodes
                | RequestType::TrieNodes
        )
    }

    /// Returns a short name for logging.
    pub fn short_name(self) -> &'static str {
        match self {
            RequestType::BlockHeaders => "headers",
            RequestType::BlockBodies => "bodies",
            RequestType::Receipts => "receipts",
            RequestType::PooledTransactions => "pool_txs",
            RequestType::AccountRange => "acc_range",
            RequestType::StorageRanges => "stor_range",
            RequestType::ByteCodes => "bytecodes",
            RequestType::TrieNodes => "trie_nodes",
        }
    }
}

impl fmt::Display for RequestType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.short_name())
    }
}

/// Severity levels for request failures.
///
/// Different failure types have different impacts on peer scoring:
/// - Lower severity failures (timeout, empty response) have smaller penalty
/// - Higher severity failures (hash mismatch, proof failure) have larger penalty
/// - Critical failures (intentional malice) result in immediate blacklisting
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum FailureSeverity {
    /// Low severity: timeouts, empty responses, temporary issues.
    ///
    /// Penalty: -1 to reliability score
    Low,

    /// Medium severity: invalid data format, wrong response count.
    ///
    /// Penalty: -3 to reliability score
    Medium,

    /// High severity: hash mismatch, proof verification failure.
    ///
    /// Penalty: -10 to reliability score
    High,

    /// Critical severity: intentionally malicious behavior.
    ///
    /// Penalty: Sets score to minimum (blacklist)
    Critical,
}

impl FailureSeverity {
    /// Returns the penalty value for this severity level.
    ///
    /// Higher severity = larger penalty (more negative).
    pub fn penalty(self) -> f64 {
        match self {
            FailureSeverity::Low => -0.01,     // -1%
            FailureSeverity::Medium => -0.03,  // -3%
            FailureSeverity::High => -0.10,    // -10%
            FailureSeverity::Critical => -1.0, // Immediate blacklist
        }
    }

    /// Returns true if this severity should trigger immediate blacklisting.
    pub fn is_blacklistable(self) -> bool {
        self == FailureSeverity::Critical
    }
}

impl fmt::Display for FailureSeverity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            FailureSeverity::Low => "low",
            FailureSeverity::Medium => "medium",
            FailureSeverity::High => "high",
            FailureSeverity::Critical => "critical",
        };
        write!(f, "{}", name)
    }
}
