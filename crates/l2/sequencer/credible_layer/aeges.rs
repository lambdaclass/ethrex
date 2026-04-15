use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use tonic::transport::Channel;
use tracing::{debug, info, warn};

use super::aeges_proto::{
    aeges_service_client::AegesServiceClient, VerifyTransactionRequest,
    Transaction as AegesTransaction,
};
use super::errors::CredibleLayerError;

/// Configuration for the Aeges mempool pre-filter.
#[derive(Debug, Clone)]
pub struct AegesConfig {
    /// gRPC endpoint URL for the Aeges service
    pub aeges_url: String,
    /// Timeout for the VerifyTransaction call
    pub timeout: Duration,
}

impl Default for AegesConfig {
    fn default() -> Self {
        Self {
            aeges_url: "http://localhost:8080".to_string(),
            timeout: Duration::from_millis(200),
        }
    }
}

/// gRPC client for the Aeges mempool pre-filter service.
///
/// Validates transactions before mempool admission via a simple unary RPC.
/// On any error or timeout, the transaction is admitted (permissive behavior).
pub struct AegesClient {
    config: AegesConfig,
    client: AegesServiceClient<Channel>,
    event_id_counter: AtomicU64,
}

impl AegesClient {
    /// Connect to the Aeges service.
    pub async fn connect(config: AegesConfig) -> Result<Self, CredibleLayerError> {
        info!(url = %config.aeges_url, "Connecting to Aeges service");

        let channel = Channel::from_shared(config.aeges_url.clone())
            .map_err(|e| CredibleLayerError::Internal(format!("Invalid Aeges URL: {e}")))?
            .connect()
            .await?;

        let client = AegesServiceClient::new(channel);

        info!("Connected to Aeges service");

        Ok(Self {
            config,
            client,
            event_id_counter: AtomicU64::new(1),
        })
    }

    /// Verify a transaction with the Aeges service.
    ///
    /// Returns `true` if the transaction should be admitted to the mempool,
    /// `false` if it should be rejected.
    /// On any error or timeout, returns `true` (permissive behavior).
    pub async fn verify_transaction(&self, transaction: AegesTransaction) -> bool {
        let event_id = self.event_id_counter.fetch_add(1, Ordering::Relaxed);

        let request = VerifyTransactionRequest {
            event_id,
            transaction: Some(transaction),
        };

        let result = tokio::time::timeout(self.config.timeout, {
            let mut client = self.client.clone();
            async move { client.verify_transaction(request).await }
        })
        .await;

        match result {
            Ok(Ok(response)) => {
                let denied = response.into_inner().denied;
                if denied {
                    debug!(event_id, "Aeges denied transaction");
                }
                !denied
            }
            Ok(Err(status)) => {
                warn!(%status, "Aeges VerifyTransaction failed, admitting tx (permissive)");
                true
            }
            Err(_) => {
                warn!("Aeges VerifyTransaction timed out, admitting tx (permissive)");
                true
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::sequencer::credible_layer::aeges_proto::{
        Transaction as AegesTransaction, VerifyTransactionRequest, VerifyTransactionResponse,
    };

    // ── AegesConfig defaults ─────────────────────────────────────────────────

    #[test]
    fn config_default_url_is_localhost_8080() {
        let cfg = AegesConfig::default();
        assert_eq!(cfg.aeges_url, "http://localhost:8080");
    }

    #[test]
    fn config_default_timeout_is_200ms() {
        let cfg = AegesConfig::default();
        assert_eq!(cfg.timeout, Duration::from_millis(200));
    }

    // ── VerifyTransactionResponse logic ─────────────────────────────────────

    /// Simulate the core result-mapping logic from `verify_transaction` without
    /// needing a real gRPC connection.
    fn admission_from_response(response: VerifyTransactionResponse) -> bool {
        !response.denied
    }

    #[test]
    fn admitted_when_denied_is_false() {
        let resp = VerifyTransactionResponse {
            event_id: 1,
            denied: false,
        };
        assert!(admission_from_response(resp));
    }

    #[test]
    fn rejected_when_denied_is_true() {
        let resp = VerifyTransactionResponse {
            event_id: 2,
            denied: true,
        };
        assert!(!admission_from_response(resp));
    }

    // ── AegesTransaction construction ────────────────────────────────────────

    #[test]
    fn aeges_transaction_fields_roundtrip() {
        let sender = vec![0xaau8; 20];
        let tx_hash = vec![0xbbu8; 32];
        let value = vec![0u8; 32];

        let tx = AegesTransaction {
            hash: tx_hash.clone(),
            sender: sender.clone(),
            to: None,
            value: value.clone(),
            nonce: 5,
            r#type: 2,
            chain_id: Some(1),
            payload: vec![],
            gas_limit: 21_000,
            gas_price: None,
            max_fee_per_gas: Some(1_000_000_000),
            max_priority_fee_per_gas: Some(100_000_000),
            max_fee_per_blob_gas: None,
            access_list: vec![],
            versioned_hashes: vec![],
            code_delegation_list: vec![],
        };

        assert_eq!(tx.hash, tx_hash);
        assert_eq!(tx.sender, sender);
        assert_eq!(tx.nonce, 5);
        assert_eq!(tx.gas_limit, 21_000);
        assert!(tx.to.is_none());
    }

    // ── VerifyTransactionRequest construction ────────────────────────────────

    #[test]
    fn verify_request_includes_event_id_and_transaction() {
        let tx = AegesTransaction {
            hash: vec![1u8; 32],
            sender: vec![2u8; 20],
            to: Some(vec![3u8; 20]),
            value: vec![0u8; 32],
            nonce: 0,
            r#type: 0,
            chain_id: None,
            payload: vec![],
            gas_limit: 21_000,
            gas_price: Some(1_000_000_000),
            max_fee_per_gas: None,
            max_priority_fee_per_gas: None,
            max_fee_per_blob_gas: None,
            access_list: vec![],
            versioned_hashes: vec![],
            code_delegation_list: vec![],
        };

        let req = VerifyTransactionRequest {
            event_id: 99,
            transaction: Some(tx),
        };

        assert_eq!(req.event_id, 99);
        assert!(req.transaction.is_some());
    }

    // ── Permissive behavior on error ─────────────────────────────────────────

    #[test]
    fn permissive_behavior_on_grpc_error() {
        // Simulate the Err branch: gRPC call returned a status error.
        // The match arm returns `true` (admit the tx).
        let simulated_result: Result<Result<tonic::Response<VerifyTransactionResponse>, tonic::Status>, tokio::time::error::Elapsed> =
            Ok(Err(tonic::Status::internal("server error")));

        let admitted = match simulated_result {
            Ok(Ok(response)) => !response.into_inner().denied,
            Ok(Err(_status)) => true,
            Err(_) => true,
        };

        assert!(admitted, "should admit tx when gRPC returns an error status");
    }
}
