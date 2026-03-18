use serde::{Deserialize, Serialize};
use url::Url;

/// Configuration for the EIP-8025 proof coordinator.
///
/// Provides the callback URL for delivering generated proofs to the Beacon API,
/// and the bind address/port for the ProofCoordinator's TCP server where
/// prover workers connect.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProofCoordinatorConfig {
    /// URL to POST `GeneratedProof` payloads to (Beacon API endpoint).
    /// Example: `http://beacon:5052/eth/v1/prover/execution_proofs`
    /// `None` means no callback delivery — proofs are stored but not pushed.
    pub callback_url: Option<Url>,
    /// Bind address for the ProofCoordinator TCP server (e.g. "127.0.0.1").
    pub coordinator_addr: String,
    /// Port for the ProofCoordinator TCP server.
    pub coordinator_port: u16,
}

impl Default for ProofCoordinatorConfig {
    fn default() -> Self {
        Self {
            callback_url: None,
            coordinator_addr: "127.0.0.1".to_string(),
            coordinator_port: 9100,
        }
    }
}
