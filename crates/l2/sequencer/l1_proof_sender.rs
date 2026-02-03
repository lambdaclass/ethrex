use std::{
    collections::{BTreeMap, HashMap},
    fs::remove_dir_all,
    path::PathBuf,
};

use ethrex_common::{Address, U256};
use ethrex_guest_program::{ZKVM_RISC0_PROGRAM_VK, ZKVM_SP1_PROGRAM_ELF};
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
use aligned_sdk::{
    common::{
        errors,
        types::{FeeEstimationType, Network, ProvingSystemId, VerificationData},
    },
    verification_layer::{estimate_fee as aligned_estimate_fee, get_nonce_from_batcher, submit},
};

use ethers::signers::{Signer as EthersSigner, Wallet};

const VERIFY_FUNCTION_SIGNATURE: &str = "verifyBatch(uint256,bytes,bytes,bytes)";

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
    fee_estimate: FeeEstimationType,
    /// Directory where checkpoints are stored.
    checkpoints_dir: PathBuf,
    aligned_mode: bool,
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
        let fee_estimate = resolve_fee_estimate(&aligned_cfg.fee_estimate)?;

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
            fee_estimate,
            checkpoints_dir,
            aligned_mode: aligned_cfg.aligned_mode,
        })
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
        let batch_to_send = if self.aligned_mode {
            std::cmp::max(latest_sent_batch_db, last_verified_batch) + 1
        } else {
            if latest_sent_batch_db < last_verified_batch {
                // hotfix: in case the latest sent batch in DB is less than the last verified on-chain,
                // we update the db to avoid stalling the proof_coordinator.
                self.rollup_store
                    .set_latest_sent_batch_proof(last_verified_batch)
                    .await?;
            }
            last_verified_batch + 1
        };

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
            if self.aligned_mode {
                self.send_proof_to_aligned(batch_to_send, proofs.values())
                    .await?;
            } else {
                self.send_proof_to_contract(batch_to_send, proofs).await?;
            }
            self.rollup_store
                .set_latest_sent_batch_proof(batch_to_send)
                .await?;

            // Remove checkpoint from batch sent - 1.
            // That checkpoint was needed to generate the proof for the batch we just sent.
            // The checkpoint for the batch we have just sent is needed for the next batch.
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

            let Some(proof) = batch_proof.compressed() else {
                return Err(ProofSenderError::AlignedWrongProofFormat);
            };

            let vm_program_code = match prover_type {
                ProverType::RISC0 => {
                    if !cfg!(feature = "risc0") {
                        return Err(ProofSenderError::UnexpectedError(
                            "Trying to send RISC0 proof but RISC0 feature is disabled".to_string(),
                        ));
                    }

                    let trimmed = ZKVM_RISC0_PROGRAM_VK.trim_start_matches("0x").trim();
                    hex::decode(trimmed).map_err(|e| {
                        std::io::Error::new(std::io::ErrorKind::InvalidData, format!("{e}"))
                    })?
                }
                ProverType::SP1 => {
                    if !cfg!(feature = "sp1") {
                        return Err(ProofSenderError::UnexpectedError(
                            "Trying to send SP1 proof but SP1 feature is disabled".to_string(),
                        ));
                    }

                    ZKVM_SP1_PROGRAM_ELF.to_vec()
                }
                _other => {
                    return Err(ProofSenderError::UnexpectedError(format!(
                        "no vm_program_code for {prover_type}"
                    )));
                }
            };

            let pub_input = Some(batch_proof.public_values());

            let verification_data = VerificationData {
                proving_system,
                proof,
                proof_generator_addr: self.signer.address().0.into(),
                vm_program_code: Some(vm_program_code),
                verification_key: None,
                pub_input,
            };

            info!(?prover_type, ?batch_number, "Submitting proof to Aligned");
            let aligned_verification_result = submit(
                self.network.clone(),
                &verification_data,
                fee_estimation,
                wallet.clone(),
                nonce,
            )
            .await;

            if let Err(errors::SubmitError::InvalidProof(_)) = aligned_verification_result.as_ref()
            {
                warn!("Proof is invalid, will be deleted");
                self.rollup_store
                    .delete_proof_by_batch_and_type(batch_number, prover_type)
                    .await?;
            }

            aligned_verification_result?;

            nonce = nonce
                .checked_add(1.into())
                .ok_or(ProofSenderError::UnexpectedError(
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

    async fn health(&self) -> CallResponse<Self> {
        let rpc_healthcheck = self.eth_client.test_urls().await;
        let signer_status = self.signer.health().await;

        let fee_estimate = if self.aligned_mode {
            Some(self.fee_estimate.clone())
        } else {
            None
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
