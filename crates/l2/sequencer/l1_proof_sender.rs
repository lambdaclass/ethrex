use aligned_sdk::{
    common::types::{FeeEstimationType, Network, ProvingSystemId, VerificationData},
    verification_layer::{estimate_fee, get_nonce_from_ethereum, submit},
};
use ethers::signers::{Signer, Wallet};
use ethrex_common::{Address, U256};
use ethrex_l2_sdk::calldata::{encode_calldata, Value};
use ethrex_rpc::EthClient;
use ethrex_storage_rollup::StoreRollup;
use secp256k1::SecretKey;
use std::collections::HashMap;
use tracing::{debug, error, info};

static ELF: &[u8] = include_bytes!("../prover/zkvm/interface/sp1/out/riscv32im-succinct-zkvm-elf");

use crate::{
    sequencer::errors::ProofSenderError,
    utils::prover::{
        proving_systems::ProverType,
        save_state::{batch_number_has_all_needed_proofs, read_proof, StateFileType},
    },
    CommitterConfig, EthConfig, ProofCoordinatorConfig, SequencerConfig,
};

use super::{
    errors::SequencerError,
    utils::{get_latest_sent_batch, send_verify_tx, sleep_random},
};

const VERIFY_FUNCTION_SIGNATURE: &str =
    "verifyBatch(uint256,bytes,bytes32,bytes,bytes,bytes,bytes32,bytes,uint256[8],bytes,bytes)";

pub async fn start_l1_proof_sender(
    cfg: SequencerConfig,
    rollup_store: StoreRollup,
    needed_proof_types: Vec<ProverType>,
) -> Result<(), SequencerError> {
    let proof_sender = L1ProofSender::new(
        &cfg.proof_coordinator,
        &cfg.l1_committer,
        &cfg.eth,
        rollup_store,
        needed_proof_types,
    )
    .await?;
    proof_sender.run().await;
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
}

impl L1ProofSender {
    async fn new(
        cfg: &ProofCoordinatorConfig,
        committer_cfg: &CommitterConfig,
        eth_cfg: &EthConfig,
        rollup_storage: StoreRollup,
        needed_proof_types: Vec<ProverType>,
    ) -> Result<Self, ProofSenderError> {
        let eth_client = EthClient::new_with_multiple_urls(eth_cfg.rpc_url.clone())?;

        Ok(Self {
            eth_client,
            l1_address: cfg.l1_address,
            l1_private_key: cfg.l1_private_key,
            on_chain_proposer_address: committer_cfg.on_chain_proposer_address,
            needed_proof_types,
            proof_send_interval_ms: cfg.proof_send_interval_ms,
            rollup_storage,
        })
    }

    async fn run(&self) {
        info!("Running L1 Proof Sender");
        info!("Needed proof systems: {:?}", self.needed_proof_types);
        loop {
            if let Err(err) = self.main_logic().await {
                error!("L1 Proof Sender Error: {}", err);
            }

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

        if batch_number_has_all_needed_proofs(batch_to_send, &self.needed_proof_types)
            .is_ok_and(|has_all_proofs| has_all_proofs)
        {
            self.send_proof(batch_to_send).await?;
            self.rollup_storage
                .set_lastest_sent_batch_proof(batch_to_send)
                .await?;
        } else {
            info!("Missing proofs for batch {batch_to_send}, skipping sending");
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

        let verification_data = VerificationData {
            proving_system: ProvingSystemId::SP1,
            proof: proof.proof(),
            proof_generator_addr: self.l1_address.0.into(),
            vm_program_code: Some(ELF.to_vec()),
            verification_key: None,
            pub_input: None,
        };

        // TODO: remove unwrap
        let fee_estimation_default = estimate_fee(
            self.eth_client.urls.first().unwrap().as_str(),
            FeeEstimationType::Instant,
        )
        .await
        .unwrap();

        let nonce = get_nonce_from_ethereum(
            self.eth_client.urls.first().unwrap().as_str(), // TODO: remove unwrap
            self.l1_address.0.into(),
            Network::Devnet,
        )
        .await
        .unwrap();

        let wallet = Wallet::from_bytes(self.l1_private_key.as_ref())
            .map_err(|_| ProofSenderError::InternalError("Failed to create wallet".to_owned()))?;

        // TODO: remove hardcoded chain id
        let wallet = wallet.with_chain_id(31337u64);

        debug!("Sending proof to Aligned");

        submit(
            Network::Devnet, //TODO: remove hardcoded network
            &verification_data,
            fee_estimation_default,
            wallet,
            nonce,
        )
        .await
        .unwrap();

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
