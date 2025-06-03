use super::{
    configs::AlignedConfig,
    errors::SequencerError,
    utils::{get_latest_sent_batch, resolve_network, send_verify_tx, sleep_random},
};
use crate::{
    sequencer::errors::ProofSenderError,
    utils::prover::{
        proving_systems::ProverType,
        save_state::{batch_number_has_all_needed_proofs, read_proof, StateFileType},
    },
    CommitterConfig, EthConfig, ProofCoordinatorConfig, SequencerConfig,
};
use aligned_sdk::{
    common::types::{FeeEstimationType, Network, ProvingSystemId, VerificationData},
    verification_layer::{estimate_fee, get_nonce_from_batcher, submit},
};
use ethers::signers::{Signer, Wallet};
use ethrex_common::{Address, U256};
use ethrex_l2_sdk::calldata::{encode_calldata, Value};
use ethrex_rpc::EthClient;
use ethrex_storage_rollup::StoreRollup;
use secp256k1::SecretKey;
use std::collections::HashMap;
use tracing::{debug, error, info};

const VERIFY_FUNCTION_SIGNATURE: &str =
    "verifyBatch(uint256,bytes,bytes32,bytes,bytes,bytes,bytes32,bytes,uint256[8],bytes,bytes)";

pub async fn start_l1_proof_sender(
    cfg: SequencerConfig,
    rollup_store: StoreRollup,
    needed_proof_types: Vec<ProverType>,
) -> Result<(), SequencerError> {
    L1ProofSender::new(
        &cfg.proof_coordinator,
        &cfg.l1_committer,
        &cfg.eth,
        &cfg.aligned,
        rollup_store,
        needed_proof_types,
    )
    .await?
    .run()
    .await;
    Ok(())
}

struct L1ProofSender {
    eth_client: EthClient,
    l1_address: Address,
    l1_private_key: SecretKey,
    on_chain_proposer_address: Address,
    needed_proof_types: Vec<ProverType>,
    proof_send_interval_ms: u64,
    rollup_storage: StoreRollup,
    l1_chain_id: u64,
    network: Network,
    fee_estimate: FeeEstimationType,
    aligned_sp1_elf_path: String,
}

impl L1ProofSender {
    async fn new(
        cfg: &ProofCoordinatorConfig,
        committer_cfg: &CommitterConfig,
        eth_cfg: &EthConfig,
        aligned_cfg: &AlignedConfig,
        rollup_storage: StoreRollup,
        needed_proof_types: Vec<ProverType>,
    ) -> Result<Self, SequencerError> {
        let eth_client = EthClient::new_with_multiple_urls(eth_cfg.rpc_url.clone())?;
        let l1_chain_id = eth_client.get_chain_id().await?.try_into().map_err(|_| {
            ProofSenderError::InternalError("Failed to convert chain ID to U256".to_owned())
        })?;
        let network = resolve_network(&aligned_cfg.network)?;
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
                rollup_storage,
                l1_chain_id,
                network,
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
            rollup_storage,
            l1_chain_id,
            network,
            fee_estimate,
            aligned_sp1_elf_path,
        })
    }

    async fn run(&self) {
        info!("Running L1 Proof Sender");
        info!("Needed proof systems: {:?}", self.needed_proof_types);
        loop {
            let _ = self
                .main_logic()
                .await
                .inspect_err(|err| error!("L1 Proof Sender Error: {err}"));

            sleep_random(self.proof_send_interval_ms).await;
        }
    }

    async fn main_logic(&self) -> Result<(), ProofSenderError> {
        let batch_to_send = 1 + get_latest_sent_batch(
            self.needed_proof_types.clone(),
            &self.rollup_storage,
            &self.eth_client,
            self.on_chain_proposer_address,
        )
        .await
        .map_err(|err| {
            error!("Failed to get next batch to send: {}", err);
            ProofSenderError::InternalError(err.to_string())
        })?;

        let last_committed_batch = self
            .eth_client
            .get_last_committed_batch(self.on_chain_proposer_address)
            .await?;

        if last_committed_batch < batch_to_send {
            info!("Next batch to send ({batch_to_send}) is not yet committed");
            return Ok(());
        }

        if batch_number_has_all_needed_proofs(batch_to_send, &self.needed_proof_types)
            .inspect_err(|_| info!("Missing proofs for batch {batch_to_send}, skipping sending"))
            .unwrap_or_default()
        {
            self.send_proof(batch_to_send).await?;
            self.rollup_storage
                .set_lastest_sent_batch_proof(batch_to_send)
                .await?;
        }

        Ok(())
    }

    pub async fn send_proof(&self, batch_number: u64) -> Result<(), ProofSenderError> {
        if self.needed_proof_types.contains(&ProverType::Aligned) {
            return self.send_proof_to_aligned(batch_number).await;
        }
        self.send_proof_to_contract(batch_number).await
    }

    async fn send_proof_to_aligned(&self, batch_number: u64) -> Result<(), ProofSenderError> {
        let proof = read_proof(batch_number, StateFileType::BatchProof(ProverType::Aligned))?;
        let elf = std::fs::read(self.aligned_sp1_elf_path.clone()).map_err(|e| {
            ProofSenderError::InternalError(format!("Failed to read ELF file: {e}"))
        })?;

        let verification_data = VerificationData {
            proving_system: ProvingSystemId::SP1,
            proof: proof.proof(),
            proof_generator_addr: self.l1_address.0.into(),
            vm_program_code: Some(elf),
            verification_key: None,
            pub_input: None,
        };

        let rpc_url = self
            .eth_client
            .urls
            .first()
            .ok_or_else(|| {
                ProofSenderError::InternalError("No Ethereum RPC URL configured".to_owned())
            })?
            .as_str();

        let fee_estimation = estimate_fee(rpc_url, self.fee_estimate.clone())
            .await
            .map_err(|err| ProofSenderError::AlignedFeeEstimateError(err.to_string()))?;

        let nonce = get_nonce_from_batcher(self.network.clone(), self.l1_address.0.into())
            .await
            .map_err(|err| {
                ProofSenderError::AlignedGetNonceError(format!("Failed to get nonce: {:?}", err))
            })?;

        let wallet = Wallet::from_bytes(self.l1_private_key.as_ref())
            .map_err(|_| ProofSenderError::InternalError("Failed to create wallet".to_owned()))?;

        let wallet = wallet.with_chain_id(self.l1_chain_id);

        debug!("Sending proof to Aligned");

        submit(
            self.network.clone(),
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

    pub async fn send_proof_to_contract(&self, batch_number: u64) -> Result<(), ProofSenderError> {
        // TODO: change error
        // TODO: If the proof is not needed, a default calldata is used,
        // the structure has to match the one defined in the OnChainProposer.sol contract.
        // It may cause some issues, but the ethrex_prover_lib cannot be imported,
        // this approach is straight-forward for now.
        let mut proofs = HashMap::with_capacity(self.needed_proof_types.len());
        for prover_type in self.needed_proof_types.iter() {
            let proof = read_proof(batch_number, StateFileType::BatchProof(*prover_type))?;
            if proof.prover_type() != *prover_type {
                return Err(ProofSenderError::ProofNotPresent(*prover_type));
            }
            proofs.insert(prover_type, proof.calldata());
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

        let verify_tx_hash = send_verify_tx(
            calldata,
            &self.eth_client,
            self.on_chain_proposer_address,
            self.l1_address,
            &self.l1_private_key,
        )
        .await?;

        info!("Sent proof for batch {batch_number}, with transaction hash {verify_tx_hash:#x}");

        Ok(())
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
