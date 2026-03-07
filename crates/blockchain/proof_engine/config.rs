/// Configuration for the EIP-8025 ProofEngine.
///
/// Built from CLI flags:
/// - `--proof-callback.url`
/// - `--proof-coordinator.addr`
/// - `--proof-coordinator.port`
#[derive(Debug, Clone)]
pub struct ProofEngineConfig {
    /// URL where generated proofs are POSTed back to the CL.
    /// e.g. "http://beacon-node:5052/eth/v1/prover/execution_proofs"
    pub callback_url: Option<String>,
    /// Address the proof coordinator listens on for prover connections.
    /// e.g. "0.0.0.0"
    pub coordinator_addr: String,
    /// Port the proof coordinator listens on.
    /// e.g. 9100
    pub coordinator_port: u16,
}

impl Default for ProofEngineConfig {
    fn default() -> Self {
        Self {
            callback_url: None,
            coordinator_addr: "0.0.0.0".to_string(),
            coordinator_port: 9100,
        }
    }
}

impl ProofEngineConfig {
    /// Returns the coordinator listen socket address as "addr:port".
    pub fn coordinator_socket_addr(&self) -> String {
        format!("{}:{}", self.coordinator_addr, self.coordinator_port)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config() {
        let config = ProofEngineConfig::default();
        assert_eq!(config.coordinator_addr, "0.0.0.0");
        assert_eq!(config.coordinator_port, 9100);
        assert!(config.callback_url.is_none());
        assert_eq!(config.coordinator_socket_addr(), "0.0.0.0:9100");
    }

    #[test]
    fn socket_addr_format() {
        let config = ProofEngineConfig {
            callback_url: Some("http://localhost:5052/eth/v1/prover/execution_proofs".to_string()),
            coordinator_addr: "127.0.0.1".to_string(),
            coordinator_port: 8080,
        };
        assert_eq!(config.coordinator_socket_addr(), "127.0.0.1:8080");
    }
}
