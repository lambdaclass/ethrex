use aligned_sdk::{
    aggregation_layer::{AggregationModeVerificationData, ProofStatus, check_proof_verification},
    common::types::Network,
};
use ethrex_common::{Address, H256, U256};
use ethrex_l2_common::{
    calldata::Value,
    prover::{BatchProof, ProverType},
};
use ethrex_l2_sdk::calldata::encode_calldata;
use ethrex_rpc::EthClient;
use ethrex_storage_rollup::StoreRollup;
use secp256k1::SecretKey;
use tracing::{error, info};

use crate::{
    CommitterConfig, EthConfig, ProofCoordinatorConfig, SequencerConfig,
    sequencer::errors::ProofVerifierError,
};

use super::{
    configs::AlignedConfig,
    errors::SequencerError,
    utils::{send_verify_tx, sleep_random},
};

const ALIGNED_VERIFY_FUNCTION_SIGNATURE: &str = "verifyBatchAligned(uint256,bytes,bytes32[])";

pub async fn start_l1_proof_verifier(
    cfg: SequencerConfig,
    rollup_store: StoreRollup,
) -> Result<(), SequencerError> {
    let l1_proof_verifier = L1ProofVerifier::new(
        &cfg.proof_coordinator,
        &cfg.l1_committer,
        &cfg.eth,
        &cfg.aligned,
        rollup_store,
    )
    .await?;
    l1_proof_verifier.run().await;
    Ok(())
}

struct L1ProofVerifier {
    eth_client: EthClient,
    beacon_url: String,
    l1_address: Address,
    l1_private_key: SecretKey,
    on_chain_proposer_address: Address,
    proof_verify_interval_ms: u64,
    network: Network,
    rollup_store: StoreRollup,
    sp1_vk: [u8; 32],
}

impl L1ProofVerifier {
    async fn new(
        proof_coordinator_cfg: &ProofCoordinatorConfig,
        committer_cfg: &CommitterConfig,
        eth_cfg: &EthConfig,
        aligned_cfg: &AlignedConfig,
        rollup_store: StoreRollup,
    ) -> Result<Self, ProofVerifierError> {
        let eth_client = EthClient::new_with_multiple_urls(eth_cfg.rpc_url.clone())?;

        let sp1_vk = eth_client
            .get_sp1_vk(committer_cfg.on_chain_proposer_address)
            .await?;

        Ok(Self {
            eth_client,
            beacon_url: aligned_cfg.beacon_url.clone(),
            network: aligned_cfg.network.clone(),
            l1_address: proof_coordinator_cfg.l1_address,
            l1_private_key: proof_coordinator_cfg.l1_private_key,
            on_chain_proposer_address: committer_cfg.on_chain_proposer_address,
            proof_verify_interval_ms: aligned_cfg.aligned_verifier_interval_ms,
            rollup_store,
            sp1_vk,
        })
    }

    async fn run(&self) {
        info!("Running L1 Proof Verifier");
        loop {
            if let Err(err) = self.main_logic().await {
                error!("L1 Proof Verifier Error: {}", err);
            }

            sleep_random(self.proof_verify_interval_ms).await;
        }
    }

    // TODO: verify all already aggregated proofs in one tx
    async fn main_logic(&self) -> Result<(), ProofVerifierError> {
        let batch_to_verify = 1 + self
            .eth_client
            .get_last_verified_batch(self.on_chain_proposer_address)
            .await?;

        let Some(aligned_proof) = self
            .rollup_store
            .get_proof_by_batch_and_type(batch_to_verify, ProverType::Aligned)
            .await?
        else {
            info!(
                ?batch_to_verify,
                "Missing Aligned proof, skipping verification"
            );
            return Ok(());
        };

        match self
            .verify_proof_aggregation(batch_to_verify, aligned_proof)
            .await?
        {
            Some(verify_tx_hash) => {
                self.rollup_store
                    .store_verify_tx_by_batch(batch_to_verify, verify_tx_hash)
                    .await?;
                info!(
                    "Batch {batch_to_verify} verified in AlignedProofAggregatorService, with transaction hash {verify_tx_hash:#x}"
                );
            }
            None => {
                info!(
                    "Batch {batch_to_verify} has not yet been aggregated by Aligned. Waiting for {} seconds",
                    self.proof_verify_interval_ms / 1000
                );
            }
        }
        Ok(())
    }

    async fn verify_proof_aggregation(
        &self,
        batch_number: u64,
        aligned_proof: BatchProof,
    ) -> Result<Option<H256>, ProofVerifierError> {
        let public_inputs = aligned_proof.public_values();

        let verification_data = AggregationModeVerificationData::SP1 {
            vk: self.sp1_vk,
            public_inputs: public_inputs.clone(),
        };

        let rpc_url = self.eth_client.urls.first().ok_or_else(|| {
            ProofVerifierError::InternalError("No Ethereum RPC URL configured".to_owned())
        })?;

        let proof_status = check_proof_verification(
            &verification_data,
            self.network.clone(),
            rpc_url.as_str().into(),
            self.beacon_url.clone(),
            None,
        )
        .await
        .map_err(|e| ProofVerifierError::InternalError(format!("{:?}", e)))?;

        let (merkle_root, merkle_path) = match proof_status {
            ProofStatus::Verified {
                merkle_root,
                merkle_path,
            } => (merkle_root, merkle_path),
            ProofStatus::Invalid => {
                return Err(ProofVerifierError::InternalError(
                    "Proof was found in the blob but the Merkle Root verification failed."
                        .to_string(),
                ));
            }
            ProofStatus::NotFound => {
                return Ok(None);
            }
        };

        let commitment = H256(verification_data.commitment());
        let merkle_root = H256(merkle_root);

        info!(
            "Proof for batch {batch_number} aggregated by Aligned with commitment {commitment:#x} and Merkle root {merkle_root:#x}"
        );

        let merkle_path = merkle_path
            .iter()
            .map(|x| Value::FixedBytes(bytes::Bytes::from_owner(*x)))
            .collect();

        let calldata_values = [
            Value::Uint(U256::from(batch_number)),
            Value::Bytes(public_inputs.into()),
            Value::Array(merkle_path),
        ];

        let calldata = encode_calldata(ALIGNED_VERIFY_FUNCTION_SIGNATURE, &calldata_values)?;

        let verify_tx_hash = send_verify_tx(
            calldata,
            &self.eth_client,
            self.on_chain_proposer_address,
            self.l1_address,
            &self.l1_private_key,
        )
        .await?;

        Ok(Some(verify_tx_hash))
    }
}
