use std::collections::HashMap;

use ethrex_common::{Address, U256};
use ethrex_l2_common::{
    calldata::Value,
    prover::{BatchProof, ProverType},
};
use ethrex_l2_sdk::calldata::encode_calldata;
use ethrex_rpc::EthClient;
use ethrex_storage_rollup::StoreRollup;
use secp256k1::SecretKey;
use spawned_concurrency::{CallResponse, CastResponse, GenServer, GenServerInMsg, send_after};
use spawned_rt::mpsc::Sender;
use tracing::{debug, error, info};

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
    common::types::{FeeEstimationType, Network, ProvingSystemId, VerificationData},
    verification_layer::{estimate_fee, get_nonce_from_batcher, submit},
};

// TODO: Remove this import once it's no longer required by the SDK.
use ethers::{
    signers::{Signer, Wallet},
    types::H256,
};

const VERIFY_FUNCTION_SIGNATURE: &str =
    "verifyBatch(uint256,bytes32,bytes,bytes32,bytes,bytes,bytes,bytes,bytes)";

#[derive(Clone)]
pub struct L1ProofSenderState {
    eth_client: EthClient,
    l1_address: Address,
    l1_private_key: SecretKey,
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

impl L1ProofSenderState {
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
            ProofSenderError::InternalError("Failed to convert chain ID to U256".to_owned())
        })?;
        let fee_estimate = resolve_fee_estimate(&aligned_cfg.fee_estimate)?;
        let aligned_sp1_elf_path = aligned_cfg.aligned_sp1_elf_path.clone();

        if cfg.dev_mode {
            return Ok(Self {
                eth_client,
                l1_address: cfg.l1_address,
                l1_private_key: cfg.l1_private_key,
                on_chain_proposer_address: committer_cfg.on_chain_proposer_address,
                needed_proof_types: vec![ProverType::Exec],
                proof_send_interval_ms: cfg.proof_send_interval_ms,
                sequencer_state,
                rollup_store,
                l1_chain_id,
                network: aligned_cfg.network.clone(),
                fee_estimate,
                aligned_sp1_elf_path,
            });
        }

        Ok(Self {
            eth_client,
            l1_address: cfg.l1_address,
            l1_private_key: cfg.l1_private_key,
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
}

#[derive(Clone)]
pub enum InMessage {
    Send,
}

#[derive(Clone, PartialEq)]
pub enum OutMessage {
    Done,
}

pub struct L1ProofSender;

impl L1ProofSender {
    pub async fn spawn(
        cfg: SequencerConfig,
        sequencer_state: SequencerState,
        rollup_store: StoreRollup,
        needed_proof_types: Vec<ProverType>,
    ) -> Result<(), ProofSenderError> {
        let state = L1ProofSenderState::new(
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
            .map_err(ProofSenderError::GenServerError)
    }
}

impl GenServer for L1ProofSender {
    type InMsg = InMessage;
    type OutMsg = OutMessage;
    type State = L1ProofSenderState;

    type Error = ProofSenderError;

    fn new() -> Self {
        Self {}
    }

    async fn handle_call(
        &mut self,
        _message: Self::InMsg,
        _tx: &Sender<GenServerInMsg<Self>>,
        _state: &mut Self::State,
    ) -> CallResponse<Self::OutMsg> {
        CallResponse::Reply(OutMessage::Done)
    }

    async fn handle_cast(
        &mut self,
        _message: Self::InMsg,
        tx: &Sender<GenServerInMsg<Self>>,
        state: &mut Self::State,
    ) -> CastResponse {
        // Right now we only have the Send message, so we ignore the message
        if let SequencerStatus::Sequencing = state.sequencer_state.status().await {
            let _ = verify_and_send_proof(state)
                .await
                .inspect_err(|err| error!("L1 Proof Sender: {err}"));
        }
        let check_interval = random_duration(state.proof_send_interval_ms);
        send_after(check_interval, tx.clone(), Self::InMsg::Send);
        CastResponse::NoReply
    }
}

async fn verify_and_send_proof(state: &L1ProofSenderState) -> Result<(), ProofSenderError> {
    let batch_to_send = 1 + get_latest_sent_batch(
        state.needed_proof_types.clone(),
        &state.rollup_store,
        &state.eth_client,
        state.on_chain_proposer_address,
    )
    .await
    .map_err(|err| {
        error!("Failed to get next batch to send: {err}");
        ProofSenderError::InternalError(err.to_string())
    })?;

    let last_committed_batch = state
        .eth_client
        .get_last_committed_batch(state.on_chain_proposer_address)
        .await?;

    if last_committed_batch < batch_to_send {
        info!("Next batch to send ({batch_to_send}) is not yet committed");
        return Ok(());
    }

    let last_block_hash = state.rollup_store.get_block_hash_by_batch(batch_to_send);

    let mut proofs = HashMap::new();
    let mut missing_proof_types = Vec::new();
    for proof_type in &state.needed_proof_types {
        if let Some(proof) = state
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
            send_proof_to_aligned(state, batch_to_send, aligned_proof).await?;
        } else {
            send_proof_to_contract(state, batch_to_send, last_block_hash, proofs).await?;
        }
        state
            .rollup_store
            .set_lastest_sent_batch_proof(batch_to_send)
            .await?;
    } else {
        let missing_proof_types: Vec<String> = missing_proof_types
            .iter()
            .map(|proof_type| format!("{proof_type:?}"))
            .collect();
        info!(
            "Missing {} batch proof(s), will not send",
            missing_proof_types.join(", ")
        );
    }

    Ok(())
}

async fn send_proof_to_aligned(
    state: &L1ProofSenderState,
    batch_number: u64,
    aligned_proof: BatchProof,
) -> Result<(), ProofSenderError> {
    let elf = std::fs::read(state.aligned_sp1_elf_path.clone())
        .map_err(|e| ProofSenderError::InternalError(format!("Failed to read ELF file: {e}")))?;

    let verification_data = VerificationData {
        proving_system: ProvingSystemId::SP1,
        proof: aligned_proof.proof(),
        proof_generator_addr: state.l1_address.0.into(),
        vm_program_code: Some(elf),
        verification_key: None,
        pub_input: None,
    };

    let rpc_url = state
        .eth_client
        .urls
        .first()
        .ok_or_else(|| {
            ProofSenderError::InternalError("No Ethereum RPC URL configured".to_owned())
        })?
        .as_str();

    let fee_estimation = estimate_fee(rpc_url, state.fee_estimate.clone())
        .await
        .map_err(|err| ProofSenderError::AlignedFeeEstimateError(err.to_string()))?;

    let nonce = get_nonce_from_batcher(state.network.clone(), state.l1_address.0.into())
        .await
        .map_err(|err| {
            ProofSenderError::AlignedGetNonceError(format!("Failed to get nonce: {err:?}"))
        })?;

    let wallet = Wallet::from_bytes(state.l1_private_key.as_ref())
        .map_err(|_| ProofSenderError::InternalError("Failed to create wallet".to_owned()))?;

    let wallet = wallet.with_chain_id(state.l1_chain_id);

    debug!("Sending proof to Aligned");

    submit(
        state.network.clone(),
        &verification_data,
        fee_estimation,
        wallet,
        nonce,
    )
    .await
    .map_err(|err| {
        ProofSenderError::AlignedSubmitProofError(format!("Failed to submit proof: {err}"))
    })?;

    info!("Proof for batch {batch_number} sent to Aligned");

    Ok(())
}

pub async fn send_proof_to_contract(
    state: &L1ProofSenderState,
    batch_number: u64,
    last_block_hash: H256,
    proofs: HashMap<ProverType, BatchProof>,
) -> Result<(), ProofSenderError> {
    info!(
        ?batch_number,
        "Sending batch verification transaction to L1"
    );

    let calldata_values = [
        &[Value::Uint(U256::from(batch_number))],
        &[Value::FixedBytes(last_block_hash.0.to_vec().into())],
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
        &state.eth_client,
        state.on_chain_proposer_address,
        state.l1_address,
        &state.l1_private_key,
    )
    .await?;

    info!(
        ?batch_number,
        ?verify_tx_hash,
        "Sent batch verification transaction to L1"
    );

    Ok(())
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
