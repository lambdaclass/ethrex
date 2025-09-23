use std::collections::{BTreeMap, HashMap};

use ethrex_common::{Address, U256};
use ethrex_l2_common::{
    calldata::Value,
    prover::{BatchProof, ProverType},
};
use ethrex_l2_rpc::signer::{Signer, SignerHealth};
use ethrex_l2_sdk::{calldata::encode_calldata, get_last_committed_batch};
#[cfg(feature = "metrics")]
use ethrex_metrics::l2::metrics::METRICS;
use ethrex_metrics::metrics;
use ethrex_rpc::{
    EthClient,
    clients::{EthClientError, eth::errors::EstimateGasError},
};
use ethrex_storage_rollup::StoreRollup;
use serde::Serialize;
use spawned_concurrency::tasks::{
    CallResponse, CastResponse, GenServer, GenServerHandle, send_after,
};
use tracing::{debug, error, info, warn};

use super::{
    configs::AlignedConfig,
    utils::{get_latest_sent_batch, random_duration, send_verify_tx},
};

use crate::{
    CommitterConfig, EthConfig, ProofCoordinatorConfig, SequencerConfig,
    based::sequencer_state::{SequencerState, SequencerStatus},
    sequencer::errors::ProofSenderError,
};
use aligned_sdk::{
    common::{
        errors,
        types::{FeeEstimationType, Network, ProvingSystemId, VerificationData},
    },
    verification_layer::{estimate_fee as aligned_estimate_fee, get_nonce_from_batcher, submit},
};

use ethers::signers::{Signer as EthersSigner, Wallet};

const VERIFY_FUNCTION_SIGNATURE: &str = "verifyBatch(uint256,bytes,bytes,bytes,bytes,bytes,bytes)";

#[derive(Clone)]
pub enum InMessage {
    Send,
}

#[derive(Clone)]
pub enum OutMessage {
    Done,
    Health(Box<L1ProofSenderHealth>),
}

#[derive(Clone)]
pub enum CallMessage {
    Health,
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
    aligned_sp1_elf_path: String,
}

#[derive(Clone, Serialize)]
pub struct L1ProofSenderHealth {
    rpc_healthcheck: BTreeMap<String, serde_json::Value>,
    signer_status: SignerHealth,
    on_chain_proposer_address: Address,
    needed_proof_types: Vec<String>,
    proof_send_interval_ms: u64,
    sequencer_state: String,
    l1_chain_id: u64,
    network: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    fee_estimate: Option<FeeEstimationType>,
    #[serde(skip_serializing_if = "Option::is_none")]
    aligned_sp1_elf_path: Option<String>,
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
        let eth_client = EthClient::new_with_config(
            eth_cfg.rpc_url.iter().map(AsRef::as_ref).collect(),
            eth_cfg.max_number_of_retries,
            eth_cfg.backoff_factor,
            eth_cfg.min_retry_delay,
            eth_cfg.max_retry_delay,
            Some(eth_cfg.maximum_allowed_max_fee_per_gas),
            Some(eth_cfg.maximum_allowed_max_fee_per_blob_gas),
        )?;
        let l1_chain_id = eth_client.get_chain_id().await?.try_into().map_err(|_| {
            ProofSenderError::UnexpectedError("Failed to convert chain ID to U256".to_owned())
        })?;
        let fee_estimate = resolve_fee_estimate(&aligned_cfg.fee_estimate)?;
        let aligned_sp1_elf_path = aligned_cfg.aligned_sp1_elf_path.clone();

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
            aligned_sp1_elf_path,
        })
    }

    pub async fn spawn(
        cfg: SequencerConfig,
        sequencer_state: SequencerState,
        rollup_store: StoreRollup,
        needed_proof_types: Vec<ProverType>,
    ) -> Result<GenServerHandle<L1ProofSender>, ProofSenderError> {
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
            .map_err(ProofSenderError::InternalError)?;
        Ok(l1_proof_sender)
    }

    async fn verify_and_send_proof(&mut self) -> Result<(), ProofSenderError> {
        let batch_to_send = 1 + get_latest_sent_batch(
            self.needed_proof_types.clone(),
            &self.rollup_store,
            &self.eth_client,
            self.on_chain_proposer_address,
        )
        .await
        .map_err(|err| {
            error!("Failed to get next batch to send: {err}");
            ProofSenderError::UnexpectedError(err.to_string())
        })?;

        let last_committed_batch =
            get_last_committed_batch(&self.eth_client, self.on_chain_proposer_address).await?;

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
            // TODO: we should put in code that if the prover is running with Aligned, then there
            // shouldn't be any other required types.
            if let Some(aligned_proof) = proofs.remove(&ProverType::Aligned) {
                self.send_proof_to_aligned(batch_to_send, aligned_proof)
                    .await?;
            } else {
                self.send_proof_to_contract(batch_to_send, proofs).await?;
            }
            self.rollup_store
                .set_lastest_sent_batch_proof(batch_to_send)
                .await?;
        } else {
            let missing_proof_types: Vec<String> = missing_proof_types
                .iter()
                .map(|proof_type| format!("{proof_type:?}"))
                .collect();
            info!(
                ?batch_to_send,
                "Missing {} batch proof(s), will not send",
                missing_proof_types.join(", ")
            );
        }

        Ok(())
    }

    async fn send_proof_to_aligned(
        &mut self,
        batch_number: u64,
        aligned_proof: BatchProof,
    ) -> Result<(), ProofSenderError> {
        let elf = std::fs::read(self.aligned_sp1_elf_path.clone()).map_err(|e| {
            ProofSenderError::UnexpectedError(format!("Failed to read ELF file: {e}"))
        })?;

        let verification_data = VerificationData {
            proving_system: ProvingSystemId::SP1,
            proof: aligned_proof.proof(),
            proof_generator_addr: self.signer.address().0.into(),
            vm_program_code: Some(elf),
            verification_key: None,
            pub_input: None,
        };

        let fee_estimation = self.estimate_fee().await?;

        let nonce = get_nonce_from_batcher(self.network.clone(), self.signer.address().0.into())
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

        debug!("Sending proof to Aligned");

        let algined_verification_result = submit(
            self.network.clone(),
            &verification_data,
            fee_estimation,
            wallet,
            nonce,
        )
        .await;

        if let Err(errors::SubmitError::InvalidProof(_)) = algined_verification_result.as_ref() {
            warn!("Deleting invalid ALIGNED proof");
            self.rollup_store
                .delete_proof_by_batch_and_type(batch_number, ProverType::Aligned)
                .await?;
        }

        algined_verification_result.map_err(|err| {
            ProofSenderError::AlignedSubmitProofError(format!("Failed to submit proof: {err}"))
        })?;

        info!("Proof for batch {batch_number} sent to Aligned");

        Ok(())
    }

    /// Performs a call to aligned SDK estimate_fee function with retries over all RPC URLs.
    async fn estimate_fee(&mut self) -> Result<ethers::types::U256, ProofSenderError> {
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
        &mut self,
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

        let send_verify_tx_result = send_verify_tx(
            calldata,
            &self.eth_client,
            self.on_chain_proposer_address,
            &self.signer,
        )
        .await;

        if let Err(EthClientError::EstimateGasError(EstimateGasError::RPCError(error))) =
            send_verify_tx_result.as_ref()
        {
            if error.contains("Invalid TDX proof") {
                warn!("Deleting invalid TDX proof");
                self.rollup_store
                    .delete_proof_by_batch_and_type(batch_number, ProverType::TDX)
                    .await?;
            } else if error.contains("Invalid RISC0 proof") {
                warn!("Deleting invalid RISC0 proof");
                self.rollup_store
                    .delete_proof_by_batch_and_type(batch_number, ProverType::RISC0)
                    .await?;
            } else if error.contains("Invalid SP1 proof") {
                warn!("Deleting invalid SP1 proof");
                self.rollup_store
                    .delete_proof_by_batch_and_type(batch_number, ProverType::SP1)
                    .await?;
            }
        }

        let verify_tx_hash = send_verify_tx_result?;

        metrics!(
            let verify_tx_receipt = self
                .eth_client
                .get_transaction_receipt(verify_tx_hash)
                .await?
                .ok_or(ProofSenderError::UnexpectedError("no verify tx receipt".to_string()))?;
            let verify_gas_used = verify_tx_receipt.tx_info.gas_used.try_into()?;
            METRICS.set_batch_verification_gas(batch_number, verify_gas_used)?;
        );

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

    async fn health(&self) -> CallResponse<Self> {
        let rpc_healthcheck = self.eth_client.test_urls().await;
        let signer_status = self.signer.health().await;

        let (fee_estimate, aligned_sp1_elf_path) =
            if self.needed_proof_types.contains(&ProverType::Aligned) {
                (
                    Some(self.fee_estimate.clone()),
                    Some(self.aligned_sp1_elf_path.clone()),
                )
            } else {
                (None, None)
            };
        CallResponse::Reply(OutMessage::Health(Box::new(L1ProofSenderHealth {
            rpc_healthcheck,
            signer_status,
            on_chain_proposer_address: self.on_chain_proposer_address,
            needed_proof_types: self
                .needed_proof_types
                .iter()
                .map(|proof_type| format!("{:?}", proof_type))
                .collect(),
            proof_send_interval_ms: self.proof_send_interval_ms,
            sequencer_state: format!("{:?}", self.sequencer_state.status().await),
            l1_chain_id: self.l1_chain_id,
            network: format!("{:?}", self.network),
            fee_estimate,
            aligned_sp1_elf_path,
        })))
    }
}

impl GenServer for L1ProofSender {
    type CallMsg = CallMessage;
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

    async fn handle_call(
        &mut self,
        message: Self::CallMsg,
        _handle: &GenServerHandle<Self>,
    ) -> CallResponse<Self> {
        match message {
            CallMessage::Health => self.health().await,
        }
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
