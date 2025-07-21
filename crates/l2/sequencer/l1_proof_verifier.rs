use std::collections::HashMap;
use std::time::Duration;

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
use ethrex_l2_rpc::signer::Signer;
use ethrex_l2_sdk::calldata::encode_calldata;
use ethrex_rpc::EthClient;
use ethrex_storage_rollup::StoreRollup;
use reqwest::Url;
use spawned_concurrency::{
    messages::Unused,
    tasks::{CastResponse, GenServer, GenServerHandle, send_after},
};
use tracing::{error, info};

use crate::{
    CommitterConfig, EthConfig, ProofCoordinatorConfig, sequencer::errors::ProofVerifierError,
};

use super::{configs::AlignedConfig, utils::send_verify_tx};

const ALIGNED_VERIFY_FUNCTION_SIGNATURE: &str =
    "verifyBatchesAligned(uint256,bytes[],bytes32[][],bytes32[][])";

#[derive(Clone, Debug)]
pub enum CastInMessage {
    Prove,
}

#[derive(Clone, PartialEq, Debug)]
pub enum OutMessage {
    Done,
}
pub struct L1ProofVerifier {}

impl L1ProofVerifier {
    pub async fn spawn(
        proof_coordinator_cfg: ProofCoordinatorConfig,
        committer_cfg: &CommitterConfig,
        eth_cfg: &EthConfig,
        aligned_cfg: &AlignedConfig,
        rollup_store: StoreRollup,
        needed_proof_types: Vec<ProverType>,
    ) -> Result<(), ProofVerifierError> {
        let state = L1ProofVerifierState::new(
            proof_coordinator_cfg,
            committer_cfg,
            eth_cfg,
            aligned_cfg,
            rollup_store,
            needed_proof_types
        )
        .await?;
        let mut handle = L1ProofVerifier::start(state);
        handle
            .cast(CastInMessage::Prove)
            .await
            .map_err(ProofVerifierError::GenServer)?;
        Ok(())
    }
}

impl GenServer for L1ProofVerifier {
    type CallMsg = Unused;
    type CastMsg = CastInMessage;
    type OutMsg = OutMessage;
    type State = L1ProofVerifierState;
    type Error = ProofVerifierError;

    fn new() -> Self {
        Self {}
    }

    async fn handle_cast(
        &mut self,
        _message: Self::CastMsg,
        handle: &GenServerHandle<Self>,
        state: Self::State,
    ) -> CastResponse<Self> {
        let _ = state.main_logic().await.inspect_err(|err| {
            error!("L1 Proof Verifier Error: {err}");
        });
        send_after(
            Duration::from_millis(state.proof_verify_interval_ms),
            handle.clone(),
            Self::CastMsg::Prove,
        );
        CastResponse::NoReply(state)
    }
}

#[derive(Clone)]
pub struct L1ProofVerifierState {
    eth_client: EthClient,
    beacon_urls: Vec<String>,
    l1_signer: Signer,
    on_chain_proposer_address: Address,
    proof_verify_interval_ms: u64,
    network: Network,
    rollup_store: StoreRollup,
    sp1_vk: [u8; 32],
    risc0_vk: [u8; 32],
    needed_proof_types: Vec<ProverType>,
}

impl L1ProofVerifierState {
    async fn new(
        proof_coordinator_cfg: ProofCoordinatorConfig,
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
            l1_signer: proof_coordinator_cfg.signer,
            on_chain_proposer_address: committer_cfg.on_chain_proposer_address,
            proof_verify_interval_ms: aligned_cfg.aligned_verifier_interval_ms,
            rollup_store,
            sp1_vk,
            risc0_vk,
            needed_proof_types,
        })
    }

    async fn main_logic(&self) -> Result<(), ProofVerifierError> {
        let first_batch_to_verify = 1 + self
            .eth_client
            .get_last_verified_batch(self.on_chain_proposer_address)
            .await?;

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
        let mut public_inputs_list = Vec::new();
        let mut sp1_merkle_proofs_list = Vec::new();
        let mut risc0_merkle_proofs_list = Vec::new();

        let mut batch_number = first_batch_number;
        loop {
            let proofs_for_batch = self.get_proofs_for_batch(batch_number).await?;

            // break if not all required proof types have been found for this batch
            if proofs_for_batch.len() != self.needed_proof_types.len() {
                break;
            }

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

                let verification_data = self.verification_data(prover_type, public_inputs)?;

                if let Some((merkle_root, merkle_path)) =
                    self.check_proof_aggregation(verification_data).await?
                {
                    info!(
                        ?batch_number,
                        ?prover_type,
                        merkle_root = %format_args!("{merkle_root:#x}"),
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

            let public_inputs =
                current_batch_public_inputs.ok_or(ProofVerifierError::InternalError(format!(
                    "no proofs for batch {batch_number}, are there any needed proof types?"
                )))?;

            public_inputs_list.push(Value::Bytes(public_inputs.into()));

            let sp1_merkle_proof =
                self.proof_of_inclusion(&aggregated_proofs_for_batch, ProverType::SP1);
            let risc0_merkle_proof =
                self.proof_of_inclusion(&aggregated_proofs_for_batch, ProverType::RISC0);

            sp1_merkle_proofs_list.push(sp1_merkle_proof);
            risc0_merkle_proofs_list.push(risc0_merkle_proof);

            batch_number += 1;
        }

        if public_inputs_list.is_empty() {
            return Ok(None);
        }

        let num_batches: u64 = public_inputs_list.len().try_into().map_err(|_| {
            ProofVerifierError::InternalError("Failed to convert num_batches into u64".to_string())
        })?;
        info!(
            "Sending verify tx for batches {} to {}",
            first_batch_number,
            first_batch_number + num_batches - 1
        );

        let calldata_values = [
            Value::Uint(U256::from(first_batch_number)),
            Value::Array(public_inputs_list),
            Value::Array(sp1_merkle_proofs_list),
            Value::Array(risc0_merkle_proofs_list),
        ];

        let calldata = encode_calldata(ALIGNED_VERIFY_FUNCTION_SIGNATURE, &calldata_values)?;

        let verify_tx_hash = send_verify_tx(
            calldata,
            &self.eth_client,
            self.on_chain_proposer_address,
            &self.l1_signer,
        )
        .await?;

        // store the verify transaction hash for each batch that was aggregated.
        for i in 0..num_batches {
            let batch_number = first_batch_number + i;
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

    fn verification_data(
        &self,
        prover_type: ProverType,
        public_inputs: Vec<u8>,
    ) -> Result<AggregationModeVerificationData, ProofVerifierError> {
        let verification_data = match prover_type {
            ProverType::SP1 => AggregationModeVerificationData::SP1 {
                vk: self.sp1_vk,
                public_inputs: public_inputs.clone(),
            },
            ProverType::RISC0 => AggregationModeVerificationData::Risc0 {
                image_id: self.risc0_vk,
                public_inputs: public_inputs.clone(),
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
