use std::collections::HashMap;

use aligned_sdk::{
    blockchain::{
        AggregationModeVerificationData, ProofStatus, ProofVerificationAggModeError,
        provider::ProofAggregationServiceProvider,
    },
    types::Network,
};
use ethrex_common::{Address, H256, U256};
use ethrex_l2_common::{
    calldata::Value,
    prover::{BatchProof, ProverType},
};
use ethrex_l2_rpc::signer::Signer;
use ethrex_l2_sdk::{
    calldata::encode_calldata, get_last_verified_batch, get_risc0_vk_for_batch,
    get_sp1_vk_for_batch,
};
use ethrex_rpc::{
    EthClient,
    clients::{EthClientError, eth::errors::RpcRequestError},
};
use ethrex_storage_rollup::StoreRollup;
use reqwest::Url;
use tracing::{error, info, warn};

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
    "verifyBatchesAligned(uint256,uint256,bytes32[][],bytes32[][])";

pub async fn start_l1_proof_verifier(
    cfg: SequencerConfig,
    rollup_store: StoreRollup,
    needed_proof_types: Vec<ProverType>,
) -> Result<(), SequencerError> {
    let l1_proof_verifier = L1ProofVerifier::new(
        cfg.proof_coordinator,
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
    l1_signer: Signer,
    on_chain_proposer_address: Address,
    timelock_address: Option<Address>,
    proof_verify_interval_ms: u64,
    network: Network,
    rollup_store: StoreRollup,
    needed_proof_types: Vec<ProverType>,
    from_block: Option<u64>,
}

impl L1ProofVerifier {
    async fn new(
        proof_coordinator_cfg: ProofCoordinatorConfig,
        committer_cfg: &CommitterConfig,
        eth_cfg: &EthConfig,
        aligned_cfg: &AlignedConfig,
        rollup_store: StoreRollup,
        needed_proof_types: Vec<ProverType>,
    ) -> Result<Self, ProofVerifierError> {
        let eth_client = EthClient::new_with_config(
            eth_cfg.rpc_url.clone(),
            eth_cfg.max_number_of_retries,
            eth_cfg.backoff_factor,
            eth_cfg.min_retry_delay,
            eth_cfg.max_retry_delay,
            Some(eth_cfg.maximum_allowed_max_fee_per_gas),
            Some(eth_cfg.maximum_allowed_max_fee_per_blob_gas),
        )?;
        let beacon_urls = parse_beacon_urls(&aligned_cfg.beacon_urls);

        Ok(Self {
            eth_client,
            beacon_urls,
            network: aligned_cfg.network.clone(),
            l1_signer: proof_coordinator_cfg.signer,
            on_chain_proposer_address: committer_cfg.on_chain_proposer_address,
            timelock_address: committer_cfg.timelock_address,
            proof_verify_interval_ms: aligned_cfg.aligned_verifier_interval_ms,
            rollup_store,
            needed_proof_types,
            from_block: aligned_cfg.from_block,
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
        let first_batch_to_verify =
            1 + get_last_verified_batch(&self.eth_client, self.on_chain_proposer_address).await?;

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

    /// Checks that all consecutive batches starting from `first_batch_number` have been
    /// verified and aggregated in Aligned Layer. This advances the OnChainProposer.
    async fn verify_proofs_aggregation(
        &self,
        first_batch_number: u64,
    ) -> Result<Option<H256>, ProofVerifierError> {
        let mut sp1_merkle_proofs_list = Vec::new();
        let mut risc0_merkle_proofs_list = Vec::new();

        let mut batch_number = first_batch_number;
        loop {
            let proofs_for_batch = self.get_proofs_for_batch(batch_number).await?;

            // break if not all required proof types have been found for this batch
            if proofs_for_batch.len() != self.needed_proof_types.len() {
                break;
            }

            // Fetch VKs for this batch from the contract
            let (sp1_vk, risc0_vk) = self.get_vks_for_batch(batch_number).await?;

            let mut aggregated_proofs_for_batch = HashMap::new();
            let mut current_batch_public_inputs = None;

            for (prover_type, proof) in proofs_for_batch {
                let public_inputs = proof.public_values();

                // check all proofs have the same public inputs
                if let Some(ref existing_pi) = current_batch_public_inputs {
                    if *existing_pi != public_inputs {
                        return Err(ProofVerifierError::MismatchedPublicInputs {
                            batch_number,
                            prover_type,
                            existing_hex: hex::encode(existing_pi),
                            latest_hex: hex::encode(public_inputs),
                        });
                    }
                } else {
                    current_batch_public_inputs = Some(public_inputs.clone());
                }

                // Create verification_data to get the commitment
                let verification_data =
                    Self::verification_data(prover_type, public_inputs.clone(), sp1_vk, risc0_vk)?;
                let commitment = H256(verification_data.commitment());
                if let Some((merkle_root, merkle_path)) = self
                    .check_proof_aggregation(prover_type, public_inputs, sp1_vk, risc0_vk)
                    .await?
                {
                    info!(
                        ?batch_number,
                        ?prover_type,
                        merkle_root = %format_args!("{merkle_root:#x}"),
                        commitment = %format_args!("{commitment:#x}"),
                        "Proof aggregated by Aligned"
                    );
                    aggregated_proofs_for_batch.insert(prover_type, merkle_path);
                } else {
                    info!(
                        ?prover_type,
                        "Proof has not been aggregated by Aligned, aborting"
                    );
                    break;
                }
            }

            // break if not all required proof types have been aggregated for this batch
            if aggregated_proofs_for_batch.len() != self.needed_proof_types.len() {
                break;
            }

            // Note: RISC0 merkle proofs are collected even though RISC0 is not currently
            // supported by Aligned in aggregation mode. These will be empty arrays since
            // needed_proof_types won't include RISC0 when aligned mode is enabled.
            // The contract's verifyBatchesAligned() accepts these empty arrays and skips
            // RISC0 verification when REQUIRE_RISC0_PROOF is false.
            // This code path is preserved for future compatibility when Aligned re-enables RISC0.
            let sp1_merkle_proof =
                self.proof_of_inclusion(&aggregated_proofs_for_batch, ProverType::SP1);
            let risc0_merkle_proof =
                self.proof_of_inclusion(&aggregated_proofs_for_batch, ProverType::RISC0);

            sp1_merkle_proofs_list.push(sp1_merkle_proof);
            risc0_merkle_proofs_list.push(risc0_merkle_proof);

            batch_number += 1;
        }

        if first_batch_number == batch_number {
            return Ok(None);
        }

        let last_batch_number = batch_number - 1;

        info!("Sending verify tx for batches {first_batch_number} to {last_batch_number}",);

        let calldata_values = [
            Value::Uint(U256::from(first_batch_number)),
            Value::Uint(U256::from(last_batch_number)),
            Value::Array(sp1_merkle_proofs_list),
            Value::Array(risc0_merkle_proofs_list),
        ];

        let calldata = encode_calldata(ALIGNED_VERIFY_FUNCTION_SIGNATURE, &calldata_values)?;

        // Based won't have timelock address until we implement it on it. For the meantime if it's None (only happens in based) we use the OCP
        let target_address = self
            .timelock_address
            .unwrap_or(self.on_chain_proposer_address);

        let send_verify_tx_result =
            send_verify_tx(calldata, &self.eth_client, target_address, &self.l1_signer).await;

        if let Err(EthClientError::RpcRequestError(RpcRequestError::RPCError { message, .. })) =
            send_verify_tx_result.as_ref()
            && message.contains("00m")
        // Invalid Aligned proof
        {
            warn!("Deleting invalid ALIGNED proof");
            for batch_number in first_batch_number..=last_batch_number {
                for proof_type in &self.needed_proof_types {
                    self.rollup_store
                        .delete_proof_by_batch_and_type(batch_number, *proof_type)
                        .await?;
                }
            }
        }
        let verify_tx_hash = send_verify_tx_result?;

        // store the verify transaction hash for each batch that was aggregated.
        for batch_number in first_batch_number..=last_batch_number {
            self.rollup_store
                .store_verify_tx_by_batch(batch_number, verify_tx_hash)
                .await?;
        }

        Ok(Some(verify_tx_hash))
    }

    fn proof_of_inclusion(
        &self,
        aggregated_proofs_for_batch: &HashMap<ProverType, Vec<[u8; 32]>>,
        prover_type: ProverType,
    ) -> Value {
        aggregated_proofs_for_batch
            .get(&prover_type)
            .map(|path| {
                Value::Array(
                    path.iter()
                        .map(|p| Value::FixedBytes(bytes::Bytes::from_owner(*p)))
                        .collect(),
                )
            })
            .unwrap_or_else(|| Value::Array(vec![]))
    }

    /// Fetches the verification keys for a batch from the contract.
    async fn get_vks_for_batch(
        &self,
        batch_number: u64,
    ) -> Result<([u8; 32], [u8; 32]), ProofVerifierError> {
        let sp1_vk = get_sp1_vk_for_batch(
            &self.eth_client,
            self.on_chain_proposer_address,
            batch_number,
        )
        .await?;
        let risc0_vk = get_risc0_vk_for_batch(
            &self.eth_client,
            self.on_chain_proposer_address,
            batch_number,
        )
        .await?;
        Ok((sp1_vk, risc0_vk))
    }

    fn verification_data(
        prover_type: ProverType,
        public_inputs: Vec<u8>,
        sp1_vk: [u8; 32],
        risc0_vk: [u8; 32],
    ) -> Result<AggregationModeVerificationData, ProofVerifierError> {
        let verification_data = match prover_type {
            ProverType::SP1 => AggregationModeVerificationData::SP1 {
                vk: sp1_vk,
                public_inputs,
            },
            ProverType::RISC0 => AggregationModeVerificationData::Risc0 {
                image_id: risc0_vk,
                public_inputs,
            },
            unsupported_type => {
                return Err(ProofVerifierError::UnsupportedProverType(
                    unsupported_type.to_string(),
                ));
            }
        };
        Ok(verification_data)
    }

    async fn get_proofs_for_batch(
        &self,
        batch_number: u64,
    ) -> Result<HashMap<ProverType, BatchProof>, ProofVerifierError> {
        let mut proofs_for_batch = HashMap::new();
        for prover_type in &self.needed_proof_types {
            if let Some(proof) = self
                .rollup_store
                .get_proof_by_batch_and_type(batch_number, *prover_type)
                .await?
            {
                proofs_for_batch.insert(*prover_type, proof);
            } else {
                break;
            }
        }
        Ok(proofs_for_batch)
    }

    /// Checks if the received proof was aggregated by Aligned.
    async fn check_proof_aggregation(
        &self,
        prover_type: ProverType,
        public_inputs: Vec<u8>,
        sp1_vk: [u8; 32],
        risc0_vk: [u8; 32],
    ) -> Result<Option<(H256, Vec<[u8; 32]>)>, ProofVerifierError> {
        let proof_status = self
            .check_proof_verification(prover_type, public_inputs, sp1_vk, risc0_vk)
            .await?;

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
        prover_type: ProverType,
        public_inputs: Vec<u8>,
        sp1_vk: [u8; 32],
        risc0_vk: [u8; 32],
    ) -> Result<ProofStatus, ProofVerifierError> {
        for rpc_url in &self.eth_client.urls {
            for beacon_url in &self.beacon_urls {
                // Create a provider for each RPC/beacon combination
                let provider = ProofAggregationServiceProvider::new(
                    self.network.clone(),
                    rpc_url.to_string(),
                    beacon_url.clone(),
                );

                // Recreate verification data for each attempt (it doesn't implement Clone)
                let verification_data =
                    Self::verification_data(prover_type, public_inputs.clone(), sp1_vk, risc0_vk)?;

                match provider
                    .check_proof_verification(self.from_block, verification_data)
                    .await
                {
                    Ok(proof_status) => return Ok(proof_status),
                    Err(ProofVerificationAggModeError::BeaconClient(e)) => {
                        warn!(
                            "Beacon client error when checking proof verification with RPC URL {rpc_url} and Beacon URL {beacon_url}: {e:?}. Trying next combination.",
                        );
                        continue;
                    }
                    Err(ProofVerificationAggModeError::EthereumProviderError(e)) => {
                        warn!(
                            "Ethereum provider error when checking proof verification with RPC URL {rpc_url} and Beacon URL {beacon_url}: {e:?}. Trying next combination.",
                        );
                        continue;
                    }
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
