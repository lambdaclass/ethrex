use std::collections::HashMap;

use bytes::Bytes;
use ethrex_common::{Address, H160, H256, U256};
use ethrex_l2_sdk::calldata::{encode_calldata, Value};
use ethrex_rpc::{
    clients::{eth::WrappedTransaction, Overrides},
    EthClient,
};
use keccak_hash::keccak;
use secp256k1::SecretKey;
use spawned_concurrency::{send_after, CallResponse, CastResponse, GenServer, GenServerInMsg};
use spawned_rt::mpsc::Sender;
use std::str::FromStr;
use tracing::{debug, error, info};

use crate::{
    sequencer::errors::ProofSenderError,
    utils::prover::{
        proving_systems::ProverType,
        save_state::{batch_number_has_all_needed_proofs, read_proof, StateFileType},
    },
    CommitterConfig, EthConfig, ProofCoordinatorConfig, SequencerConfig,
};

use super::{errors::SequencerError, utils::random_duration};

const VERIFY_FUNCTION_SIGNATURE: &str =
    "verifyBatch(uint256,bytes,bytes32,bytes,bytes,bytes,bytes32,bytes,uint256[8],bytes,bytes)";

const DEV_MODE_ADDRESS: H160 = H160([
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0xAA,
]);

#[derive(Clone)]
pub struct L1ProofSenderState {
    eth_client: EthClient,
    l1_address: Address,
    l1_private_key: SecretKey,
    on_chain_proposer_address: Address,
    needed_proof_types: Vec<ProverType>,
    proof_send_interval_ms: u64,
}

impl L1ProofSenderState {
    async fn new(
        cfg: &ProofCoordinatorConfig,
        committer_cfg: &CommitterConfig,
        eth_cfg: &EthConfig,
    ) -> Result<Self, ProofSenderError> {
        let eth_client = EthClient::new_with_multiple_urls(eth_cfg.rpc_url.clone())?;

        if cfg.dev_mode {
            return Ok(Self {
                eth_client,
                l1_address: cfg.l1_address,
                l1_private_key: cfg.l1_private_key,
                on_chain_proposer_address: committer_cfg.on_chain_proposer_address,
                needed_proof_types: vec![ProverType::Exec],
                proof_send_interval_ms: cfg.proof_send_interval_ms,
            });
        }

        let mut needed_proof_types = vec![];
        for prover_type in ProverType::all() {
            let Some(getter) = prover_type.verifier_getter() else {
                continue;
            };
            let calldata = Bytes::copy_from_slice(keccak(getter)[..4].as_ref());
            let response = eth_client
                .call(
                    committer_cfg.on_chain_proposer_address,
                    calldata,
                    Overrides::default(),
                )
                .await?;
            // trim to 20 bytes, also removes 0x prefix
            let trimmed_response = &response[26..];

            let address = Address::from_str(&format!("0x{trimmed_response}"))
                .map_err(|_| ProofSenderError::FailedToParseOnChainProposerResponse(response))?;

            if address != DEV_MODE_ADDRESS {
                info!("{prover_type} proof needed");
                needed_proof_types.push(prover_type);
            }
        }

        Ok(Self {
            eth_client,
            l1_address: cfg.l1_address,
            l1_private_key: cfg.l1_private_key,
            on_chain_proposer_address: committer_cfg.on_chain_proposer_address,
            needed_proof_types,
            proof_send_interval_ms: cfg.proof_send_interval_ms,
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
    pub async fn spawn(cfg: SequencerConfig) -> Result<(), ProofSenderError> {
        let state =
            L1ProofSenderState::new(&cfg.proof_coordinator, &cfg.l1_committer, &cfg.eth).await?;
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

    type Error = SequencerError;

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
        let _ = verify_and_send_proof(state)
            .await
            .inspect_err(|err| error!("L1 Proof Sender: {err}"));
        let check_interval = random_duration(state.proof_send_interval_ms);
        send_after(check_interval, tx.clone(), Self::InMsg::Send);
        CastResponse::NoReply
    }
}

async fn verify_and_send_proof(state: &L1ProofSenderState) -> Result<(), ProofSenderError> {
    let batch_to_verify = 1 + state
        .eth_client
        .get_last_verified_batch(state.on_chain_proposer_address)
        .await?;

    if batch_number_has_all_needed_proofs(batch_to_verify, &state.needed_proof_types)
        .inspect_err(|_| info!("Missing proofs for batch {batch_to_verify}, skipping sending"))
        .unwrap_or_default()
    {
        send_proof(state, batch_to_verify).await?;
    }

    Ok(())
}

pub async fn send_proof(
    state: &L1ProofSenderState,
    batch_number: u64,
) -> Result<H256, ProofSenderError> {
    // TODO: change error
    // TODO: If the proof is not needed, a default calldata is used,
    // the structure has to match the one defined in the OnChainProposer.sol contract.
    // It may cause some issues, but the ethrex_prover_lib cannot be imported,
    // this approach is straight-forward for now.
    let mut proofs = HashMap::with_capacity(state.needed_proof_types.len());
    for prover_type in state.needed_proof_types.iter() {
        let proof = read_proof(batch_number, StateFileType::Proof(*prover_type))?;
        if proof.prover_type != *prover_type {
            return Err(ProofSenderError::ProofNotPresent(*prover_type));
        }
        proofs.insert(prover_type, proof.calldata);
    }

    debug!("Sending proof for batch number: {batch_number}");

    let calldata_values = [
        &[Value::Uint(U256::from(batch_number))],
        proofs
            .get(&ProverType::RISC0)
            .unwrap_or(&ProverType::RISC0.empty_calldata())
            .as_slice(),
        proofs
            .get(&ProverType::SP1)
            .unwrap_or(&ProverType::SP1.empty_calldata())
            .as_slice(),
        proofs
            .get(&ProverType::Pico)
            .unwrap_or(&ProverType::Pico.empty_calldata())
            .as_slice(),
        proofs
            .get(&ProverType::TDX)
            .unwrap_or(&ProverType::TDX.empty_calldata())
            .as_slice(),
    ]
    .concat();

    let calldata = encode_calldata(VERIFY_FUNCTION_SIGNATURE, &calldata_values)?;

    let gas_price = state
        .eth_client
        .get_gas_price_with_extra(20)
        .await?
        .try_into()
        .map_err(|_| {
            ProofSenderError::InternalError("Failed to convert gas_price to a u64".to_owned())
        })?;

    let verify_tx = state
        .eth_client
        .build_eip1559_transaction(
            state.on_chain_proposer_address,
            state.l1_address,
            calldata.into(),
            Overrides {
                max_fee_per_gas: Some(gas_price),
                max_priority_fee_per_gas: Some(gas_price),
                ..Default::default()
            },
        )
        .await?;

    let mut tx = WrappedTransaction::EIP1559(verify_tx);

    let verify_tx_hash = state
        .eth_client
        .send_tx_bump_gas_exponential_backoff(&mut tx, &state.l1_private_key)
        .await?;

    info!("Sent proof for batch {batch_number}, with transaction hash {verify_tx_hash:#x}");

    Ok(verify_tx_hash)
}
