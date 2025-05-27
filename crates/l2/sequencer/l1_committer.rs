use crate::{
    sequencer::{
        errors::CommitterError,
        state_diff::{get_nonce_diff, AccountStateDiff, DepositLog, StateDiff, WithdrawalLog},
    },
    utils::helpers::is_withdrawal_l2,
    CommitterConfig, EthConfig, SequencerConfig,
};

use ethrex_blockchain::vm::StoreVmDatabase;
#[cfg(feature = "metrics")]
use ethrex_common::types::BYTES_PER_BLOB;
use ethrex_common::{
    types::{
        blobs_bundle, fake_exponential_checked, AccountUpdate, BlobsBundle, BlobsBundleError,
        Block, BlockHeader, BlockNumber, PrivilegedL2Transaction, Receipt, Transaction, TxKind,
        BLOB_BASE_FEE_UPDATE_FRACTION, MIN_BASE_FEE_PER_BLOB_GAS,
    },
    Address, H256, U256,
};
use ethrex_l2_sdk::{
    calldata::{encode_calldata, Value},
    merkle_tree::merkelize,
};
use ethrex_metrics::metrics;
#[cfg(feature = "metrics")]
use ethrex_metrics::metrics_l2::{MetricsL2BlockType, METRICS_L2};
use ethrex_rpc::{
    clients::eth::{eth_sender::Overrides, BlockByNumber, EthClient, WrappedTransaction},
    utils::get_withdrawal_hash,
};
use ethrex_storage::Store;
use ethrex_storage_rollup::StoreRollup;
use ethrex_vm::{Evm, EvmEngine};
use keccak_hash::keccak;
use secp256k1::SecretKey;
use std::{collections::HashMap, sync::Arc};
use tracing::{debug, error, info, warn};

use super::{
    errors::{BlobEstimationError, SequencerError},
    execution_cache::ExecutionCache,
    utils::sleep_random,
};

const COMMIT_FUNCTION_SIGNATURE: &str = "commitBatch(uint256,bytes32,bytes32,bytes32,bytes32)";

pub struct Committer {
    eth_client: EthClient,
    on_chain_proposer_address: Address,
    store: Store,
    rollup_store: StoreRollup,
    l1_address: Address,
    l1_private_key: SecretKey,
    commit_time_ms: u64,
    arbitrary_base_blob_gas_price: u64,
    execution_cache: Arc<ExecutionCache>,
    validium: bool,
}

pub async fn start_l1_committer(
    store: Store,
    rollup_store: StoreRollup,
    execution_cache: Arc<ExecutionCache>,
    cfg: SequencerConfig,
) -> Result<(), SequencerError> {
    let mut committer = Committer::new_from_config(
        &cfg.l1_committer,
        &cfg.eth,
        store,
        rollup_store,
        execution_cache,
    )?;
    committer.run().await;
    Ok(())
}

impl Committer {
    pub fn new_from_config(
        committer_config: &CommitterConfig,
        eth_config: &EthConfig,
        store: Store,
        rollup_store: StoreRollup,
        execution_cache: Arc<ExecutionCache>,
    ) -> Result<Self, CommitterError> {
        Ok(Self {
            eth_client: EthClient::new_with_config(
                eth_config.rpc_url.iter().map(AsRef::as_ref).collect(),
                eth_config.max_number_of_retries,
                eth_config.backoff_factor,
                eth_config.min_retry_delay,
                eth_config.max_retry_delay,
                Some(eth_config.maximum_allowed_max_fee_per_gas),
                Some(eth_config.maximum_allowed_max_fee_per_blob_gas),
            )?,
            on_chain_proposer_address: committer_config.on_chain_proposer_address,
            store,
            rollup_store,
            l1_address: committer_config.l1_address,
            l1_private_key: committer_config.l1_private_key,
            commit_time_ms: committer_config.commit_time_ms,
            arbitrary_base_blob_gas_price: committer_config.arbitrary_base_blob_gas_price,
            execution_cache,
            validium: committer_config.validium,
        })
    }

    pub async fn run(&mut self) {
        loop {
            if let Err(err) = self.main_logic().await {
                error!("L1 Committer Error: {}", err);
            }

            sleep_random(self.commit_time_ms).await;
        }
    }

    async fn main_logic(&mut self) -> Result<(), CommitterError> {
        // Get the batch to commit
        let last_committed_batch_number = self
            .eth_client
            .get_last_committed_batch(self.on_chain_proposer_address)
            .await?;
        let batch_to_commit = last_committed_batch_number + 1;

        // Get the last committed block_number
        let last_committed_block_number = self
            .rollup_store
            .get_block_numbers_by_batch(last_committed_batch_number)
            .await?
            .and_then(|blocks| blocks.last().copied())
            .ok_or_else(|| CommitterError::InternalError("Invalid rollup_storage state".into()))?;

        let first_block_to_commit = last_committed_block_number + 1;

        // Try to prepare batch
        let (
            blobs_bundle,
            new_state_root,
            withdrawal_hashes,
            deposit_logs_hash,
            last_block_of_batch,
        ) = self
            .prepare_batch_from_block(last_committed_block_number)
            .await?;

        if last_committed_block_number == last_block_of_batch {
            debug!("No new blocks to commit, skipping");
            return Ok(());
        }

        let withdrawal_logs_merkle_root =
            self.get_withdrawals_merkle_root(withdrawal_hashes.clone())?;

        info!("Sending commitment for batch {batch_to_commit}. first_block: {first_block_to_commit}, last_block: {last_block_of_batch}");

        match self
            .send_commitment(
                batch_to_commit,
                new_state_root,
                withdrawal_logs_merkle_root,
                deposit_logs_hash,
                blobs_bundle,
            )
            .await
        {
            Ok(commit_tx_hash) => {
                metrics!(
                let _ = METRICS_L2
                    .set_block_type_and_block_number(
                        MetricsL2BlockType::LastCommittedBlock,
                        last_block_of_batch,
                    )
                    .inspect_err(|e| {
                        tracing::error!(
                            "Failed to set metric: last committed block {}",
                            e.to_string()
                        )
                    });
                );

                info!(
                    "Sent commitment for batch {batch_to_commit}, with tx hash {commit_tx_hash:#x}.",
                );
                self.rollup_store.store_batch(batch_to_commit, first_block_to_commit, last_block_of_batch, withdrawal_hashes).await?;
                Ok(())
            }
            Err(error) => Err(CommitterError::FailedToSendCommitment(format!(
                "Failed to send commitment for batch {batch_to_commit}. first_block: {first_block_to_commit} last_block: {last_block_of_batch}: {error}"
            ))),
        }
    }

    async fn prepare_batch_from_block(
        &self,
        mut last_added_block_number: BlockNumber,
    ) -> Result<(BlobsBundle, H256, Vec<H256>, H256, BlockNumber), CommitterError> {
        let first_block_of_batch = last_added_block_number + 1;
        let mut blobs_bundle = BlobsBundle::default();

        let mut acc_withdrawals = vec![];
        let mut acc_deposits = vec![];
        let mut acc_account_updates: HashMap<Address, AccountUpdate> = HashMap::new();
        let mut withdrawal_hashes = vec![];
        let mut deposit_logs_hashes = vec![];
        let mut new_state_root = H256::default();

        #[cfg(feature = "metrics")]
        let mut tx_count = 0_u64;
        let mut _blob_size = 0_usize;

        info!("Preparing state diff from block {first_block_of_batch}");

        loop {
            // Get a block to add to the batch
            let Some(block_to_commit_body) = self
                .store
                .get_block_body(last_added_block_number + 1)
                .await
                .map_err(CommitterError::from)?
            else {
                debug!("No new block to commit, skipping..");
                break;
            };
            let block_to_commit_header = self
                .store
                .get_block_header(last_added_block_number + 1)
                .map_err(CommitterError::from)?
                .ok_or(CommitterError::FailedToGetInformationFromStorage(
                    "Failed to get_block_header() after get_block_body()".to_owned(),
                ))?;

            // Get block transactions and receipts
            let mut txs_and_receipts = vec![];
            for (index, tx) in block_to_commit_body.transactions.iter().enumerate() {
                let receipt = self
                    .store
                    .get_receipt(last_added_block_number + 1, index.try_into()?)
                    .await?
                    .ok_or(CommitterError::InternalError(
                        "Transactions in a block should have a receipt".to_owned(),
                    ))?;
                txs_and_receipts.push((tx.clone(), receipt));
            }

            metrics!(
                tx_count += txs_and_receipts
                    .len()
                    .try_into()
                    .inspect_err(|_| tracing::error!("Failed to collect metric tx count"))
                    .unwrap_or(0)
            );
            // Get block withdrawals and deposits
            let withdrawals = self.get_block_withdrawals(&txs_and_receipts)?;
            let deposits = self.get_block_deposits(&txs_and_receipts);

            // Get block account updates.
            let block_to_commit = Block::new(block_to_commit_header.clone(), block_to_commit_body);
            let account_updates = match self.execution_cache.get(block_to_commit.hash())? {
                Some(account_updates) => account_updates,
                None => {
                    warn!(
                            "Could not find execution cache result for block {}, falling back to re-execution", last_added_block_number + 1
                        );

                    let vm_db = StoreVmDatabase::new(
                        self.store.clone(),
                        block_to_commit.header.parent_hash,
                    );
                    let mut vm = Evm::new(EvmEngine::default(), vm_db);
                    vm.execute_block(&block_to_commit)?;
                    vm.get_state_transitions()?
                }
            };

            // Accumulate block data with the rest of the batch.
            acc_withdrawals.extend(withdrawals.clone());
            acc_deposits.extend(deposits.clone());
            for account in account_updates {
                let address = account.address;
                if let Some(existing) = acc_account_updates.get_mut(&address) {
                    existing.merge(account);
                } else {
                    acc_account_updates.insert(address, account);
                }
            }

            let result = if !self.validium {
                // Prepare current state diff.
                let state_diff = self
                    .prepare_state_diff(
                        first_block_of_batch,
                        block_to_commit_header,
                        self.store.clone(),
                        &acc_withdrawals,
                        &acc_deposits,
                        acc_account_updates.clone().into_values().collect(),
                    )
                    .await?;
                self.generate_blobs_bundle(&state_diff)
            } else {
                Ok((BlobsBundle::default(), 0_usize))
            };

            match result {
                Ok((bundle, latest_blob_size)) => {
                    // Save current blobs_bundle and continue to add more blocks.
                    blobs_bundle = bundle;
                    _blob_size = latest_blob_size;
                    for (_, tx) in &withdrawals {
                        let hash = get_withdrawal_hash(tx)
                            .ok_or(CommitterError::InvalidWithdrawalTransaction)?;
                        withdrawal_hashes.push(hash);
                    }

                    deposit_logs_hashes.extend(
                        deposits
                            .iter()
                            .filter_map(|tx| tx.get_deposit_hash())
                            .collect::<Vec<H256>>(),
                    );

                    new_state_root = self
                        .store
                        .state_trie(block_to_commit.hash())?
                        .ok_or(CommitterError::FailedToGetInformationFromStorage(
                            "Failed to get state root from storage".to_owned(),
                        ))?
                        .hash_no_commit();

                    last_added_block_number += 1;
                }
                Err(_) => {
                    warn!("Batch size limit reached. Any remaining blocks will be processed in the next batch.");
                    // Break loop. Use the previous generated blobs_bundle.
                    break;
                }
            }
        }

        metrics!(if let (Ok(deposits_count), Ok(withdrawals_count)) = (
                deposit_logs_hashes.len().try_into(),
                withdrawal_hashes.len().try_into()
            ) {
                let _ = self
                    .rollup_store
                    .update_operations_count(tx_count, deposits_count, withdrawals_count)
                    .await
                    .inspect_err(|e| {
                        tracing::error!("Failed to update operations metric: {}", e.to_string())
                    });
            }
            #[allow(clippy::as_conversions)]
            METRICS_L2
                .set_blob_usage_percentage((_blob_size as f64 / BYTES_PER_BLOB as f64) * 100_f64);
        );

        let deposit_logs_hash = self.get_deposit_hash(deposit_logs_hashes)?;
        Ok((
            blobs_bundle,
            new_state_root,
            withdrawal_hashes,
            deposit_logs_hash,
            last_added_block_number,
        ))
    }

    fn get_block_withdrawals(
        &self,
        txs_and_receipts: &[(Transaction, Receipt)],
    ) -> Result<Vec<(H256, Transaction)>, CommitterError> {
        let mut ret = vec![];

        for (tx, receipt) in txs_and_receipts {
            if is_withdrawal_l2(tx, receipt)? {
                ret.push((tx.compute_hash(), tx.clone()))
            }
        }
        Ok(ret)
    }

    fn get_withdrawals_merkle_root(
        &self,
        withdrawals_hashes: Vec<H256>,
    ) -> Result<H256, CommitterError> {
        if !withdrawals_hashes.is_empty() {
            merkelize(withdrawals_hashes).map_err(CommitterError::FailedToMerkelize)
        } else {
            Ok(H256::zero())
        }
    }

    fn get_block_deposits(
        &self,
        txs_and_receipts: &[(Transaction, Receipt)],
    ) -> Vec<PrivilegedL2Transaction> {
        let deposits = txs_and_receipts
            .iter()
            .filter_map(|(tx, _)| match tx {
                Transaction::PrivilegedL2Transaction(tx) => Some(tx.clone()),
                _ => None,
            })
            .collect();

        deposits
    }

    fn get_deposit_hash(&self, deposit_hashes: Vec<H256>) -> Result<H256, CommitterError> {
        if !deposit_hashes.is_empty() {
            let deposit_hashes_len: u16 = deposit_hashes
                .len()
                .try_into()
                .map_err(CommitterError::from)?;
            Ok(H256::from_slice(
                [
                    &deposit_hashes_len.to_be_bytes(),
                    keccak(
                        deposit_hashes
                            .iter()
                            .map(H256::as_bytes)
                            .collect::<Vec<&[u8]>>()
                            .concat(),
                    )
                    .as_bytes()
                    .get(2..32)
                    .ok_or(CommitterError::FailedToDecodeDepositHash)?,
                ]
                .concat()
                .as_slice(),
            ))
        } else {
            Ok(H256::zero())
        }
    }

    /// Prepare the state diff for the block.
    async fn prepare_state_diff(
        &self,
        first_block_number: BlockNumber,
        last_header: BlockHeader,
        store: Store,
        withdrawals: &[(H256, Transaction)],
        deposits: &[PrivilegedL2Transaction],
        account_updates: Vec<AccountUpdate>,
    ) -> Result<StateDiff, CommitterError> {
        let mut modified_accounts = HashMap::new();
        for account_update in account_updates {
            // If we want the state_diff of a batch, we will have to change the -1 with the `batch_size`
            // and we may have to keep track of the latestCommittedBlock (last block of the batch),
            // the batch_size and the latestCommittedBatch in the contract.
            let nonce_diff = get_nonce_diff(&account_update, &store, None, first_block_number)
                .await
                .map_err(CommitterError::from)?;

            modified_accounts.insert(
                account_update.address,
                AccountStateDiff {
                    new_balance: account_update.info.clone().map(|info| info.balance),
                    nonce_diff,
                    storage: account_update.added_storage.clone().into_iter().collect(),
                    bytecode: account_update.code.clone(),
                    bytecode_hash: None,
                },
            );
        }

        let state_diff = StateDiff {
            modified_accounts,
            version: StateDiff::default().version,
            last_header,
            withdrawal_logs: withdrawals
                .iter()
                .map(|(hash, tx)| WithdrawalLog {
                    address: match tx.to() {
                        TxKind::Call(address) => address,
                        TxKind::Create => Address::zero(),
                    },
                    amount: tx.value(),
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
                    nonce: tx.nonce,
                })
                .collect(),
        };

        Ok(state_diff)
    }

    /// Generate the blob bundle necessary for the EIP-4844 transaction.
    fn generate_blobs_bundle(
        &self,
        state_diff: &StateDiff,
    ) -> Result<(BlobsBundle, usize), CommitterError> {
        let blob_data = state_diff.encode().map_err(CommitterError::from)?;

        let blob_size = blob_data.len();

        let blob = blobs_bundle::blob_from_bytes(blob_data).map_err(CommitterError::from)?;

        Ok((
            BlobsBundle::create_from_blobs(&vec![blob]).map_err(CommitterError::from)?,
            blob_size,
        ))
    }

    async fn send_commitment(
        &self,
        batch_number: u64,
        new_state_root: H256,
        withdrawal_logs_merkle_root: H256,
        deposit_logs_hash: H256,
        blobs_bundle: BlobsBundle,
    ) -> Result<H256, CommitterError> {
        let state_diff_kzg_versioned_hash = if !self.validium {
            let blob_versioned_hashes = blobs_bundle.generate_versioned_hashes();
            *blob_versioned_hashes
                .first()
                .ok_or(BlobsBundleError::BlobBundleEmptyError)
                .map_err(CommitterError::from)?
                .as_fixed_bytes()
        } else {
            [0u8; 32] // Validium doesn't send state_diff_kzg_versioned_hash.
        };

        let calldata_values = vec![
            Value::Uint(U256::from(batch_number)),
            Value::FixedBytes(new_state_root.0.to_vec().into()),
            Value::FixedBytes(state_diff_kzg_versioned_hash.to_vec().into()),
            Value::FixedBytes(withdrawal_logs_merkle_root.0.to_vec().into()),
            Value::FixedBytes(deposit_logs_hash.0.to_vec().into()),
        ];

        let calldata = encode_calldata(COMMIT_FUNCTION_SIGNATURE, &calldata_values)?;

        let gas_price = self
            .eth_client
            .get_gas_price_with_extra(20)
            .await?
            .try_into()
            .map_err(|_| {
                CommitterError::InternalError("Failed to convert gas_price to a u64".to_owned())
            })?;

        // Validium: EIP1559 Transaction.
        // Rollup: EIP4844 Transaction -> For on-chain Data Availability.
        let mut tx = if !self.validium {
            let le_bytes = estimate_blob_gas(
                &self.eth_client,
                self.arbitrary_base_blob_gas_price,
                20, // 20% of headroom
            )
            .await?
            .to_le_bytes();

            let gas_price_per_blob = U256::from_little_endian(&le_bytes);

            let wrapped_tx = self
                .eth_client
                .build_eip4844_transaction(
                    self.on_chain_proposer_address,
                    self.l1_address,
                    calldata.into(),
                    Overrides {
                        from: Some(self.l1_address),
                        gas_price_per_blob: Some(gas_price_per_blob),
                        max_fee_per_gas: Some(gas_price),
                        max_priority_fee_per_gas: Some(gas_price),
                        ..Default::default()
                    },
                    blobs_bundle,
                )
                .await
                .map_err(CommitterError::from)?;

            WrappedTransaction::EIP4844(wrapped_tx)
        } else {
            let wrapped_tx = self
                .eth_client
                .build_eip1559_transaction(
                    self.on_chain_proposer_address,
                    self.l1_address,
                    calldata.into(),
                    Overrides {
                        from: Some(self.l1_address),
                        max_fee_per_gas: Some(gas_price),
                        max_priority_fee_per_gas: Some(gas_price),
                        ..Default::default()
                    },
                )
                .await
                .map_err(CommitterError::from)?;

            WrappedTransaction::EIP1559(wrapped_tx)
        };

        self.eth_client
            .set_gas_for_wrapped_tx(&mut tx, self.l1_address)
            .await?;

        let commit_tx_hash = self
            .eth_client
            .send_tx_bump_gas_exponential_backoff(&mut tx, &self.l1_private_key)
            .await?;

        info!("Commitment sent: {commit_tx_hash:#x}");

        Ok(commit_tx_hash)
    }
}

/// Estimates the gas price for blob transactions based on the current state of the blockchain.
///
/// # Parameters:
/// - `eth_client`: The Ethereum client used to fetch the latest block.
/// - `arbitrary_base_blob_gas_price`: The base gas price that serves as the minimum price for blob transactions.
/// - `headroom`: Percentage applied to the estimated gas price to provide a buffer against fluctuations.
///
/// # Formula:
/// The gas price is estimated using an exponential function based on the blob gas used in the latest block and the
/// excess blob gas from the block header, following the formula from EIP-4844:
/// ```txt
///    blob_gas = arbitrary_base_blob_gas_price + (excess_blob_gas + blob_gas_used) * headroom
/// ```
async fn estimate_blob_gas(
    eth_client: &EthClient,
    arbitrary_base_blob_gas_price: u64,
    headroom: u64,
) -> Result<u64, CommitterError> {
    let latest_block = eth_client
        .get_block_by_number(BlockByNumber::Latest)
        .await?;

    let blob_gas_used = latest_block.header.blob_gas_used.unwrap_or(0);
    let excess_blob_gas = latest_block.header.excess_blob_gas.unwrap_or(0);

    // Using the formula from the EIP-4844
    // https://eips.ethereum.org/EIPS/eip-4844
    // def get_base_fee_per_blob_gas(header: Header) -> int:
    // return fake_exponential(
    //     MIN_BASE_FEE_PER_BLOB_GAS,
    //     header.excess_blob_gas,
    //     BLOB_BASE_FEE_UPDATE_FRACTION
    // )
    //
    // factor * e ** (numerator / denominator)
    // def fake_exponential(factor: int, numerator: int, denominator: int) -> int:

    // Check if adding the blob gas used and excess blob gas would overflow
    let total_blob_gas = match excess_blob_gas.checked_add(blob_gas_used) {
        Some(total) => total,
        None => return Err(BlobEstimationError::OverflowError.into()),
    };

    // If the blob's market is in high demand, the equation may give a really big number.
    // This function doesn't panic, it performs checked/saturating operations.
    let blob_gas = fake_exponential_checked(
        MIN_BASE_FEE_PER_BLOB_GAS,
        total_blob_gas,
        BLOB_BASE_FEE_UPDATE_FRACTION,
    )
    .map_err(BlobEstimationError::FakeExponentialError)?;

    let gas_with_headroom = (blob_gas * (100 + headroom)) / 100;

    // Check if we have an overflow when we take the headroom into account.
    let blob_gas = match arbitrary_base_blob_gas_price.checked_add(gas_with_headroom) {
        Some(gas) => gas,
        None => return Err(BlobEstimationError::OverflowError.into()),
    };

    Ok(blob_gas)
}
