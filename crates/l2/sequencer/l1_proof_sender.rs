use std::{
    collections::{BTreeMap, HashMap},
    fs::remove_dir_all,
    path::PathBuf,
};

use aligned_sdk::gateway::provider::AggregationModeGatewayProvider;
#[cfg(feature = "sp1")]
use aligned_sdk::gateway::provider::GatewayError;
use aligned_sdk::types::Network;
use alloy::signers::local::PrivateKeySigner;
use ethrex_common::{Address, U256};
use ethrex_l2_common::{
    calldata::Value,
    prover::{BatchProof, ProverType},
};
use ethrex_l2_rpc::signer::{Signer, SignerHealth};
use ethrex_l2_sdk::{calldata::encode_calldata, get_last_committed_batch, get_last_verified_batch};
#[cfg(feature = "metrics")]
use ethrex_metrics::l2::metrics::METRICS;
use ethrex_metrics::metrics;
use ethrex_rpc::{
    EthClient,
    clients::{EthClientError, eth::errors::RpcRequestError},
};
use ethrex_storage_rollup::StoreRollup;
use serde::Serialize;
use spawned_concurrency::tasks::{
    CallResponse, CastResponse, GenServer, GenServerHandle, send_after,
};
use tracing::{error, info, warn};

use super::{
    configs::AlignedConfig,
    utils::{random_duration, send_verify_tx},
};

use crate::{
    CommitterConfig, EthConfig, ProofCoordinatorConfig, SequencerConfig,
    sequencer::{
        errors::ProofSenderError,
        sequencer_state::{SequencerState, SequencerStatus},
        utils::batch_checkpoint_name,
    },
};

#[cfg(feature = "sp1")]
use ethrex_guest_program::ZKVM_SP1_PROGRAM_ELF;
#[cfg(feature = "sp1")]
use sp1_sdk::{HashableKey, Prover, SP1ProofWithPublicValues, SP1VerifyingKey};

const VERIFY_FUNCTION_SIGNATURE: &str = "verifyBatch(uint256,bytes,bytes,bytes)";
const VERIFY_BATCHES_FUNCTION_SIGNATURE: &str = "verifyBatches(uint256,bytes[],bytes[],bytes[])";

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
    timelock_address: Option<Address>,
    needed_proof_types: Vec<ProverType>,
    proof_send_interval_ms: u64,
    sequencer_state: SequencerState,
    rollup_store: StoreRollup,
    l1_chain_id: u64,
    network: Network,
    /// Directory where checkpoints are stored.
    checkpoints_dir: PathBuf,
    aligned_mode: bool,
    /// Cached SP1 verifying key for aligned mode
    #[cfg(feature = "sp1")]
    sp1_vk: Option<SP1VerifyingKey>,
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
}

impl L1ProofSender {
    #[expect(clippy::too_many_arguments)]
    async fn new(
        cfg: &ProofCoordinatorConfig,
        committer_cfg: &CommitterConfig,
        eth_cfg: &EthConfig,
        sequencer_state: SequencerState,
        aligned_cfg: &AlignedConfig,
        rollup_store: StoreRollup,
        needed_proof_types: Vec<ProverType>,
        checkpoints_dir: PathBuf,
    ) -> Result<Self, ProofSenderError> {
        let eth_client = EthClient::new_with_config(
            eth_cfg.rpc_url.clone(),
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

        // Initialize SP1 verifying key if in aligned mode with sp1 feature
        #[cfg(feature = "sp1")]
        let sp1_vk = if aligned_cfg.aligned_mode {
            Some(Self::init_sp1_vk()?)
        } else {
            None
        };

        Ok(Self {
            eth_client,
            signer: cfg.signer.clone(),
            on_chain_proposer_address: committer_cfg.on_chain_proposer_address,
            timelock_address: committer_cfg.timelock_address,
            needed_proof_types,
            proof_send_interval_ms: cfg.proof_send_interval_ms,
            sequencer_state,
            rollup_store,
            l1_chain_id,
            network: aligned_cfg.network.clone(),
            checkpoints_dir,
            aligned_mode: aligned_cfg.aligned_mode,
            #[cfg(feature = "sp1")]
            sp1_vk,
        })
    }

    #[cfg(feature = "sp1")]
    fn init_sp1_vk() -> Result<SP1VerifyingKey, ProofSenderError> {
        // Setup the prover client to get the verifying key
        let client = sp1_sdk::CpuProver::new();
        let (_pk, vk) = client.setup(ZKVM_SP1_PROGRAM_ELF);
        info!("Initialized SP1 verifying key: {}", vk.bytes32());
        Ok(vk)
    }

    pub async fn spawn(
        cfg: SequencerConfig,
        sequencer_state: SequencerState,
        rollup_store: StoreRollup,
        needed_proof_types: Vec<ProverType>,
        checkpoints_dir: PathBuf,
    ) -> Result<GenServerHandle<L1ProofSender>, ProofSenderError> {
        let state = Self::new(
            &cfg.proof_coordinator,
            &cfg.l1_committer,
            &cfg.eth,
            sequencer_state,
            &cfg.aligned,
            rollup_store,
            needed_proof_types,
            checkpoints_dir,
        )
        .await?;
        let mut l1_proof_sender = L1ProofSender::start(state);
        l1_proof_sender
            .cast(InMessage::Send)
            .await
            .map_err(ProofSenderError::InternalError)?;
        Ok(l1_proof_sender)
    }

    async fn verify_and_send_proof(&self) -> Result<(), ProofSenderError> {
        let last_verified_batch =
            get_last_verified_batch(&self.eth_client, self.on_chain_proposer_address).await?;
        let latest_sent_batch_db = self.rollup_store.get_latest_sent_batch_proof().await?;

        if self.aligned_mode {
            let batch_to_send = std::cmp::max(latest_sent_batch_db, last_verified_batch) + 1;
            return self.verify_and_send_proof_aligned(batch_to_send).await;
        }

        // Non-aligned path: preserve hotfix for DB < on-chain
        if latest_sent_batch_db < last_verified_batch {
            self.rollup_store
                .set_latest_sent_batch_proof(last_verified_batch)
                .await?;
        }

        let first_batch = last_verified_batch + 1;

        let last_committed_batch =
            get_last_committed_batch(&self.eth_client, self.on_chain_proposer_address).await?;

        if last_committed_batch < first_batch {
            info!("Next batch to send ({first_batch}) is not yet committed");
            return Ok(());
        }

        // Collect consecutive proven batches starting from first_batch
        let mut ready_batches: Vec<(u64, HashMap<ProverType, BatchProof>)> = Vec::new();
        for batch in first_batch..=last_committed_batch {
            let mut proofs = HashMap::new();
            let mut all_present = true;
            for proof_type in &self.needed_proof_types {
                if let Some(proof) = self
                    .rollup_store
                    .get_proof_by_batch_and_type(batch, *proof_type)
                    .await?
                {
                    proofs.insert(*proof_type, proof);
                } else {
                    all_present = false;
                    break;
                }
            }
            if !all_present {
                break;
            }
            ready_batches.push((batch, proofs));
        }

        if ready_batches.is_empty() {
            info!(
                ?first_batch,
                "No consecutive batches ready to send starting from first_batch"
            );
            return Ok(());
        }

        // safe: we checked non-empty above
        let last_batch_sent = match ready_batches.last() {
            Some((batch, _)) => *batch,
            None => return Ok(()),
        };

        if ready_batches.len() == 1 {
            // Single batch: use existing send_proof_to_contract for backward compat
            let Some((batch_number, proofs)) = ready_batches.into_iter().next() else {
                return Ok(());
            };
            self.send_proof_to_contract(batch_number, proofs).await?;
        } else {
            // Multiple batches: use verifyBatches
            self.send_batches_proof_to_contract(first_batch, &ready_batches)
                .await?;
        }

        self.rollup_store
            .set_latest_sent_batch_proof(last_batch_sent)
            .await?;

        // Clean up checkpoints for each batch in range
        for batch in first_batch..=last_batch_sent {
            let checkpoint_path = self.checkpoints_dir.join(batch_checkpoint_name(batch - 1));
            if checkpoint_path.exists() {
                let _ = remove_dir_all(&checkpoint_path).inspect_err(|e| {
                    error!(
                        "Failed to remove checkpoint directory at path {checkpoint_path:?}. Should be removed manually. Error: {e}"
                    )
                });
            }
        }

        Ok(())
    }

    async fn verify_and_send_proof_aligned(
        &self,
        batch_to_send: u64,
    ) -> Result<(), ProofSenderError> {
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
            self.send_proof_to_aligned(batch_to_send, proofs.values())
                .await?;
            self.rollup_store
                .set_latest_sent_batch_proof(batch_to_send)
                .await?;

            let checkpoint_path = self
                .checkpoints_dir
                .join(batch_checkpoint_name(batch_to_send - 1));
            if checkpoint_path.exists() {
                let _ = remove_dir_all(&checkpoint_path).inspect_err(|e| {
                    error!(
                        "Failed to remove checkpoint directory at path {checkpoint_path:?}. Should be removed manually. Error: {e}"
                    )
                });
            }
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

        let Signer::Local(local_signer) = &self.signer else {
            return Err(ProofSenderError::UnexpectedError(
                "Aligned mode only supports local signer".to_string(),
            ));
        };

        // Create alloy signer from private key
        // Convert secp256k1::SecretKey to FixedBytes<32> for alloy signer
        let private_key_bytes: [u8; 32] = local_signer.private_key.secret_bytes();
        let signer = PrivateKeySigner::from_bytes(&private_key_bytes.into()).map_err(|e| {
            ProofSenderError::UnexpectedError(format!("Failed to create signer: {e}"))
        })?;

        let sender_address = format!("{:?}", self.signer.address());

        // Create the gateway provider with signer
        let gateway = AggregationModeGatewayProvider::new_with_signer(self.network.clone(), signer)
            .map_err(|e| {
                ProofSenderError::UnexpectedError(format!("Failed to create gateway: {e:?}"))
            })?;

        for batch_proof in batch_proofs {
            let prover_type = batch_proof.prover_type();

            match prover_type {
                ProverType::SP1 => {
                    self.submit_sp1_proof_to_aligned(
                        &gateway,
                        &sender_address,
                        batch_number,
                        batch_proof,
                    )
                    .await?;
                }
                // Future: Add risc0, zisk, etc. support here
                _ => {
                    warn!(
                        ?prover_type,
                        "Prover type not yet supported for Aligned, skipping"
                    );
                    return Err(ProofSenderError::AlignedUnsupportedProverType(
                        prover_type.to_string(),
                    ));
                }
            }
        }

        Ok(())
    }

    #[cfg(feature = "sp1")]
    async fn submit_sp1_proof_to_aligned(
        &self,
        gateway: &AggregationModeGatewayProvider<PrivateKeySigner>,
        sender_address: &str,
        batch_number: u64,
        batch_proof: &BatchProof,
    ) -> Result<(), ProofSenderError> {
        let prover_type = batch_proof.prover_type();

        let sp1_vk = self.sp1_vk.as_ref().ok_or_else(|| {
            ProofSenderError::UnexpectedError("SP1 verifying key not initialized".to_string())
        })?;

        let Some(proof_bytes) = batch_proof.compressed() else {
            return Err(ProofSenderError::AlignedWrongProofFormat);
        };

        // Deserialize the proof from bincode format
        let proof: SP1ProofWithPublicValues = bincode::deserialize(&proof_bytes).map_err(|e| {
            ProofSenderError::UnexpectedError(format!("Failed to deserialize SP1 proof: {e}"))
        })?;

        // Get the nonce that will be used for this submission
        let nonce = gateway
            .get_nonce_for(sender_address.to_string())
            .await
            .map_err(|e| ProofSenderError::AlignedGetNonceError(format!("{e:?}")))?
            .data
            .nonce;

        info!(
            ?prover_type,
            ?batch_number,
            ?nonce,
            "Submitting proof to Aligned"
        );

        let result = gateway.submit_sp1_proof(&proof, sp1_vk).await;

        match result {
            Ok(response) => {
                info!(
                    ?batch_number,
                    ?nonce,
                    task_id = ?response.data.task_id,
                    "Submitted proof to Aligned"
                );
            }
            Err(GatewayError::Api { status, message }) if message.contains("invalid") => {
                warn!("Proof is invalid, will be deleted: {message}");
                self.rollup_store
                    .delete_proof_by_batch_and_type(batch_number, prover_type)
                    .await?;
                return Err(ProofSenderError::AlignedSubmitProofError(
                    GatewayError::Api { status, message },
                ));
            }
            Err(e) => {
                return Err(ProofSenderError::AlignedSubmitProofError(e));
            }
        }

        Ok(())
    }

    #[cfg(not(feature = "sp1"))]
    async fn submit_sp1_proof_to_aligned(
        &self,
        _gateway: &AggregationModeGatewayProvider<PrivateKeySigner>,
        _sender_address: &str,
        _batch_number: u64,
        _batch_proof: &BatchProof,
    ) -> Result<(), ProofSenderError> {
        Err(ProofSenderError::UnexpectedError(
            "SP1 proofs require the 'sp1' feature to be enabled".to_string(),
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

        // Based won't have timelock address until we implement it on it. For the meantime if it's None (only happens in based) we use the OCP
        let target_address = self
            .timelock_address
            .unwrap_or(self.on_chain_proposer_address);

        let send_verify_tx_result =
            send_verify_tx(calldata, &self.eth_client, target_address, &self.signer).await;

        if let Err(EthClientError::RpcRequestError(RpcRequestError::RPCError { message, .. })) =
            send_verify_tx_result.as_ref()
        {
            if message.contains("Invalid TDX proof") {
                warn!("Deleting invalid TDX proof");
                self.rollup_store
                    .delete_proof_by_batch_and_type(batch_number, ProverType::TDX)
                    .await?;
            } else if message.contains("Invalid RISC0 proof") {
                warn!("Deleting invalid RISC0 proof");
                self.rollup_store
                    .delete_proof_by_batch_and_type(batch_number, ProverType::RISC0)
                    .await?;
            } else if message.contains("Invalid SP1 proof") {
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

    /// Sends multiple consecutive batch proofs in a single verifyBatches transaction.
    /// On revert with an invalid proof message, falls back to single-batch sending
    /// to identify which batch has the bad proof.
    pub async fn send_batches_proof_to_contract(
        &self,
        first_batch: u64,
        batches: &[(u64, HashMap<ProverType, BatchProof>)],
    ) -> Result<(), ProofSenderError> {
        let batch_count = batches.len();
        info!(
            first_batch,
            batch_count, "Sending multi-batch verification transaction to L1"
        );

        // Build three Value::Array of Value::Bytes for risc0, sp1, tdx
        let mut risc0_array = Vec::with_capacity(batch_count);
        let mut sp1_array = Vec::with_capacity(batch_count);
        let mut tdx_array = Vec::with_capacity(batch_count);

        for (_batch_number, proofs) in batches {
            // Each proof type's calldata is a Vec<Value> with one element: Value::Bytes.
            // For missing proofs, use empty bytes.
            let risc0_bytes = proofs
                .get(&ProverType::RISC0)
                .map(|proof| proof.calldata())
                .unwrap_or(ProverType::RISC0.empty_calldata())
                .into_iter()
                .next()
                .unwrap_or(Value::Bytes(vec![].into()));
            risc0_array.push(risc0_bytes);

            let sp1_bytes = proofs
                .get(&ProverType::SP1)
                .map(|proof| proof.calldata())
                .unwrap_or(ProverType::SP1.empty_calldata())
                .into_iter()
                .next()
                .unwrap_or(Value::Bytes(vec![].into()));
            sp1_array.push(sp1_bytes);

            let tdx_bytes = proofs
                .get(&ProverType::TDX)
                .map(|proof| proof.calldata())
                .unwrap_or(ProverType::TDX.empty_calldata())
                .into_iter()
                .next()
                .unwrap_or(Value::Bytes(vec![].into()));
            tdx_array.push(tdx_bytes);
        }

        let calldata_values = vec![
            Value::Uint(U256::from(first_batch)),
            Value::Array(risc0_array),
            Value::Array(sp1_array),
            Value::Array(tdx_array),
        ];

        let calldata = encode_calldata(VERIFY_BATCHES_FUNCTION_SIGNATURE, &calldata_values)?;

        let target_address = self
            .timelock_address
            .unwrap_or(self.on_chain_proposer_address);

        let send_verify_tx_result =
            send_verify_tx(calldata, &self.eth_client, target_address, &self.signer).await;

        // On revert with invalid proof message, fall back to single-batch sending
        // to identify which batch has the bad proof.
        if let Err(EthClientError::RpcRequestError(RpcRequestError::RPCError {
            ref message, ..
        })) = send_verify_tx_result
            && (message.contains("Invalid TDX proof")
                || message.contains("Invalid RISC0 proof")
                || message.contains("Invalid SP1 proof"))
        {
            warn!(
                "Multi-batch verify reverted with invalid proof, falling back to single-batch sending"
            );
            for (batch_number, proofs) in batches {
                self.send_proof_to_contract(*batch_number, proofs.clone())
                    .await?;
            }
            return Ok(());
        }

        let verify_tx_hash = send_verify_tx_result?;

        // Store verify tx hash for all batches
        for (batch_number, _) in batches {
            self.rollup_store
                .store_verify_tx_by_batch(*batch_number, verify_tx_hash)
                .await?;
        }

        info!(
            first_batch,
            batch_count,
            ?verify_tx_hash,
            "Sent multi-batch verification transaction to L1"
        );

        Ok(())
    }

    async fn health(&self) -> CallResponse<Self> {
        let rpc_healthcheck = self.eth_client.test_urls().await;
        let signer_status = self.signer.health().await;

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
