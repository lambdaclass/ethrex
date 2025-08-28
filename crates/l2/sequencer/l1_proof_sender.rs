use std::collections::HashMap;

use ethrex_common::{Address, U256};
use ethrex_l2_common::{
    calldata::Value,
    prover::{BatchProof, ProverType},
};
use ethrex_l2_rpc::signer::Signer;
use ethrex_l2_sdk::calldata::encode_calldata;
use ethrex_rpc::EthClient;
use ethrex_storage_rollup::StoreRollup;
use spawned_concurrency::{
    messages::Unused,
    tasks::{CastResponse, GenServer, GenServerHandle, send_after},
};
use tracing::{error, info};

use super::{
    configs::AlignedConfig,
    utils::{random_duration, send_verify_tx},
};

use crate::{
    CommitterConfig, EthConfig, ProofCoordinatorConfig, SequencerConfig,
    based::sequencer_state::{SequencerState, SequencerStatus},
    sequencer::errors::ProofSenderError,
};
use aligned_sdk::{
    common::types::{FeeEstimationType, Network, ProvingSystemId, VerificationData},
    verification_layer::{estimate_fee as aligned_estimate_fee, get_nonce_from_batcher, submit},
};

use ethers::signers::{Signer as EthersSigner, Wallet};

const VERIFY_FUNCTION_SIGNATURE: &str = "verifyBatch(uint256,bytes,bytes,bytes,bytes,bytes,bytes)";

#[derive(Clone)]
pub enum InMessage {
    Send,
}

#[derive(Clone, PartialEq)]
pub enum OutMessage {
    Done,
}

pub struct L1ProofSender {
    eth_client: EthClient,
    signer: ethrex_l2_rpc::signer::Signer,
    on_chain_proposer_address: Address,
    needed_proof_types: Vec<ProverType>,
    proof_send_interval_ms: u64,
    sequencer_state: SequencerState,
    rollup_store: StoreRollup,
    l1_chain_id: u64,
    network: Network,
    fee_estimate: FeeEstimationType,
    aligned_mode: bool,
}

impl L1ProofSender {
    async fn new(
        cfg: &ProofCoordinatorConfig,
        committer_cfg: &CommitterConfig,
        eth_cfg: &EthConfig,
        sequencer_state: SequencerState,
        aligned_cfg: &AlignedConfig,
        rollup_store: StoreRollup,
        needed_proof_types: Vec<ProverType>,
    ) -> Result<Self, ProofSenderError> {
        let eth_client = EthClient::new_with_multiple_urls(eth_cfg.rpc_url.clone())?;
        let l1_chain_id = eth_client.get_chain_id().await?.try_into().map_err(|_| {
            ProofSenderError::UnexpectedError("Failed to convert chain ID to U256".to_owned())
        })?;
        let fee_estimate = resolve_fee_estimate(&aligned_cfg.fee_estimate)?;

        Ok(Self {
            eth_client,
            signer: cfg.signer.clone(),
            on_chain_proposer_address: committer_cfg.on_chain_proposer_address,
            needed_proof_types,
            proof_send_interval_ms: cfg.proof_send_interval_ms,
            sequencer_state,
            rollup_store,
            l1_chain_id,
            network: aligned_cfg.network.clone(),
            fee_estimate,
            aligned_mode: aligned_cfg.aligned_mode,
        })
    }

    pub async fn spawn(
        cfg: SequencerConfig,
        sequencer_state: SequencerState,
        rollup_store: StoreRollup,
        needed_proof_types: Vec<ProverType>,
    ) -> Result<(), ProofSenderError> {
        let state = Self::new(
            &cfg.proof_coordinator,
            &cfg.l1_committer,
            &cfg.eth,
            sequencer_state,
            &cfg.aligned,
            rollup_store,
            needed_proof_types,
        )
        .await?;
        let mut l1_proof_sender = L1ProofSender::start(state);
        l1_proof_sender
            .cast(InMessage::Send)
            .await
            .map_err(ProofSenderError::InternalError)
    }

    async fn verify_and_send_proof(&self) -> Result<(), ProofSenderError> {
        let last_verified_batch = self
            .eth_client
            .get_last_verified_batch(self.on_chain_proposer_address)
            .await?;
        let batch_to_send = if self.aligned_mode {
            let last_sent_batch = self.rollup_store.get_latest_sent_batch_proof().await?;
            std::cmp::max(last_sent_batch, last_verified_batch) + 1
        } else {
            last_verified_batch + 1
        };

        let last_committed_batch = self
            .eth_client
            .get_last_committed_batch(self.on_chain_proposer_address)
            .await?;

        if last_committed_batch < batch_to_send {
            info!("Next batch to send ({batch_to_send}) is not yet committed");
            return Ok(());
        }

        let mut proofs = HashMap::new();
        let mut missing_proof_types = Vec::new();
        for proof_type in &self.needed_proof_types {
            if let Some(proof) = self
                .rollup_store
                .get_proof_by_batch_and_type(batch_to_send, *proof_type)
                .await?
            {
                proofs.insert(*proof_type, proof);
            } else {
                missing_proof_types.push(proof_type);
            }
        }

        if missing_proof_types.is_empty() {
            if self.aligned_mode {
                self.send_proof_to_aligned(batch_to_send, proofs.values())
                    .await?;
            } else {
                self.send_proof_to_contract(batch_to_send, proofs).await?;
            }
            self.rollup_store
                .set_latest_sent_batch_proof(batch_to_send)
                .await?;
        } else {
            let missing_proof_types: Vec<String> = missing_proof_types
                .iter()
                .map(|proof_type| format!("{proof_type:?}"))
                .collect();
            info!(
                ?missing_proof_types,
                ?batch_to_send,
                "Missing batch proof(s), will not send",
            );
        }

        Ok(())
    }

    async fn send_proof_to_aligned(
        &self,
        batch_number: u64,
        batch_proofs: impl IntoIterator<Item = &BatchProof>,
    ) -> Result<(), ProofSenderError> {
        info!(?batch_number, "Sending batch proof(s) to Aligned Layer");

        let fee_estimation = Self::estimate_fee(self).await?;

        let mut nonce =
            get_nonce_from_batcher(self.network.clone(), self.signer.address().0.into())
                .await
                .map_err(|err| {
                    ProofSenderError::AlignedGetNonceError(format!("Failed to get nonce: {err:?}"))
                })?;

        let Signer::Local(local_signer) = &self.signer else {
            return Err(ProofSenderError::UnexpectedError(
                "Aligned mode only supports local signer".to_string(),
            ));
        };

        let wallet = Wallet::from_bytes(local_signer.private_key.as_ref())
            .map_err(|_| ProofSenderError::UnexpectedError("Failed to create wallet".to_owned()))?;

        let wallet = wallet.with_chain_id(self.l1_chain_id);

        for batch_proof in batch_proofs {
            let prover_type = batch_proof.prover_type();
            let proving_system = match prover_type {
                ProverType::RISC0 => ProvingSystemId::Risc0,
                ProverType::SP1 => ProvingSystemId::SP1,
                _ => continue,
            };

            info!(?prover_type, ?batch_number, "Submitting proof to Aligned");

            let Some(proof) = batch_proof.compressed() else {
                return Err(ProofSenderError::AlignedWrongProofFormat);
            };

            let Some(vm_program_code) = prover_type.aligned_vm_program_code()? else {
                return Err(ProofSenderError::InternalError(format!(
                    "no vm_program_code for {prover_type}"
                )));
            };

            let pub_input = match prover_type {
                ProverType::RISC0 => Some(batch_proof.public_values()),
                ProverType::SP1 => None, // SP1 pub inputs are embedded in the proof
                _ => continue,
            };

            let verification_data = VerificationData {
                proving_system,
                proof,
                proof_generator_addr: self.signer.address().0.into(),
                vm_program_code: Some(vm_program_code),
                verification_key: None,
                pub_input,
            };

            submit(
                self.network.clone(),
                &verification_data,
                fee_estimation,
                wallet.clone(),
                nonce,
            )
            .await?;

            nonce = nonce
                .checked_add(1.into())
                .ok_or(ProofSenderError::InternalError(
                    "aligned batcher nonce overflow".to_string(),
                ))?;

            info!(?prover_type, ?batch_number, "Submitted proof to Aligned");
        }

        Ok(())
    }

    /// Performs a call to aligned SDK estimate_fee function with retries over all RPC URLs.
    async fn estimate_fee(&self) -> Result<ethers::types::U256, ProofSenderError> {
        for rpc_url in &self.eth_client.urls {
            if let Ok(estimation) =
                aligned_estimate_fee(rpc_url.as_str(), self.fee_estimate.clone()).await
            {
                return Ok(estimation);
            }
        }
        Err(ProofSenderError::AlignedFeeEstimateError(
            "All Ethereum RPC URLs failed".to_string(),
        ))
    }

    pub async fn send_proof_to_contract(
        &self,
        batch_number: u64,
        proofs: HashMap<ProverType, BatchProof>,
    ) -> Result<(), ProofSenderError> {
        info!(
            ?batch_number,
            "Sending batch verification transaction to L1"
        );

        let calldata_values = [
            &[Value::Uint(U256::from(batch_number))],
            proofs
                .get(&ProverType::RISC0)
                .map(|proof| proof.calldata())
                .unwrap_or(ProverType::RISC0.empty_calldata())
                .as_slice(),
            proofs
                .get(&ProverType::SP1)
                .map(|proof| proof.calldata())
                .unwrap_or(ProverType::SP1.empty_calldata())
                .as_slice(),
            proofs
                .get(&ProverType::TDX)
                .map(|proof| proof.calldata())
                .unwrap_or(ProverType::TDX.empty_calldata())
                .as_slice(),
        ]
        .concat();

        let calldata = encode_calldata(VERIFY_FUNCTION_SIGNATURE, &calldata_values)?;

        let verify_tx_hash = send_verify_tx(
            calldata,
            &self.eth_client,
            self.on_chain_proposer_address,
            &self.signer,
        )
        .await?;

        self.rollup_store
            .store_verify_tx_by_batch(batch_number, verify_tx_hash)
            .await?;

        info!(
            ?batch_number,
            ?verify_tx_hash,
            "Sent batch verification transaction to L1"
        );

        Ok(())
    }
}

impl GenServer for L1ProofSender {
    type CallMsg = Unused;
    type CastMsg = InMessage;
    type OutMsg = OutMessage;

    type Error = ProofSenderError;

    async fn handle_cast(
        &mut self,
        _message: Self::CastMsg,
        handle: &GenServerHandle<Self>,
    ) -> CastResponse {
        // Right now we only have the Send message, so we ignore the message
        if let SequencerStatus::Sequencing = self.sequencer_state.status().await {
            let _ = self
                .verify_and_send_proof()
                .await
                .inspect_err(|err| error!("L1 Proof Sender: {err}"));
        }
        let check_interval = random_duration(self.proof_send_interval_ms);
        send_after(check_interval, handle.clone(), Self::CastMsg::Send);
        CastResponse::NoReply
    }
}

fn resolve_fee_estimate(fee_estimate: &str) -> Result<FeeEstimationType, ProofSenderError> {
    match fee_estimate {
        "instant" => Ok(FeeEstimationType::Instant),
        "default" => Ok(FeeEstimationType::Default),
        _ => Err(ProofSenderError::AlignedFeeEstimateError(
            "Unsupported fee estimation type".to_string(),
        )),
    }
}
