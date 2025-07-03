use aligned_sdk::{
    aggregation_layer::{
        AggregationModeVerificationData, ProofStatus, ProofVerificationAggModeError,
        check_proof_verification as aligned_check_proof_verification,
    },
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
use reqwest::Url;
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

const ALIGNED_VERIFY_FUNCTION_SIGNATURE: &str =
    "verifyBatchesAligned(uint256,bytes[],bytes32[][],bytes[],bytes32[][])";

pub async fn start_l1_proof_verifier(
    cfg: SequencerConfig,
    rollup_store: StoreRollup,
    needed_proof_types: Vec<ProverType>,
) -> Result<(), SequencerError> {
    let l1_proof_verifier = L1ProofVerifier::new(
        &cfg.proof_coordinator,
        &cfg.l1_committer,
        &cfg.eth,
        &cfg.aligned,
        rollup_store,
        needed_proof_types,
    )
    .await?;
    l1_proof_verifier.run().await;
    Ok(())
}

struct L1ProofVerifier {
    eth_client: EthClient,
    beacon_urls: Vec<String>,
    l1_address: Address,
    l1_private_key: SecretKey,
    on_chain_proposer_address: Address,
    proof_verify_interval_ms: u64,
    network: Network,
    rollup_store: StoreRollup,
    sp1_vk: [u8; 32],
    risc0_vk: [u8; 32],
    needed_proof_types: Vec<ProverType>,
}

impl L1ProofVerifier {
    async fn new(
        proof_coordinator_cfg: &ProofCoordinatorConfig,
        committer_cfg: &CommitterConfig,
        eth_cfg: &EthConfig,
        aligned_cfg: &AlignedConfig,
        rollup_store: StoreRollup,
        needed_proof_types: Vec<ProverType>,
    ) -> Result<Self, ProofVerifierError> {
        let eth_client = EthClient::new_with_multiple_urls(eth_cfg.rpc_url.clone())?;
        let beacon_urls = parse_beacon_urls(&aligned_cfg.beacon_urls);

        let sp1_vk = eth_client
            .get_sp1_vk(committer_cfg.on_chain_proposer_address)
            .await?;
        let risc0_vk = eth_client
            .get_risc0_vk(committer_cfg.on_chain_proposer_address)
            .await?;

        Ok(Self {
            eth_client,
            beacon_urls,
            network: aligned_cfg.network.clone(),
            l1_address: proof_coordinator_cfg.l1_address,
            l1_private_key: proof_coordinator_cfg.l1_private_key,
            on_chain_proposer_address: committer_cfg.on_chain_proposer_address,
            proof_verify_interval_ms: aligned_cfg.aligned_verifier_interval_ms,
            rollup_store,
            sp1_vk,
            risc0_vk,
            needed_proof_types,
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

    async fn main_logic(&self) -> Result<(), ProofVerifierError> {
        let first_batch_to_verify = 1 + self
            .eth_client
            .get_last_verified_batch(self.on_chain_proposer_address)
            .await?;

        for prover_type in &self.needed_proof_types {
            if self
                .rollup_store
                .get_proof_by_batch_and_type(first_batch_to_verify, *prover_type)
                .await?
                .is_none()
            {
                info!(
                    ?first_batch_to_verify,
                    ?prover_type,
                    "Missing proof, skipping verification"
                );
                return Ok(());
            };
        }

        match self
            .verify_proofs_aggregation(first_batch_to_verify)
            .await?
        {
            Some(verify_tx_hash) => {
                info!(
                    "Batches verified in OnChainProposer, with transaction hash {verify_tx_hash:#x}"
                );
            }
            None => {
                info!(
                    "Batch {first_batch_to_verify} has not yet been aggregated by Aligned. Waiting for {} seconds",
                    self.proof_verify_interval_ms / 1000
                );
            }
        }
        Ok(())
    }

    async fn verify_proofs_aggregation(
        &self,
        first_batch_number: u64,
    ) -> Result<Option<H256>, ProofVerifierError> {
        let proofs = self.get_available_proofs(first_batch_number).await?;
        let aggregated_proofs = self.get_aggregated_proofs(proofs).await?;

        let aggregated_proofs_count = u64::try_from(aggregated_proofs.len())
            .map_err(|e| ProofVerifierError::InternalError(e.to_string()))?;

        match aggregated_proofs_count {
            0 => return Ok(None),
            1 => info!("Sending verify tx for batch {first_batch_number}"),
            n => {
                info!(
                    "Sending verify tx for batches {first_batch_number} to {}",
                    first_batch_number + n - 1
                );
            }
        };

        let mut sp1_public_inputs_vec = Vec::new();
        let mut sp1_merkle_paths = Vec::new();
        let mut risc0_public_inputs_vec = Vec::new();
        let mut risc0_merkle_paths = Vec::new();

        for (prover_type, public_inputs, merkle_path) in aggregated_proofs {
            let merkle_path = merkle_path
                .iter()
                .map(|x| Value::FixedBytes(bytes::Bytes::from_owner(*x)))
                .collect();
            match prover_type {
                ProverType::SP1 => {
                    sp1_public_inputs_vec.push(Value::Bytes(public_inputs.into()));
                    sp1_merkle_paths.push(Value::Array(merkle_path));
                }
                ProverType::RISC0 => {
                    risc0_public_inputs_vec.push(Value::Bytes(public_inputs.into()));
                    risc0_merkle_paths.push(Value::Array(merkle_path));
                }
                unsupported_type => {
                    return Err(ProofVerifierError::UnsupportedProverType(
                        unsupported_type.to_string(),
                    ));
                }
            }
        }

        let calldata_values = [
            Value::Uint(U256::from(first_batch_number)),
            Value::Array(sp1_public_inputs_vec),
            Value::Array(sp1_merkle_paths),
            Value::Array(risc0_public_inputs_vec),
            Value::Array(risc0_merkle_paths),
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

        // Store the verify transaction hash for each batch that was aggregated.
        for i in 0..aggregated_proofs_count {
            let batch_number = first_batch_number + i;
            self.rollup_store
                .store_verify_tx_by_batch(batch_number, verify_tx_hash)
                .await?;
        }

        Ok(Some(verify_tx_hash))
    }

    /// Returns all proofs that have already been generated, starting from the given batch number.
    async fn get_available_proofs(
        &self,
        mut batch_number: u64,
    ) -> Result<Vec<(u64, BatchProof)>, ProofVerifierError> {
        let mut proofs = Vec::new();
        for prover_type in &self.needed_proof_types {
            while let Some(proof) = self
                .rollup_store
                .get_proof_by_batch_and_type(batch_number, *prover_type)
                .await?
            {
                proofs.push((batch_number, proof));
                batch_number += 1;
            }
        }
        Ok(proofs)
    }

    /// Receives an array of proofs.
    /// Returns merkle proofs for only those zk proofs that were aggregated by Aligned.
    async fn get_aggregated_proofs(
        &self,
        proofs: Vec<(u64, BatchProof)>,
    ) -> Result<Vec<(ProverType, Vec<u8>, Vec<[u8; 32]>)>, ProofVerifierError> {
        let mut aggregated_proofs = Vec::new();
        for (batch_number, proof) in proofs {
            let public_inputs = proof.public_values();
            let prover_type = proof.prover_type();

            let verification_data = match prover_type {
                ProverType::RISC0 => AggregationModeVerificationData::Risc0 {
                    image_id: self.risc0_vk,
                    public_inputs: public_inputs.clone(),
                },
                ProverType::SP1 => AggregationModeVerificationData::SP1 {
                    vk: self.sp1_vk,
                    public_inputs: public_inputs.clone(),
                },
                unsupported_type => {
                    return Err(ProofVerifierError::UnsupportedProverType(
                        unsupported_type.to_string(),
                    ));
                }
            };
            let commitment = H256(verification_data.commitment());

            if let Some((merkle_root, merkle_path)) =
                self.check_proof_aggregation(verification_data).await?
            {
                info!(
                    ?batch_number,
                    commitment = %format_args!("{commitment:#x}"),
                    merkle_root = %format_args!("{merkle_root:#x}"),
                    "Proof aggregated by Aligned"
                );
                aggregated_proofs.push((prover_type, public_inputs, merkle_path));
            }
        }
        Ok(aggregated_proofs)
    }

    /// Checks if the received proof was aggregated by Aligned.
    async fn check_proof_aggregation(
        &self,
        verification_data: AggregationModeVerificationData,
    ) -> Result<Option<(H256, Vec<[u8; 32]>)>, ProofVerifierError> {
        let proof_status = self.check_proof_verification(&verification_data).await?;

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

        let merkle_root = H256(merkle_root);

        Ok(Some((merkle_root, merkle_path)))
    }

    /// Performs the call to the aligned proof verification function with retries over multiple RPC URLs and beacon URLs.
    async fn check_proof_verification(
        &self,
        verification_data: &AggregationModeVerificationData,
    ) -> Result<ProofStatus, ProofVerifierError> {
        for rpc_url in &self.eth_client.urls {
            for beacon_url in &self.beacon_urls {
                match aligned_check_proof_verification(
                    verification_data,
                    self.network.clone(),
                    rpc_url.as_str().into(),
                    beacon_url.clone(),
                    None,
                )
                .await
                {
                    Ok(proof_status) => return Ok(proof_status),
                    Err(ProofVerificationAggModeError::BeaconClient(_)) => continue,
                    Err(ProofVerificationAggModeError::EthereumProviderError(_)) => break,
                    Err(e) => return Err(ProofVerifierError::InternalError(format!("{e:?}"))),
                }
            }
        }
        Err(ProofVerifierError::InternalError(
            "Verification failed. All RPC URLs were exhausted.".to_string(),
        ))
    }
}

fn parse_beacon_urls(beacon_urls: &[Url]) -> Vec<String> {
    beacon_urls
        .iter()
        .map(|url| {
            url.as_str()
                .strip_suffix('/')
                .unwrap_or_else(|| url.as_str())
                .to_string()
        })
        .collect()
}
