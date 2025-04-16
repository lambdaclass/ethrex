use std::time::Duration;

use ethrex_common::{Address, H160, H256, U256};
use ethrex_l2_sdk::calldata::{encode_calldata, Value};
use ethrex_rpc::{
    clients::{eth::WrappedTransaction, Overrides},
    EthClient,
};
use secp256k1::SecretKey;
use tokio::sync::oneshot::Receiver;
use tracing::{debug, error, info};

use crate::utils::{
    config::{
        committer::CommitterConfig, errors::ConfigError, eth::EthConfig,
        prover_server::ProverServerConfig,
    },
    prover::{
        proving_systems::ProverType,
        save_state::{block_number_has_all_needed_proofs, read_proof, StateFileType},
    },
};

use super::errors::ProverServerError;

// These constants have to match with the OnChainProposer.sol contract
const R0VERIFIER: &str = "R0VERIFIER()";
const SP1VERIFIER: &str = "SP1VERIFIER()";
const PICOVERIFIER: &str = "PICOVERIFIER()";
pub const VERIFIER_CONTRACTS: [&str; 3] = [R0VERIFIER, SP1VERIFIER, PICOVERIFIER];
pub const DEV_MODE_ADDRESS: H160 = H160([
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0xAA,
]);
pub const VERIFY_FUNCTION_SIGNATURE: &str =
    "verify(uint256,bytes,bytes32,bytes32,bytes32,bytes,bytes,bytes32,bytes,uint256[8])";

pub async fn start_proof_sender() -> Result<(), ConfigError> {
    let eth_config = EthConfig::from_env()?;
    let committer_config = CommitterConfig::from_env()?;
    let prover_server_config = ProverServerConfig::from_env()?;

    let proof_sender =
        ProofSender::new(&prover_server_config, &committer_config, &eth_config).await?;
    proof_sender.run().await;

    Ok(())
}

struct ProofSender {
    eth_client: EthClient,
    l1_address: Address,
    l1_private_key: SecretKey,
    on_chain_proposer_address: Address,
    needed_proof_types: Vec<ProverType>,
}

impl ProofSender {
    async fn new(
        config: &ProverServerConfig,
        committer_config: &CommitterConfig,
        eth_config: &EthConfig,
    ) -> Result<Self, ConfigError> {
        let eth_client = EthClient::new(&eth_config.rpc_url);

        let verifier_contracts = EthClient::get_verifier_contracts(
            &eth_client,
            &VERIFIER_CONTRACTS,
            committer_config.on_chain_proposer_address,
        )
        .await?;

        let mut needed_proof_types = vec![];
        if !config.dev_mode {
            for (key, addr) in verifier_contracts {
                if addr == DEV_MODE_ADDRESS {
                    continue;
                } else {
                    match key.as_str() {
                        "R0VERIFIER()" => {
                            info!("RISC0 proof needed");
                            needed_proof_types.push(ProverType::RISC0);
                        }
                        "SP1VERIFIER()" => {
                            info!("SP1 proof needed");
                            needed_proof_types.push(ProverType::SP1);
                        }
                        "PICOVERIFIER()" => {
                            info!("PICO proof needed");
                            needed_proof_types.push(ProverType::Pico);
                        }
                        _ => unreachable!("There shouldn't be a value different than the used backends/verifiers R0VERIFIER|SP1VERIFER|PICOVERIFIER."),
                    }
                }
            }
        } else {
            needed_proof_types.push(ProverType::Exec);
        }

        Ok(Self {
            eth_client,
            l1_address: config.l1_address,
            l1_private_key: config.l1_private_key,
            on_chain_proposer_address: committer_config.on_chain_proposer_address,
            needed_proof_types,
        })
    }

    async fn run(&self) {
        let (shutdown_tx, mut shutdown_rx) = tokio::sync::oneshot::channel();
        tokio::task::spawn(async move {
            if let Err(e) = tokio::signal::ctrl_c().await {
                error!("Error handling ctrl_c: {e}");
            };
            if let Err(e) = shutdown_tx.send(0) {
                error!("Error sending shutdown message through the oneshot::channel {e}");
            };
        });

        loop {
            match self.main_logic(&mut shutdown_rx).await {
                Ok(()) => {
                    info!("Proof sender finished. Shutting down");
                    break;
                }
                Err(e) => {
                    error!("Proof sender exited with error, trying to restart the main_logic function: {e}")
                }
            }

            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    }

    async fn main_logic(&self, shutdown_rx: &mut Receiver<i32>) -> Result<(), ProverServerError> {
        loop {
            if shutdown_rx.try_recv().is_ok() {
                debug!("Received shutdown signal");
                break;
            }

            let block_to_verify = 1 + EthClient::get_last_verified_block(
                &self.eth_client,
                self.on_chain_proposer_address,
            )
            .await?;

            if block_number_has_all_needed_proofs(block_to_verify, &self.needed_proof_types)
                .is_ok_and(|has_all_proofs| has_all_proofs)
            {
                self.send_proof(block_to_verify).await?;
            }
        }

        Ok(())
    }

    pub async fn send_proof(&self, block_number: u64) -> Result<H256, ProverServerError> {
        // TODO: change error
        // TODO: If the proof is not needed, a default calldata is used,
        // the structure has to match the one defined in the OnChainProposer.sol contract.
        // It may cause some issues, but the ethrex_prover_lib cannot be imported,
        // this approach is straight-forward for now.
        let risc0_proof = {
            if self.needed_proof_types.contains(&ProverType::RISC0) {
                let risc0_proof =
                    read_proof(block_number, StateFileType::Proof(ProverType::RISC0))?;
                if risc0_proof.prover_type != ProverType::RISC0 {
                    return Err(ProverServerError::Custom(
                        "RISC0 Proof isn't present".to_string(),
                    ));
                }
                risc0_proof.calldata
            } else {
                ProverType::RISC0.empty_calldata()
            }
        };

        let sp1_proof = {
            if self.needed_proof_types.contains(&ProverType::SP1) {
                let sp1_proof = read_proof(block_number, StateFileType::Proof(ProverType::SP1))?;
                if sp1_proof.prover_type != ProverType::SP1 {
                    return Err(ProverServerError::Custom(
                        "SP1 Proof isn't present".to_string(),
                    ));
                }
                sp1_proof.calldata
            } else {
                ProverType::SP1.empty_calldata()
            }
        };

        let pico_proof = {
            if self.needed_proof_types.contains(&ProverType::Pico) {
                let pico_proof = read_proof(block_number, StateFileType::Proof(ProverType::Pico))?;
                if pico_proof.prover_type != ProverType::Pico {
                    return Err(ProverServerError::Custom(
                        "Pico Proof isn't present".to_string(),
                    ));
                }
                pico_proof.calldata
            } else {
                ProverType::Pico.empty_calldata()
            }
        };

        debug!("Sending proof for block number: {block_number}");

        let calldata_values = [
            &[Value::Uint(U256::from(block_number))],
            risc0_proof.as_slice(),
            sp1_proof.as_slice(),
            pico_proof.as_slice(),
        ]
        .concat();

        let calldata = encode_calldata(VERIFY_FUNCTION_SIGNATURE, &calldata_values)?;

        let gas_price = self
            .eth_client
            .get_gas_price_with_extra(20)
            .await?
            .try_into()
            .map_err(|_| {
                ProverServerError::InternalError("Failed to convert gas_price to a u64".to_owned())
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

        info!("Sent proof for block {block_number}, with transaction hash {verify_tx_hash:#x}");

        Ok(verify_tx_hash)
    }
}
