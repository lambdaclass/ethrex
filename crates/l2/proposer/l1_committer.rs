use crate::{
    proposer::{
        errors::CommitterError,
        state_diff::{AccountStateDiff, DepositLog, StateDiff, WithdrawalLog},
    },
    utils::{
        config::{committer::CommitterConfig, eth::EthConfig},
        eth_client::{errors::EthClientError, eth_sender::Overrides, EthClient},
        merkle_tree::merkelize,
    },
};
use bytes::Bytes;
use ethrex_blockchain::constants::TX_GAS_COST;
use ethrex_core::{
    types::{
        blobs_bundle, BlobsBundle, Block, EIP1559Transaction, GenericTransaction,
        PrivilegedL2Transaction, PrivilegedTxType, Transaction, TxKind,
    },
    Address, H256, U256,
};
use ethrex_rpc::types::transaction::WrappedEIP4844Transaction;
use ethrex_storage::Store;
use ethrex_vm::{evm_state, execute_block, get_state_transitions};
use keccak_hash::keccak;
use secp256k1::SecretKey;
use sha2::{Digest, Sha256};
use std::ops::Div;
use std::{collections::HashMap, time::Duration};
use tokio::time::sleep;
use tracing::{error, info, warn};

const COMMIT_FUNCTION_SELECTOR: [u8; 4] = [132, 97, 12, 179];

pub struct Committer {
    eth_client: EthClient,
    on_chain_proposer_address: Address,
    store: Store,
    l1_address: Address,
    l1_private_key: SecretKey,
    interval_ms: u64,
}

pub async fn start_l1_commiter(store: Store) {
    let eth_config = EthConfig::from_env().expect("EthConfig::from_env()");
    let committer_config = CommitterConfig::from_env().expect("CommitterConfig::from_env");
    let committer = Committer::new_from_config(&committer_config, eth_config, store);
    committer.start().await.expect("committer.start()");
}

impl Committer {
    pub fn new_from_config(
        committer_config: &CommitterConfig,
        eth_config: EthConfig,
        store: Store,
    ) -> Self {
        Self {
            eth_client: EthClient::new(&eth_config.rpc_url),
            on_chain_proposer_address: committer_config.on_chain_proposer_address,
            store,
            l1_address: committer_config.l1_address,
            l1_private_key: committer_config.l1_private_key,
            interval_ms: committer_config.interval_ms,
        }
    }

    pub async fn start(&self) -> Result<(), CommitterError> {
        loop {
            let last_committed_block = EthClient::get_last_committed_block(
                &self.eth_client,
                self.on_chain_proposer_address,
            )
            .await?;

            let block_number_to_fetch = if last_committed_block == u64::MAX {
                0
            } else {
                last_committed_block + 1
            };

            if let Some(block_to_commit_body) = self
                .store
                .get_block_body(block_number_to_fetch)
                .map_err(CommitterError::from)?
            {
                let block_to_commit_header = self
                    .store
                    .get_block_header(block_number_to_fetch)
                    .map_err(CommitterError::from)?
                    .ok_or(CommitterError::FailedToGetInformationFromStorage(
                        "Failed to get_block_header() after get_block_body()".to_owned(),
                    ))?;

                let block_to_commit = Block::new(block_to_commit_header, block_to_commit_body);

                let withdrawals = self.get_block_withdrawals(&block_to_commit)?;
                let deposits = self.get_block_deposits(&block_to_commit)?;

                let withdrawal_logs_merkle_root = self.get_withdrawals_merkle_root(
                    withdrawals
                        .iter()
                        .map(|(_hash, tx)| {
                            tx.get_withdrawal_hash()
                                .expect("Not a withdrawal transaction")
                        })
                        .collect(),
                );
                let deposit_logs_hash = self.get_deposit_hash(
                    deposits
                        .iter()
                        .filter_map(|tx| tx.get_deposit_hash())
                        .collect(),
                );

                let state_diff = self.prepare_state_diff(
                    &block_to_commit,
                    self.store.clone(),
                    withdrawals,
                    deposits,
                )?;

                let blobs_bundle = self.generate_blobs_bundle(state_diff.clone())?;

                let head_block_hash = block_to_commit.hash();
                match self
                    .send_commitment(
                        block_to_commit.header.number,
                        withdrawal_logs_merkle_root,
                        deposit_logs_hash,
                        blobs_bundle,
                    )
                    .await
                {
                    Ok(commit_tx_hash) => {
                        info!(
                    "Sent commitment to block {head_block_hash:#x}, with transaction hash {commit_tx_hash:#x}"
                );
                    }
                    Err(error) => {
                        error!("Failed to send commitment to block {head_block_hash:#x}. Manual intervention required: {error}");
                        panic!("Failed to send commitment to block {head_block_hash:#x}. Manual intervention required: {error}");
                    }
                }
            }

            sleep(Duration::from_millis(self.interval_ms)).await;
        }
    }

    pub fn get_block_withdrawals(
        &self,
        block: &Block,
    ) -> Result<Vec<(H256, PrivilegedL2Transaction)>, CommitterError> {
        let withdrawals = block
            .body
            .transactions
            .iter()
            .filter_map(|tx| match tx {
                Transaction::PrivilegedL2Transaction(priv_tx)
                    if priv_tx.tx_type == PrivilegedTxType::Withdrawal =>
                {
                    Some((tx.compute_hash(), priv_tx.clone()))
                }
                _ => None,
            })
            .collect();

        Ok(withdrawals)
    }

    pub fn get_withdrawals_merkle_root(&self, withdrawals_hashes: Vec<H256>) -> H256 {
        if !withdrawals_hashes.is_empty() {
            merkelize(withdrawals_hashes)
        } else {
            H256::zero()
        }
    }

    pub fn get_block_deposits(
        &self,
        block: &Block,
    ) -> Result<Vec<PrivilegedL2Transaction>, CommitterError> {
        let deposits = block
            .body
            .transactions
            .iter()
            .filter_map(|tx| match tx {
                Transaction::PrivilegedL2Transaction(tx)
                    if tx.tx_type == PrivilegedTxType::Deposit =>
                {
                    Some(tx.clone())
                }
                _ => None,
            })
            .collect();

        Ok(deposits)
    }

    pub fn get_deposit_hash(&self, deposit_hashes: Vec<H256>) -> H256 {
        if !deposit_hashes.is_empty() {
            H256::from_slice(
                [
                    &(deposit_hashes.len() as u16).to_be_bytes(),
                    &keccak(
                        deposit_hashes
                            .iter()
                            .map(H256::as_bytes)
                            .collect::<Vec<&[u8]>>()
                            .concat(),
                    )
                    .as_bytes()[2..32],
                ]
                .concat()
                .as_slice(),
            )
        } else {
            H256::zero()
        }
    }
    /// Prepare the state diff for the block.
    pub fn prepare_state_diff(
        &self,
        block: &Block,
        store: Store,
        withdrawals: Vec<(H256, PrivilegedL2Transaction)>,
        deposits: Vec<PrivilegedL2Transaction>,
    ) -> Result<StateDiff, CommitterError> {
        info!("Preparing state diff for block {}", block.header.number);

        let mut state = evm_state(store.clone(), block.header.parent_hash);
        execute_block(block, &mut state).map_err(CommitterError::from)?;
        let account_updates = get_state_transitions(&mut state);

        let mut modified_accounts = HashMap::new();
        account_updates.iter().for_each(|account_update| {
            modified_accounts.insert(
                account_update.address,
                AccountStateDiff {
                    new_balance: account_update.info.clone().map(|info| info.balance),
                    nonce_diff: account_update.info.clone().map(|info| info.nonce as u16),
                    storage: account_update.added_storage.clone().into_iter().collect(),
                    bytecode: account_update.code.clone(),
                    bytecode_hash: None,
                },
            );
        });

        let state_diff = StateDiff {
            modified_accounts,
            version: StateDiff::default().version,
            withdrawal_logs: withdrawals
                .iter()
                .map(|(hash, tx)| WithdrawalLog {
                    address: match tx.to {
                        TxKind::Call(address) => address,
                        TxKind::Create => Address::zero(),
                    },
                    amount: tx.value,
                    tx_hash: *hash,
                })
                .collect(),
            deposit_logs: deposits
                .iter()
                .map(|tx| DepositLog {
                    address: match tx.to {
                        TxKind::Call(address) => address,
                        TxKind::Create => Address::zero(),
                    },
                    amount: tx.value,
                })
                .collect(),
        };

        Ok(state_diff)
    }

    /// Generate the blob bundle necessary for the EIP-4844 transaction.
    pub fn generate_blobs_bundle(
        &self,
        state_diff: StateDiff,
    ) -> Result<BlobsBundle, CommitterError> {
        let blob_data = state_diff.encode().map_err(CommitterError::from)?;

        let blob = blobs_bundle::blob_from_bytes(blob_data).map_err(CommitterError::from)?;

        BlobsBundle::create_from_blobs(&vec![blob]).map_err(CommitterError::from)
    }

    pub async fn send_commitment(
        &self,
        block_number: u64,
        withdrawal_logs_merkle_root: H256,
        deposit_logs_hash: H256,
        blobs_bundle: BlobsBundle,
    ) -> Result<H256, CommitterError> {
        info!("Sending commitment for block {block_number}");

        // TODO: This could be done using BlobsBundle.generate_versioned_hashes but we use different hashing crates
        // in l1 and l2, we might need to consider unification later, for now the implementation here still works
        let mut hasher = Sha256::new();
        hasher.update(
            blobs_bundle
                .commitments
                .first()
                .expect("At least one commitment should be present"),
        );
        let mut blob_versioned_hash = hasher.finalize();
        blob_versioned_hash[0] = 0x01; // EIP-4844 versioning

        let mut calldata = Vec::with_capacity(132);
        calldata.extend(COMMIT_FUNCTION_SELECTOR);
        let mut block_number_bytes = [0_u8; 32];
        U256::from(block_number).to_big_endian(&mut block_number_bytes);
        calldata.extend(block_number_bytes);
        calldata.extend(blob_versioned_hash);
        calldata.extend(withdrawal_logs_merkle_root.0);
        calldata.extend(deposit_logs_hash.0);

        let wrapped_tx = self
            .eth_client
            .build_eip4844_transaction(
                self.on_chain_proposer_address,
                Bytes::from(calldata),
                Overrides {
                    from: Some(self.l1_address),
                    gas_price_per_blob: Some(U256::from_dec_str("100000000000").unwrap()),
                    ..Default::default()
                },
                blobs_bundle,
            )
            .await
            .map_err(CommitterError::from)?;

        let commit_tx_hash = self
            .eth_client
            .send_eip4844_transaction(wrapped_tx.clone(), &self.l1_private_key)
            .await
            .map_err(CommitterError::from)?;

        let commit_tx_hash = wrapped_eip4844_transaction_handler(
            &self.eth_client,
            &wrapped_tx,
            &self.l1_private_key,
            commit_tx_hash,
            10,
        )
        .await?;

        info!("Commitment sent: {commit_tx_hash:#x}");

        Ok(commit_tx_hash)
    }
}

pub async fn send_transaction_with_calldata(
    eth_client: &EthClient,
    l1_address: Address,
    l1_private_key: SecretKey,
    to: Address,
    nonce: Option<u64>,
    calldata: Bytes,
) -> Result<H256, EthClientError> {
    let mut tx = EIP1559Transaction {
        to: TxKind::Call(to),
        data: calldata,
        max_fee_per_gas: eth_client.get_gas_price().await?.as_u64() * 2,
        nonce: nonce.unwrap_or(eth_client.get_nonce(l1_address).await?),
        chain_id: eth_client.get_chain_id().await?.as_u64(),
        // Should the max_priority_fee_per_gas be dynamic?
        max_priority_fee_per_gas: 10u64,
        ..Default::default()
    };

    let mut generic_tx = GenericTransaction::from(tx.clone());
    generic_tx.from = l1_address;

    tx.gas_limit = eth_client
        .estimate_gas(generic_tx)
        .await?
        .saturating_add(TX_GAS_COST);

    eth_client
        .send_eip1559_transaction(tx, &l1_private_key)
        .await
}

async fn wrapped_eip4844_transaction_handler(
    eth_client: &EthClient,
    wrapped_eip4844: &WrappedEIP4844Transaction,
    l1_private_key: &SecretKey,
    commit_tx_hash: H256,
    max_retries: u32,
) -> Result<H256, CommitterError> {
    let mut retries = 0;
    let max_receipt_retries: u32 = 60 * 2; // 2 minutes
    let mut commit_tx_hash = commit_tx_hash;
    let mut wrapped_tx = wrapped_eip4844.clone();

    while retries < max_retries {
        if (eth_client.get_transaction_receipt(commit_tx_hash).await?).is_some() {
            // If the tx_receipt was found, return the tx_hash.
            return Ok(commit_tx_hash);
        } else {
            // Else, wait for receipt and send again if necessary.
            let mut receipt_retries = 0;

            // Try for 2 minutes with an interval of 1 second to get the tx_receipt.
            while receipt_retries < max_receipt_retries {
                match eth_client.get_transaction_receipt(commit_tx_hash).await? {
                    Some(_) => return Ok(commit_tx_hash),
                    None => {
                        receipt_retries += 1;
                        sleep(Duration::from_secs(1)).await;
                    }
                }
            }

            // If receipt was not found, send the same tx(same nonce) but with more gas.
            // Sometimes the penalty is a 100%
            warn!("Transaction not confirmed, resending with 110% more gas...");
            // Increase max fee per gas by 110% (set it to 210% of the original)
            wrapped_tx.tx.max_fee_per_gas =
                (wrapped_tx.tx.max_fee_per_gas as f64 * 2.1).round() as u64;
            wrapped_tx.tx.max_priority_fee_per_gas =
                (wrapped_tx.tx.max_priority_fee_per_gas as f64 * 2.1).round() as u64;
            wrapped_tx.tx.max_fee_per_blob_gas = wrapped_tx
                .tx
                .max_fee_per_blob_gas
                .saturating_mul(U256::from(20))
                .div(10);

            commit_tx_hash = eth_client
                .send_eip4844_transaction(wrapped_tx.clone(), l1_private_key)
                .await
                .map_err(CommitterError::from)?;

            retries += 1;
        }
    }
    Err(CommitterError::FailedToSendCommitment(
        "Error handling eip4844".to_owned(),
    ))
}
