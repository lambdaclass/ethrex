use std::{collections::HashMap, str::FromStr};

use ethrex_common::{Address, H160, H256, U256};
use ethrex_l2_sdk::calldata::{encode_calldata, Value};
use ethrex_rpc::{
    clients::{eth::WrappedTransaction, Overrides},
    EthClient,
};
use keccak_hash::keccak;
use secp256k1::SecretKey;
use tracing::{debug, error, info};

use crate::{
    sequencer::errors::ProofSenderError,
    utils::prover::{
        proving_systems::ProverType,
        save_state::{batch_number_has_all_needed_proofs, read_proof, StateFileType},
    },
    CommitterConfig, EthConfig, ProofCoordinatorConfig, SequencerConfig,
};

use super::{errors::SequencerError, utils::sleep_random};

const DEV_MODE_ADDRESS: H160 = H160([
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0xAA,
]);
const VERIFY_FUNCTION_SIGNATURE: &str =
    "verifyBatch(uint256,bytes,bytes32,bytes,bytes,bytes,bytes32,bytes,uint256[8],bytes,bytes)";

pub async fn start_l1_proof_sender(cfg: SequencerConfig) -> Result<(), SequencerError> {
    let proof_sender =
        L1ProofSender::new(&cfg.proof_coordinator, &cfg.l1_committer, &cfg.eth).await?;
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
}

impl L1ProofSender {
    async fn new(
        cfg: &ProofCoordinatorConfig,
        committer_cfg: &CommitterConfig,
        eth_cfg: &EthConfig,
    ) -> Result<Self, ProofSenderError> {
        let eth_client = EthClient::new_with_multiple_urls(eth_cfg.rpc_url.clone())?;

        let mut needed_proof_types = vec![];
        if !cfg.dev_mode {
            for prover_type in ProverType::all() {
                let Some(getter) = prover_type.verifier_getter() else {
                    continue;
                };
                let calldata = keccak(getter)[..4].to_vec();

                let response = eth_client
                    .call(
                        committer_cfg.on_chain_proposer_address,
                        calldata.into(),
                        Overrides::default(),
                    )
                    .await?;
                // trim to 20 bytes, also removes 0x prefix
                let trimmed_response = &response[26..];

                let address =
                    Address::from_str(&format!("0x{trimmed_response}")).map_err(|_| {
                        ProofSenderError::FailedToParseOnChainProposerResponse(response)
                    })?;

                if address != DEV_MODE_ADDRESS {
                    info!("{prover_type} proof needed");
                    needed_proof_types.push(prover_type);
                }
            }
        } else {
            needed_proof_types.push(ProverType::Exec);
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

    async fn run(&self) {
        loop {
            info!("Running L1 Proof Sender");
            info!("Needed proof systems: {:?}", self.needed_proof_types);
            if let Err(err) = self.main_logic().await {
                error!("L1 Proof Sender Error: {}", err);
            }

            sleep_random(self.proof_send_interval_ms).await;
        }
    }

    async fn main_logic(&self) -> Result<(), ProofSenderError> {
        let batch_to_verify = 1 + self
            .eth_client
            .get_last_verified_batch(self.on_chain_proposer_address)
            .await?;

        if batch_number_has_all_needed_proofs(batch_to_verify, &self.needed_proof_types)
            .is_ok_and(|has_all_proofs| has_all_proofs)
        {
            self.send_proof(batch_to_verify).await?;
        } else {
            info!("Missing proofs for batch {batch_to_verify}, skipping sending");
        }

        Ok(())
    }

    pub async fn send_proof(&self, batch_number: u64) -> Result<H256, ProofSenderError> {
        // TODO: change error
        // TODO: If the proof is not needed, a default calldata is used,
        // the structure has to match the one defined in the OnChainProposer.sol contract.
        // It may cause some issues, but the ethrex_prover_lib cannot be imported,
        // this approach is straight-forward for now.
        let mut proofs = HashMap::with_capacity(self.needed_proof_types.len());
        for prover_type in self.needed_proof_types.iter() {
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

        let gas_price = self
            .eth_client
            .get_gas_price_with_extra(20)
            .await?
            .try_into()
            .map_err(|_| {
                ProofSenderError::InternalError("Failed to convert gas_price to a u64".to_owned())
            })?;

        let verify_tx = self
            .eth_client
            .build_eip1559_transaction(
                self.on_chain_proposer_address,
                self.l1_address,
                calldata.into(),
                Overrides {
                    max_fee_per_gas: Some(gas_price),
                    max_priority_fee_per_gas: Some(gas_price),
                    ..Default::default()
                },
            )
            .await?;

        let mut tx = WrappedTransaction::EIP1559(verify_tx);

        let verify_tx_hash = self
            .eth_client
            .send_tx_bump_gas_exponential_backoff(&mut tx, &self.l1_private_key)
            .await?;

        info!("Sent proof for batch {batch_number}, with transaction hash {verify_tx_hash:#x}");

        Ok(verify_tx_hash)
    }
}
