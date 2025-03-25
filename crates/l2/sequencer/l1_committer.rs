use crate::{
    sequencer::{
        errors::CommitterError,
        state_diff::{get_nonce_diff, AccountStateDiff, DepositLog, StateDiff, WithdrawalLog},
    },
    utils::{
        config::{committer::CommitterConfig, errors::ConfigError, eth::EthConfig},
        helpers::is_withdrawal_l2,
    },
};

use ethrex_common::{
    types::{
        blobs_bundle, fake_exponential_checked, BlobsBundle, BlobsBundleError, Block,
        PrivilegedL2Transaction, Receipt, Transaction, TxKind, BLOB_BASE_FEE_UPDATE_FRACTION,
        MIN_BASE_FEE_PER_BLOB_GAS,
    },
    Address, H256, U256,
};
use ethrex_l2_sdk::calldata::{encode_calldata, Value};
use ethrex_l2_sdk::{get_withdrawal_hash, merkle_tree::merkelize};
use ethrex_rpc::clients::eth::{
    eth_sender::Overrides, BlockByNumber, EthClient, WrappedTransaction,
};
use ethrex_storage::{AccountUpdate, Store};
use ethrex_vm::backends::Evm;
use keccak_hash::keccak;
use secp256k1::SecretKey;
use std::{collections::HashMap, sync::Arc, time::Duration};
use tokio::time::sleep;
use tracing::{debug, error, info, warn};

use super::{errors::BlobEstimationError, execution_cache::ExecutionCache};

const COMMIT_FUNCTION_SIGNATURE: &str = "commit(uint256,bytes32,bytes32,bytes32)";

pub struct Committer {
    eth_client: EthClient,
    on_chain_proposer_address: Address,
    store: Store,
    l1_address: Address,
    l1_private_key: SecretKey,
    interval_ms: u64,
    arbitrary_base_blob_gas_price: u64,
    execution_cache: Arc<ExecutionCache>,
}

pub async fn start_l1_committer(
    store: Store,
    execution_cache: Arc<ExecutionCache>,
) -> Result<(), ConfigError> {
    let eth_config = EthConfig::from_env()?;
    let committer_config = CommitterConfig::from_env()?;

    let mut committer =
        Committer::new_from_config(&committer_config, eth_config, store, execution_cache);
    committer.run().await;
    Ok(())
}

impl Committer {
    pub fn new_from_config(
        committer_config: &CommitterConfig,
        eth_config: EthConfig,
        store: Store,
        execution_cache: Arc<ExecutionCache>,
    ) -> Self {
        Self {
            eth_client: EthClient::new(&eth_config.rpc_url),
            on_chain_proposer_address: committer_config.on_chain_proposer_address,
            store,
            l1_address: committer_config.l1_address,
            l1_private_key: committer_config.l1_private_key,
            interval_ms: committer_config.interval_ms,
            arbitrary_base_blob_gas_price: committer_config.arbitrary_base_blob_gas_price,
            execution_cache,
        }
    }

    pub async fn run(&mut self) {
        loop {
            if let Err(err) = self.main_logic().await {
                error!("L1 Committer Error: {}", err);
            }

            sleep(Duration::from_millis(self.interval_ms)).await;
        }
    }

    async fn main_logic(&mut self) -> Result<(), CommitterError> {
        let block_number = 1 + EthClient::get_last_committed_block(
            &self.eth_client,
            self.on_chain_proposer_address,
        )
        .await?;

        let Some(block_to_commit_body) = self
            .store
            .get_block_body(block_number)
            .map_err(CommitterError::from)?
        else {
            debug!("No new block to commit, skipping..");
            return Ok(());
        };

        let block_to_commit_header = self
            .store
            .get_block_header(block_number)
            .map_err(CommitterError::from)?
            .ok_or(CommitterError::FailedToGetInformationFromStorage(
                "Failed to get_block_header() after get_block_body()".to_owned(),
            ))?;

        let mut txs_and_receipts = vec![];
        for (index, tx) in block_to_commit_body.transactions.iter().enumerate() {
            let receipt = self
                .store
                .get_receipt(block_number, index.try_into()?)?
                .ok_or(CommitterError::InternalError(
                    "Transactions in a block should have a receipt".to_owned(),
                ))?;
            txs_and_receipts.push((tx.clone(), receipt));
        }

        let block_to_commit = Block::new(block_to_commit_header, block_to_commit_body);

        let withdrawals = self.get_block_withdrawals(&txs_and_receipts)?;
        let deposits = self.get_block_deposits(&block_to_commit);

        let mut withdrawal_hashes = vec![];

        for (_, tx) in &withdrawals {
            let hash =
                get_withdrawal_hash(tx).ok_or(CommitterError::InvalidWithdrawalTransaction)?;
            withdrawal_hashes.push(hash);
        }

        let withdrawal_logs_merkle_root = self.get_withdrawals_merkle_root(withdrawal_hashes)?;
        let deposit_logs_hash = self.get_deposit_hash(
            deposits
                .iter()
                .filter_map(|tx| tx.get_deposit_hash())
                .collect(),
        )?;

        let account_updates = match self.execution_cache.get(block_to_commit.hash())? {
            Some(account_updates) => account_updates,
            None => {
                warn!(
                            "Could not find execution cache result for block {block_number}, falling back to re-execution"
                        );
                Evm::default(self.store.clone(), block_to_commit.header.parent_hash)
                    .execute_block(&block_to_commit)
                    .map(|result| result.account_updates)?
            }
        };

        let state_diff = self.prepare_state_diff(
            &block_to_commit,
            self.store.clone(),
            withdrawals,
            deposits,
            &account_updates,
        )?;

        let blobs_bundle = self.generate_blobs_bundle(&state_diff)?;

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
                info!("Sent commitment to block {head_block_hash:#x}, with transaction hash {commit_tx_hash:#x}");
                Ok(())
            }
            Err(error) => Err(CommitterError::FailedToSendCommitment(format!(
                "Failed to send commitment to block {head_block_hash:#x}: {error}"
            ))),
        }
    }

    fn get_block_withdrawals(
        &self,
        txs_and_receipts: &[(Transaction, Receipt)],
    ) -> Result<Vec<(H256, Transaction)>, CommitterError> {
        let mut ret = vec![];

        for (tx, receipt) in txs_and_receipts {
            if is_withdrawal_l2(tx, receipt) {
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

    fn get_block_deposits(&self, block: &Block) -> Vec<PrivilegedL2Transaction> {
        let deposits = block
            .body
            .transactions
            .iter()
            .filter_map(|tx| match tx {
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
    fn prepare_state_diff(
        &self,
        block: &Block,
        store: Store,
        withdrawals: Vec<(H256, Transaction)>,
        deposits: Vec<PrivilegedL2Transaction>,
        account_updates: &[AccountUpdate],
    ) -> Result<StateDiff, CommitterError> {
        info!("Preparing state diff for block {}", block.header.number);

        let mut modified_accounts = HashMap::new();
        for account_update in account_updates {
            // If we want the state_diff of a batch, we will have to change the -1 with the `batch_size`
            // and we may have to keep track of the latestCommittedBlock (last block of the batch),
            // the batch_size and the latestCommittedBatch in the contract.
            let nonce_diff = get_nonce_diff(account_update, &store, block.header.number)
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
            header: block.header.clone(),
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
    fn generate_blobs_bundle(&self, state_diff: &StateDiff) -> Result<BlobsBundle, CommitterError> {
        let blob_data = state_diff.encode().map_err(CommitterError::from)?;

        let blob = blobs_bundle::blob_from_bytes(blob_data).map_err(CommitterError::from)?;

        BlobsBundle::create_from_blobs(&vec![blob]).map_err(CommitterError::from)
    }

    async fn send_commitment(
        &self,
        block_number: u64,
        withdrawal_logs_merkle_root: H256,
        deposit_logs_hash: H256,
        blobs_bundle: BlobsBundle,
    ) -> Result<H256, CommitterError> {
        info!("Sending commitment for block {block_number}");

        let blob_versioned_hashes = blobs_bundle.generate_versioned_hashes();
        let calldata_values = vec![
            Value::Uint(U256::from(block_number)),
            Value::FixedBytes(
                blob_versioned_hashes
                    .first()
                    .ok_or(BlobsBundleError::BlobBundleEmptyError)
                    .map_err(CommitterError::from)?
                    .as_fixed_bytes()
                    .to_vec()
                    .into(),
            ),
            Value::FixedBytes(withdrawal_logs_merkle_root.0.to_vec().into()),
            Value::FixedBytes(deposit_logs_hash.0.to_vec().into()),
        ];

        let calldata = encode_calldata(COMMIT_FUNCTION_SIGNATURE, &calldata_values)?;

        let le_bytes = estimate_blob_gas(
            &self.eth_client,
            self.arbitrary_base_blob_gas_price,
            20, // 20% of headroom
        )
        .await?
        .to_le_bytes();

        let gas_price_per_blob = Some(U256::from_little_endian(&le_bytes));

        let wrapped_tx = self
            .eth_client
            .build_eip4844_transaction(
                self.on_chain_proposer_address,
                self.l1_address,
                calldata.into(),
                Overrides {
                    from: Some(self.l1_address),
                    gas_price_per_blob,
                    ..Default::default()
                },
                blobs_bundle,
                10,
            )
            .await
            .map_err(CommitterError::from)?;

        let commit_tx_hash = self
            .eth_client
            .send_wrapped_transaction_with_retry(
                &WrappedTransaction::EIP4844(wrapped_tx),
                &self.l1_private_key,
                3 * 60, // 3 minutes
                10,     // 180[secs]/20[retries] -> 18 seconds per retry
            )
            .await
            .map_err(CommitterError::from)?;

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
